// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! PLDM requester that sends FD-initiated PLDM messages over MCTP.
//!
//! During the download, verify, and apply phases of a PLDM firmware update
//! the Firmware Device (FD) acts as an *initiator*: it sends requests such
//! as `RequestFirmwareData`, `TransferComplete`, `VerifyComplete`, and
//! `ApplyComplete` to the Update Agent (UA) and waits for the corresponding
//! responses.  [`PldmRequester`] wraps a [`CmdInterface`] and handles the
//! MCTP transport for this initiator role, complementing the responder-side
//! [`PldmResponder`] which handles inbound UA requests.
//!
//! ## Buffer layout
//!
//! The buffer passed to [`PldmRequester::run_once`] uses the same layout as
//! [`PldmResponder`]:
//!
//! ```text
//! buf[0]          : MCTP message-type (0x01) – written by run_once
//! buf[1..]        : PLDM message (header + data)
//! ```
//!
//! [`PldmResponder`]: crate::responder::PldmResponder

use openprot_mctp_api::MctpClient;
use pldm_interface::cmd_interface::CmdInterface;
use pldm_interface::control_context::ProtocolCapability;

use crate::error::PldmServiceError;
use crate::transport::MctpPldmTransport;

/// PLDM requester service (FD initiator mode).
///
/// Wraps a [`CmdInterface`] and sends FD-initiated PLDM requests over an
/// MCTP transport provided by a [`Stack`].  Use this alongside a
/// [`PldmResponder`] when the firmware device needs to actively exchange
/// messages with an Update Agent during the download, verify, and apply
/// phases of a firmware update.
///
/// # Example
///
/// ```rust,ignore
/// use openprot_pldm_service::requester::PldmRequester;
/// use pldm_interface::control_context::ProtocolCapability;
/// use pldm_common::protocol::base::{PldmControlCmd, PldmSupportedType};
///
/// const CTRL_CMDS: [u8; 5] = [
///     PldmControlCmd::SetTid as u8,
///     PldmControlCmd::GetTid as u8,
///     PldmControlCmd::GetPldmCommands as u8,
///     PldmControlCmd::GetPldmVersion as u8,
///     PldmControlCmd::GetPldmTypes as u8,
/// ];
/// static CAPS: [ProtocolCapability<'static>; 1] = [
///     ProtocolCapability::new(PldmSupportedType::Base, "1.1.0", &CTRL_CMDS).unwrap(),
/// ];
///
/// let mut requester = PldmRequester::new(&CAPS);
/// let mut buf = [0u8; 1024];
/// loop {
///     if let Err(e) = requester.run_once(&stack, REMOTE_EID, &mut buf, 0) {
///         // handle error
///     }
/// }
/// ```
///
/// [`PldmResponder`]: crate::responder::PldmResponder
pub struct PldmRequester<'a> {
    cmd_interface: CmdInterface<'a>,
}

impl<'a> PldmRequester<'a> {
    /// Create a new PLDM requester with the given protocol capabilities.
    ///
    /// `protocol_capabilities` describes the PLDM types, versions, and
    /// commands that this device supports and advertises.
    pub fn new(protocol_capabilities: &'a [ProtocolCapability<'a>]) -> Self {
        PldmRequester {
            cmd_interface: CmdInterface::new(protocol_capabilities),
        }
    }

    /// Execute one FD-initiated request/response cycle.
    ///
    /// 1. Generates the next pending FD-initiated PLDM request via the
    ///    command interface.  If no request is pending, returns `Ok(())`
    ///    immediately without touching the transport.
    /// 2. Delegates the MCTP send/receive cycle to [`MctpPldmTransport::send_request`],
    ///    which opens a channel, sends the PLDM payload, receives the response,
    ///    and stamps the framing byte.
    /// 3. Passes the response back through the command interface for
    ///    protocol-level processing (e.g. storing downloaded firmware data,
    ///    recording transfer results).
    ///
    /// `buf` must be large enough to hold the 1-byte MCTP message-type prefix
    /// plus the largest PLDM message expected.  Byte 0 is reserved for that
    /// prefix; the PLDM payload occupies `buf[1..]`.
    ///
    /// A `timeout_millis` of `0` blocks indefinitely.
    ///
    /// # Errors
    ///
    /// Returns [`PldmServiceError::MsgHandler`] if the command interface
    /// cannot generate a request or process the response.
    /// Returns [`PldmServiceError::Mctp`] on any transport error (e.g.
    /// timeout, channel exhausted).
    /// Returns [`PldmServiceError::Overflow`] if `buf` is too small.
    pub fn run_once<C: MctpClient>(
        &mut self,
        transport: &MctpPldmTransport<C>,
        remote_eid: u8,
        buf: &mut [u8],
        timeout_millis: u32,
    ) -> Result<(), PldmServiceError> {
        // Step 1 – generate the next FD-initiated request.
        //
        // generate_initiator_request writes the MCTP type byte into buf[0]
        // and the PLDM request into buf[1..].  It returns the number of PLDM
        // bytes written (not counting buf[0]).  A return value of 0 means
        // there is no pending request and we can return early.
        let pldm_req_len = self
            .cmd_interface
            .generate_initiator_request(buf)
            .map_err(PldmServiceError::MsgHandler)?;

        if pldm_req_len == 0 {
            return Ok(());
        }

        // Steps 2–4 – send the request and receive the response.
        //
        // send_request stamps buf[0], sends buf[1..1+pldm_req_len], receives
        // the response into buf[1..], and returns the response length.
        let pldm_resp_len =
            transport.send_request(remote_eid, pldm_req_len, buf, timeout_millis)?;

        // Step 5 – process the response.
        let resp_end = pldm_resp_len
            .checked_add(1)
            .ok_or(PldmServiceError::Overflow)?;
        let resp_buf = buf.get_mut(..resp_end).ok_or(PldmServiceError::Overflow)?;

        self.cmd_interface
            .process_initiator_response(resp_buf)
            .map_err(PldmServiceError::MsgHandler)
    }
}
