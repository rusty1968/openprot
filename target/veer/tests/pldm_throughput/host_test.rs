// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host Update-Agent driver for PLDM over MCTP over I3C — Phase B.
//!
//! Drives a full one-component firmware update against the FD firmware over
//! the I3C socket: `RequestUpdate` -> `PassComponentTable` -> `UpdateComponent`
//! (UA->FD commands), then services the FD-initiated download loop
//! (`RequestFirmwareData` chunks, then `TransferComplete`/`VerifyComplete`/
//! `ApplyComplete` acks) until the FD applies and exits 0.
//!
//! Two directions over the wire:
//! * UA->FD command: host private-write the request, private-read the response.
//! * FD->UA request: host private-read the FD's staged request, private-write
//!   the response.
//!
//! Framing mirrors `tests/mctp_i3c`; PLDM messages use `pldm_common` codecs.

use i3c_host::{
    connect_i3c_socket, crc8_smbus, read_outgoing_packet, send_private_read_on_stream,
    send_private_write_raw_on_stream, Runner,
};
use pldm_common::codec::{PldmCodec, PldmCodecWithLifetime};
use pldm_common::message::firmware_update::apply_complete::ApplyCompleteResponse;
use pldm_common::message::firmware_update::pass_component::PassComponentTableRequest;
use pldm_common::message::firmware_update::request_fw_data::{
    RequestFirmwareDataRequest, RequestFirmwareDataResponse,
};
use pldm_common::message::firmware_update::request_update::RequestUpdateRequest;
use pldm_common::message::firmware_update::transfer_complete::TransferCompleteResponse;
use pldm_common::message::firmware_update::update_component::UpdateComponentRequest;
use pldm_common::message::firmware_update::verify_complete::VerifyCompleteResponse;
use pldm_common::protocol::base::{
    PldmBaseCompletionCode, PldmMsgHeader, PldmMsgType, TransferRespFlag,
};
use pldm_common::protocol::firmware_update::{
    ComponentClassification, FwUpdateCmd, PldmFirmwareString, UpdateOptionFlags, VersionStringType,
    PLDM_FWUP_IMAGE_SET_VER_STR_MAX_LEN,
};
use std::thread;
use std::time::{Duration, Instant};

const I3C_TARGET_ADDR: u8 = 0x08;
const I3C_CTRL_ADDR: u8 = 0x0a;
const FD_EID: u8 = 8;
const UA_EID: u8 = 0x0a;
const PLDM_MSG_TYPE: u8 = 0x01;

/// Component image the UA serves; must match the FD's IMAGE_SIZE (4096).
const IMAGE_SIZE: u32 = 4096;
const COMP_ID: u16 = 0x0001;

const SUCCESS: u8 = PldmBaseCompletionCode::Success as u8;

fn fw_string(s: &str) -> PldmFirmwareString {
    let bytes = s.as_bytes();
    let mut str_data = [0u8; PLDM_FWUP_IMAGE_SET_VER_STR_MAX_LEN];
    str_data[..bytes.len()].copy_from_slice(bytes);
    PldmFirmwareString {
        str_type: VersionStringType::Ascii as u8,
        str_len: bytes.len() as u8,
        str_data,
    }
}

fn image() -> Vec<u8> {
    (0..IMAGE_SIZE).map(|i| (i as u8).wrapping_mul(7)).collect()
}

/// Wrap PLDM bytes in an MCTP-over-I3C frame (UA -> FD) with the given MCTP
/// flags byte, appending PEC. `flags` bit3 is TO (tag owner); bits2:0 are the tag.
fn mctp_frame(pldm: &[u8], flags: u8) -> Vec<u8> {
    let mut mctp = vec![0x01, FD_EID, UA_EID, flags, PLDM_MSG_TYPE];
    mctp.extend_from_slice(pldm);
    let byte_count = (1 + mctp.len()) as u8;
    let mut frame = vec![
        I3C_TARGET_ADDR << 1,
        0x0f,
        byte_count,
        (I3C_CTRL_ADDR << 1) | 1,
    ];
    frame.extend_from_slice(&mctp);
    frame.push(crc8_smbus(&frame));
    frame
}

/// A UA-originated PLDM *request* (RequestUpdate, PassComponentTable, ...):
/// SOM|EOM, TO=1 (the UA owns the tag), tag 0.
fn ua_to_fd_frame(pldm: &[u8]) -> Vec<u8> {
    mctp_frame(pldm, 0xC8)
}

/// A UA *response* to an FD-initiated request (RequestFirmwareData / *Complete):
/// SOM|EOM, TO=0, echoing the tag the FD owns for that transaction. Sending
/// these with TO=1 makes the FD's stack route them to its responder (awaiting UA
/// commands) instead of its requester (awaiting this reply), stalling download.
fn fd_response_frame(pldm: &[u8], tag: u8) -> Vec<u8> {
    mctp_frame(pldm, 0xC0 | (tag & 0x07))
}

/// The MCTP flags byte of an FD frame (index 7: [i3c 4][ver dest src flags]).
fn mctp_tag_from_frame(data: &[u8]) -> Option<u8> {
    data.get(7).map(|flags| flags & 0x07)
}

/// Extract the PLDM payload from an FD frame. Layout:
/// [i3c hdr 4][mctp hdr 4][msg_type 1][pldm...][PEC].
fn pldm_from_frame(data: &[u8]) -> Option<&[u8]> {
    const PREFIX: usize = 4 + 4 + 1;
    if data.len() < PREFIX + 1 {
        return None;
    }
    Some(&data[PREFIX..data.len() - 1])
}

/// True if the PLDM message is a request (Rq bit set in byte 0).
fn is_request(pldm: &[u8]) -> bool {
    pldm.first().is_some_and(|b| b & 0x80 != 0)
}

/// Send a UA->FD command and poll for its response. Returns the completion
/// code once a matching *response* (Rq clear, same command) arrives.
fn command(stream: &mut std::net::TcpStream, runner: &Runner, pldm: &[u8], cmd: u8) -> Option<u8> {
    // Send the request once, then *poll* (read-only) for the response. The FD
    // (run_terminus) processes a command once and moves on to await the next, so
    // re-sending the same command every poll re-triggers it faster than it can
    // stage a reply we then read — livelocking the round-trip. Re-send only
    // occasionally, to recover a first write that raced the socket accept.
    const RESEND_EVERY: u32 = 10;
    let frame = ua_to_fd_frame(pldm);
    for i in 0..200u32 {
        if runner.exited() {
            return None;
        }
        if i % RESEND_EVERY == 0 {
            let _ = send_private_write_raw_on_stream(stream, I3C_TARGET_ADDR, &frame);
        }
        thread::sleep(Duration::from_millis(50));
        if send_private_read_on_stream(stream, I3C_TARGET_ADDR).is_ok()
            && let Ok(pkt) = read_outgoing_packet(stream)
            && let Some(resp) = pldm_from_frame(&pkt.data)
            && resp.len() >= 4
            && !is_request(resp)
            && resp[2] == cmd
        {
            return Some(resp[3]);
        }
    }
    None
}

/// Read one FD-initiated request (Rq set). Retries while the FD stages it.
/// Returns the PLDM payload and the MCTP tag the FD owns, so the reply can echo
/// it (with TO=0) and be routed back to the FD's requester.
fn read_fd_request(stream: &mut std::net::TcpStream, runner: &Runner) -> Option<(Vec<u8>, u8)> {
    for _ in 0..100 {
        if runner.exited() {
            return None;
        }
        if send_private_read_on_stream(stream, I3C_TARGET_ADDR).is_ok()
            && let Ok(pkt) = read_outgoing_packet(stream)
            && let Some(tag) = mctp_tag_from_frame(&pkt.data)
            && let Some(pldm) = pldm_from_frame(&pkt.data)
            && is_request(pldm)
        {
            return Some((pldm.to_vec(), tag));
        }
        thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn pldm_throughput_host_test() {
    let runner = Runner::spawn(
        "target/veer/tests/pldm_throughput/pldm_throughput_runner.sh",
        "pldm fd: waiting for update agent",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before firmware readiness"
    );

    let mut stream =
        connect_i3c_socket(Duration::from_secs(5)).expect("failed to connect to I3C socket");
    // A private-read returns nothing until the FD has staged a frame. Without a
    // read timeout, read_outgoing_packet's read_exact blocks forever on the
    // first empty read, so command()/read_fd_request could never re-send. The
    // timeout turns "nothing staged yet" into a fast error, letting the poll
    // loops re-send the request until the FD answers.
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("set read timeout on I3C socket");
    let comp_ver = fw_string("v1.0");
    let img = image();
    let mut buf = [0u8; 256];
    let mut iid = 0u8;

    // ---- RequestUpdate ----
    let n = RequestUpdateRequest::new(iid, PldmMsgType::Request, IMAGE_SIZE, 1, 1, 0, &comp_ver)
        .encode(&mut buf)
        .expect("encode RequestUpdate");
    let cc = command(
        &mut stream,
        &runner,
        &buf[..n],
        FwUpdateCmd::RequestUpdate as u8,
    );
    assert_eq!(cc, Some(SUCCESS), "RequestUpdate should succeed");

    // ---- PassComponentTable ----
    iid += 1;
    let n = PassComponentTableRequest::new(
        iid,
        PldmMsgType::Request,
        TransferRespFlag::StartAndEnd,
        ComponentClassification::Firmware,
        COMP_ID,
        0,
        0,
        &comp_ver,
    )
    .encode(&mut buf)
    .expect("encode PassComponentTable");
    let cc = command(
        &mut stream,
        &runner,
        &buf[..n],
        FwUpdateCmd::PassComponentTable as u8,
    );
    assert_eq!(cc, Some(SUCCESS), "PassComponentTable should succeed");

    // ---- UpdateComponent: FD enters Download and starts RequestFirmwareData ----
    iid += 1;
    let n = UpdateComponentRequest::new(
        iid,
        PldmMsgType::Request,
        ComponentClassification::Firmware,
        COMP_ID,
        0,
        0,
        IMAGE_SIZE,
        UpdateOptionFlags(0),
        &comp_ver,
    )
    .encode(&mut buf)
    .expect("encode UpdateComponent");
    let cc = command(
        &mut stream,
        &runner,
        &buf[..n],
        FwUpdateCmd::UpdateComponent as u8,
    );
    assert_eq!(cc, Some(SUCCESS), "UpdateComponent should succeed");

    // ---- FD-initiated download/verify/apply loop ----
    let mut downloaded = 0usize;
    let mut applied = false;
    // UA-side download timing, matching caliptra-mcu-sw's PLDM UA metric
    // (commit 78e0c7b6): start the clock at the first firmware-data chunk and
    // report bytes/s over the download phase.
    let mut xfer_start: Option<Instant> = None;
    for _ in 0..200 {
        let Some((req, tag)) = read_fd_request(&mut stream, &runner) else {
            break;
        };
        let Some(hdr) = PldmMsgHeader::<[u8; 3]>::decode(&req).ok() else {
            continue;
        };
        let fd_iid = hdr.instance_id();
        let mut resp = [0u8; 256];
        let resp_len = match FwUpdateCmd::try_from(hdr.cmd_code()) {
            Ok(FwUpdateCmd::RequestFirmwareData) => {
                let fw =
                    RequestFirmwareDataRequest::decode(&req).expect("decode RequestFirmwareData");
                let off = fw.offset as usize;
                let end = (off + fw.length as usize).min(img.len());
                downloaded = downloaded.max(end);
                if xfer_start.is_none() {
                    xfer_start = Some(Instant::now());
                }
                let data = &img[off..end];
                PldmCodecWithLifetime::encode(
                    &RequestFirmwareDataResponse::new(fd_iid, SUCCESS, data),
                    &mut resp,
                )
                .expect("encode RequestFirmwareData response")
            }
            Ok(FwUpdateCmd::TransferComplete) => {
                if let Some(t) = xfer_start {
                    let secs = t.elapsed().as_secs_f64().max(1e-9);
                    let bps = downloaded as f64 / secs;
                    println!(
                        "PLDM_THROUGHPUT: {downloaded} bytes in {secs:.3} s ({bps:.1} B/s, {:.1} KB/s)",
                        bps / 1024.0
                    );
                }
                TransferCompleteResponse::new(fd_iid, SUCCESS)
                    .encode(&mut resp)
                    .expect("encode TransferComplete response")
            }
            Ok(FwUpdateCmd::VerifyComplete) => VerifyCompleteResponse::new(fd_iid, SUCCESS)
                .encode(&mut resp)
                .expect("encode VerifyComplete response"),
            Ok(FwUpdateCmd::ApplyComplete) => {
                applied = true;
                ApplyCompleteResponse::new(fd_iid, SUCCESS)
                    .encode(&mut resp)
                    .expect("encode ApplyComplete response")
            }
            _ => continue,
        };
        let _ = send_private_write_raw_on_stream(
            &mut stream,
            I3C_TARGET_ADDR,
            &fd_response_frame(&resp[..resp_len], tag),
        );
        if applied {
            break;
        }
    }

    let status = runner.wait();
    assert!(
        applied,
        "FD never reached ApplyComplete (downloaded {downloaded} bytes)"
    );
    assert_eq!(
        downloaded, IMAGE_SIZE as usize,
        "the whole image should have been requested"
    );
    assert!(status.success(), "runner exited with status: {status}");
}
