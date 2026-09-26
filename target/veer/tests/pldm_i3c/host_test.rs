// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host Update-Agent driver for PLDM over MCTP over I3C — Phase A.
//!
//! Proves the plumbing before the full firmware-update flow: sends a single
//! PLDM `RequestUpdate` to the FD firmware over the I3C socket and checks the
//! FD accepts it (enters update mode) and replies. This exercises the two
//! FD MCTP transports over one mctp channel, the EID wiring, and the
//! PLDM-over-MCTP-over-I3C framing. The FD exits 0 once the update is
//! requested (see `pldm_fd.rs`). Phase B/C add the download/verify/apply loop.
//!
//! Framing mirrors `tests/mctp_i3c` (MCTP-over-I3C), with MCTP message type
//! 0x01 (PLDM) and PLDM bytes built via `pldm_common` codecs.

use i3c_host::{
    connect_i3c_socket, crc8_smbus, read_outgoing_packet, send_private_read_on_stream,
    send_private_write_raw_on_stream, Runner,
};
use pldm_common::codec::PldmCodec;
use pldm_common::message::firmware_update::request_update::RequestUpdateRequest;
use pldm_common::protocol::base::{PldmBaseCompletionCode, PldmMsgType};
use pldm_common::protocol::firmware_update::{
    FwUpdateCompletionCode, PldmFirmwareString, VersionStringType,
    PLDM_FWUP_IMAGE_SET_VER_STR_MAX_LEN,
};
use std::thread;
use std::time::Duration;

const I3C_TARGET_ADDR: u8 = 0x08;
const I3C_CTRL_ADDR: u8 = 0x0a;
const FD_EID: u8 = 8; // the target's MCTP EID (mctp_server OWN_EID)
const UA_EID: u8 = 0x0a; // the FD serves this remote EID
const PLDM_MSG_TYPE: u8 = 0x01;
const MAX_XFER: u32 = 180;
/// PLDM firmware-update RequestUpdate command code (DSP0267).
const REQUEST_UPDATE_CMD: u8 = 0x10;

/// Build a fixed-size PLDM firmware version string.
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

/// Wrap PLDM bytes in an MCTP-over-I3C frame (UA -> FD), with PEC.
fn ua_to_fd_frame(pldm: &[u8]) -> Vec<u8> {
    // MCTP packet: version, dest EID, src EID, flags (SOM|EOM|TO|tag0), msg type.
    let mut mctp = vec![0x01, FD_EID, UA_EID, 0xC8, PLDM_MSG_TYPE];
    mctp.extend_from_slice(pldm);
    // I3C block-write frame.
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

/// Extract the PLDM payload from an FD reply frame.
/// Layout: [i3c hdr 4][mctp hdr 4][msg_type 1][pldm...][PEC].
fn pldm_from_reply(data: &[u8]) -> Option<&[u8]> {
    const PREFIX: usize = 4 + 4 + 1;
    if data.len() < PREFIX + 1 {
        return None;
    }
    Some(&data[PREFIX..data.len() - 1])
}

#[test]
fn pldm_i3c_request_update_host_test() {
    let runner = Runner::spawn(
        "target/veer/tests/pldm_i3c/pldm_i3c_runner.sh",
        "pldm fd: waiting for update agent",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before firmware readiness"
    );

    let mut stream =
        connect_i3c_socket(Duration::from_secs(5)).expect("failed to connect to I3C socket");

    // Build one RequestUpdate for a single component.
    let comp_ver = fw_string("v1.0");
    let req = RequestUpdateRequest::new(
        0, // instance_id
        PldmMsgType::Request,
        MAX_XFER, // max_transfer_size
        1,        // num_of_comp
        1,        // max_outstanding_transfer_req
        0,        // pkg_data_len
        &comp_ver,
    );
    let mut pldm = [0u8; 128];
    let n = req.encode(&mut pldm).expect("encode RequestUpdate");
    let frame = ua_to_fd_frame(&pldm[..n]);

    // Retry until the FD answers (the readiness log races the IRQ arming). A
    // retry after the first landed gets AlreadyInUpdateMode, which equally
    // proves the FD processed a RequestUpdate.
    let success = PldmBaseCompletionCode::Success as u8;
    let already = FwUpdateCompletionCode::AlreadyInUpdateMode as u8;
    let mut accepted = false;
    let mut attempts = 0u32;
    while !accepted && !runner.exited() && attempts < 100 {
        attempts += 1;
        let _ = send_private_write_raw_on_stream(&mut stream, I3C_TARGET_ADDR, &frame);
        thread::sleep(Duration::from_millis(100));
        // Accept only a genuine RequestUpdate *response*: a PLDM header
        // (byte0 Rq bit clear) for the RequestUpdate command with a plausible
        // completion code. A too-loose check (just byte3 == 0) false-accepts a
        // stale/garbled read and stops retrying, which is what masked a lost
        // first frame.
        if send_private_read_on_stream(&mut stream, I3C_TARGET_ADDR).is_ok()
            && let Ok(packet) = read_outgoing_packet(&mut stream)
            && let Some(resp) = pldm_from_reply(&packet.data)
            && resp.len() >= 4
            && (resp[0] & 0x80) == 0
            && resp[2] == REQUEST_UPDATE_CMD
            && (resp[3] == success || resp[3] == already)
        {
            accepted = true;
        }
    }

    let status = runner.wait();
    assert!(accepted, "FD did not accept RequestUpdate after {attempts} attempts");
    assert!(
        status.success(),
        "runner exited with status: {status} (accepted={accepted})"
    );
}
