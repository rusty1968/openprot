// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! A PLDM Update Agent (UA) for the host side of the VeeR PLDM tests.
//!
//! Drives one firmware update of one component against the PLDM firmware
//! device in the firmware, over MCTP over I3C through `caliptra_mcu_hw_model`:
//!
//! 1. UA to FD commands, each a request the FD answers: `RequestUpdate`,
//!    `PassComponentTable`, `UpdateComponent`.
//! 2. The FD-initiated loop: the FD requests the image in chunks
//!    (`RequestFirmwareData`) and the UA answers each with the data, then the FD
//!    reports `TransferComplete`, `VerifyComplete` and `ApplyComplete`.
//! 3. `ActivateFirmware`. The FD then has nothing left to do, waits out its
//!    idle timeout and exits PASS.
//!
//! Every message goes through `veer_host_util`: a request carries the tag-owner
//! bit and the UA's tag; a UA reply to an FD request has the bit clear and echoes
//! the FD's tag (a reply with the bit set would be routed to the FD's responder,
//! which awaits UA commands, instead of its requester, which awaits this reply,
//! and the download would stall).
//!
//! The download phase is timed on the host's wall clock, from the first
//! `RequestFirmwareData` the FD sends to its `TransferComplete`. That is how
//! caliptra-mcu-sw measures its own PLDM transfer speed, so the number is
//! dominated by the per-chunk round trip, not the firmware's raw speed.

use std::time::{Duration, Instant};

use caliptra_mcu_hw_model::McuHwModel;
use caliptra_mcu_testing_common::i3c_socket::BufferedStream;
use pldm_common::codec::{PldmCodec, PldmCodecWithLifetime};
use pldm_common::message::firmware_update::activate_fw::{
    ActivateFirmwareRequest, SelfContainedActivationRequest,
};
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
use veer_host_util::{read_message, send_message, Message};

/// The firmware device's MCTP EID (`FD_EID` in the firmware).
pub const FD_EID: u8 = 8;
/// This UA's MCTP EID (`UA_EID` in the firmware).
pub const UA_EID: u8 = 0x0a;
/// The MCTP message type of PLDM, the first byte of every message body.
pub const PLDM_MSG_TYPE: u8 = 0x01;
/// The component the UA updates.
pub const COMP_ID: u16 = 0x0001;

const SUCCESS: u8 = PldmBaseCompletionCode::Success as u8;

/// What to update with.
#[derive(Debug, Clone, Copy)]
pub struct UpdateConfig {
    /// Size of the component image in bytes.
    pub image_size: u32,
    /// The largest chunk the UA offers in `RequestUpdate`. The FD asks for the
    /// smaller of this and its own cap.
    pub max_transfer_size: u32,
}

/// What happened.
#[derive(Debug, Clone)]
pub struct Report {
    /// Highest image offset the FD has asked for, plus its length.
    pub downloaded: usize,
    /// How many `RequestFirmwareData` the FD sent.
    pub chunks: u32,
    /// Host wall clock from the first `RequestFirmwareData` to `TransferComplete`.
    pub transfer: Option<Duration>,
    /// Whether the FD reported `ApplyComplete`.
    pub applied: bool,
}

impl Report {
    /// Download throughput in bytes per second, if the transfer was timed.
    pub fn bytes_per_second(&self) -> Option<f64> {
        let secs = self.transfer?.as_secs_f64();
        (secs > 0.0).then(|| self.downloaded as f64 / secs)
    }
}

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

/// The image the UA serves: a pattern that is not constant, so a wrong offset
/// shows up.
pub fn image(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i as u8).wrapping_mul(7)).collect()
}

/// A message body for PLDM: the MCTP message type byte, then the PLDM message.
fn pldm_body(pldm: &[u8]) -> Vec<u8> {
    let mut body = vec![PLDM_MSG_TYPE];
    body.extend_from_slice(pldm);
    body
}

/// Send a UA to FD command and read the FD's response. Returns the response's
/// completion code. Checks the response is for this command and tag.
fn command(
    hw: &mut impl McuHwModel,
    stream: &mut BufferedStream,
    log: &mut String,
    target_addr: u8,
    pldm: &[u8],
    cmd: FwUpdateCmd,
    name: &str,
) -> u8 {
    let tag = 0;
    let request = Message {
        dest: FD_EID,
        src: UA_EID,
        tag_owner: true,
        tag,
        body: pldm_body(pldm),
        packets: 0,
    };
    send_message(hw, stream, log, target_addr, &request);

    let reply = read_message(hw, stream, log, target_addr);
    assert_eq!(reply.dest, UA_EID, "{name} response destination");
    assert_eq!(reply.src, FD_EID, "{name} response source");
    assert!(!reply.tag_owner, "{name}: the FD answered with a request");
    assert_eq!(reply.tag, tag, "{name} response tag");
    assert_eq!(reply.body[0] & 0x7f, PLDM_MSG_TYPE, "{name} message type");
    let pldm = &reply.body[1..];
    // [Rq|instance][type][command][completion code]
    assert!(pldm.len() >= 4, "{name} response too short: {pldm:02x?}");
    assert_eq!(
        pldm[0] & 0x80,
        0,
        "{name}: the FD sent a request, not a response"
    );
    assert_eq!(pldm[2], cmd as u8, "response to a different command");
    pldm[3]
}

/// Run one update: the three commands, the download loop, and activation.
pub fn run_update(
    hw: &mut impl McuHwModel,
    stream: &mut BufferedStream,
    log: &mut String,
    target_addr: u8,
    config: UpdateConfig,
) -> Report {
    let img = image(config.image_size as usize);
    let comp_ver = fw_string("v1.0");
    let mut buf = [0u8; 256];
    let mut iid = 0u8;

    // ---- RequestUpdate ----
    let n = RequestUpdateRequest::new(
        iid,
        PldmMsgType::Request,
        config.max_transfer_size,
        1,
        1,
        0,
        &comp_ver,
    )
    .encode(&mut buf)
    .expect("encode RequestUpdate");
    let cc = command(
        hw,
        stream,
        log,
        target_addr,
        &buf[..n],
        FwUpdateCmd::RequestUpdate,
        "RequestUpdate",
    );
    assert_eq!(cc, SUCCESS, "RequestUpdate should succeed");

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
        hw,
        stream,
        log,
        target_addr,
        &buf[..n],
        FwUpdateCmd::PassComponentTable,
        "PassComponentTable",
    );
    assert_eq!(cc, SUCCESS, "PassComponentTable should succeed");

    // ---- UpdateComponent: the FD enters Download and starts requesting data ----
    iid += 1;
    let n = UpdateComponentRequest::new(
        iid,
        PldmMsgType::Request,
        ComponentClassification::Firmware,
        COMP_ID,
        0,
        0,
        config.image_size,
        UpdateOptionFlags(0),
        &comp_ver,
    )
    .encode(&mut buf)
    .expect("encode UpdateComponent");
    let cc = command(
        hw,
        stream,
        log,
        target_addr,
        &buf[..n],
        FwUpdateCmd::UpdateComponent,
        "UpdateComponent",
    );
    assert_eq!(cc, SUCCESS, "UpdateComponent should succeed");

    // ---- the FD-initiated download, verify and apply loop ----
    let mut report = Report {
        downloaded: 0,
        chunks: 0,
        transfer: None,
        applied: false,
    };
    let mut xfer_start: Option<Instant> = None;
    // Room for the largest chunk the FD can ask for plus the response header.
    let mut resp = vec![0u8; 1100];
    while !report.applied {
        let request = read_message(hw, stream, log, target_addr);
        assert_eq!(request.dest, UA_EID, "FD request destination");
        assert_eq!(request.src, FD_EID, "FD request source");
        assert!(request.tag_owner, "the FD sent a response, not a request");
        assert_eq!(request.body[0] & 0x7f, PLDM_MSG_TYPE, "message type");
        let req = &request.body[1..];
        let hdr = PldmMsgHeader::<[u8; 3]>::decode(req).expect("PLDM header");
        let fd_iid = hdr.instance_id();

        let resp_len = match FwUpdateCmd::try_from(hdr.cmd_code()) {
            Ok(FwUpdateCmd::RequestFirmwareData) => {
                xfer_start.get_or_insert_with(Instant::now);
                let fw = RequestFirmwareDataRequest::decode(req).expect("RequestFirmwareData");
                let off = fw.offset as usize;
                let end = (off + fw.length as usize).min(img.len());
                report.downloaded = report.downloaded.max(end);
                report.chunks += 1;
                PldmCodecWithLifetime::encode(
                    &RequestFirmwareDataResponse::new(fd_iid, SUCCESS, &img[off..end]),
                    &mut resp,
                )
                .expect("encode RequestFirmwareData response")
            }
            Ok(FwUpdateCmd::TransferComplete) => {
                report.transfer = xfer_start.map(|t| t.elapsed());
                TransferCompleteResponse::new(fd_iid, SUCCESS)
                    .encode(&mut resp)
                    .expect("encode TransferComplete response")
            }
            Ok(FwUpdateCmd::VerifyComplete) => VerifyCompleteResponse::new(fd_iid, SUCCESS)
                .encode(&mut resp)
                .expect("encode VerifyComplete response"),
            Ok(FwUpdateCmd::ApplyComplete) => {
                report.applied = true;
                ApplyCompleteResponse::new(fd_iid, SUCCESS)
                    .encode(&mut resp)
                    .expect("encode ApplyComplete response")
            }
            _ => panic!(
                "unexpected request from the FD: command {:#04x}, {req:02x?}",
                hdr.cmd_code()
            ),
        };
        let reply = Message {
            dest: FD_EID,
            src: UA_EID,
            tag_owner: false,
            tag: request.tag,
            body: pldm_body(&resp[..resp_len]),
            packets: 0,
        };
        send_message(hw, stream, log, target_addr, &reply);
    }

    // ---- ActivateFirmware: the last command; the FD then idles out and exits ----
    iid += 1;
    let n = ActivateFirmwareRequest::new(
        iid,
        PldmMsgType::Request,
        SelfContainedActivationRequest::NotActivateSelfContainedComponents,
    )
    .encode(&mut buf)
    .expect("encode ActivateFirmware");
    let cc = command(
        hw,
        stream,
        log,
        target_addr,
        &buf[..n],
        FwUpdateCmd::ActivateFirmware,
        "ActivateFirmware",
    );
    assert_eq!(cc, SUCCESS, "ActivateFirmware should succeed");

    report
}
