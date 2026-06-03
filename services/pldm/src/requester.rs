// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! PLDM requester that sends queued PLDM messages over MCTP and processes
//! responses.
//!
//! [`PldmRequester`] acts as a PLDM *initiator*: it takes a queued command,
//! sends it to a remote endpoint over MCTP, and validates the response. It
//! complements the responder-side [`PldmResponder`], which handles inbound
//! requests.
//!
//! ## Buffer layout
//!
//! The buffer passed to [`PldmRequester::run_once`] uses the same layout as
//! [`PldmResponder`]:
//!
//! ```text
//! buf[0]          : MCTP message-type (0x01) – written by send_request
//! buf[1..]        : PLDM message (header + data)
//! ```
//!
//! [`PldmResponder`]: crate::responder::PldmResponder

use openprot_mctp_api::MctpClient;
use pldm_common::codec::PldmCodec;
use pldm_common::message::control::{GetTidRequest, GetTidResponse};
use pldm_common::protocol::base::{PldmBaseCompletionCode, PldmMsgType};
use pldm_interface::control_context::ProtocolCapability;
use pldm_interface::error::MsgHandlerError;

use crate::error::PldmServiceError;
use crate::transport::MctpPldmTransport;

/// One outbound requester command to send on the next [`PldmRequester::run_once`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PldmRequesterCommand {
    /// PLDM Base `GetTID` request.
    GetTid,
}

/// PLDM requester service (initiator mode).
///
/// Sends a PLDM request over an MCTP transport provided by a
/// [`MctpPldmTransport`] and validates the response.  Use this alongside a
/// [`PldmResponder`] to exercise a complete request/response exchange.
///
/// # Example
///
/// ```rust,ignore
/// use openprot_pldm_service::requester::PldmRequester;
/// use openprot_pldm_service::transport::MctpPldmTransport;
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
/// static CAPS: [ProtocolCapability<'static>; 1] = [ProtocolCapability {
///     pldm_type: PldmSupportedType::Base,
///     protocol_version: 0xF1F1F000,
///     supported_commands: &CTRL_CMDS,
/// }];
///
/// let transport = MctpPldmTransport::new(client);
/// let mut requester = PldmRequester::new(&CAPS);
/// let mut buf = [0u8; 1024];
/// requester.queue_get_tid();
/// requester.run_once(&transport, REMOTE_EID, &mut buf, 0).unwrap();
/// ```
///
/// [`PldmResponder`]: crate::responder::PldmResponder
pub struct PldmRequester {
    /// Instance ID stamped into the next outgoing request header.  Incremented
    /// (with wraparound) after each completed exchange so successive requests
    /// carry distinct instance IDs, as required by the PLDM base spec.
    instance_id: u8,
    /// Pending command to send on the next [`run_once`](Self::run_once).
    pending_command: Option<PldmRequesterCommand>,
}

impl PldmRequester {
    /// Create a new PLDM requester.
    ///
    /// `protocol_capabilities` describes the PLDM types, versions, and commands
    /// the local endpoint advertises. It is accepted for symmetry with
    /// [`PldmResponder::new`] and future expansion.
    ///
    /// [`PldmResponder::new`]: crate::responder::PldmResponder::new
    pub fn new(_protocol_capabilities: &[ProtocolCapability]) -> Self {
        PldmRequester {
            instance_id: 0,
            pending_command: None,
        }
    }

    /// Queue a requester command for the next [`run_once`](Self::run_once).
    pub fn queue_command(&mut self, command: PldmRequesterCommand) {
        self.pending_command = Some(command);
    }

    /// Convenience helper that queues a PLDM Base `GetTID` request.
    pub fn queue_get_tid(&mut self) {
        self.queue_command(PldmRequesterCommand::GetTid);
    }

    /// Execute one queued request/response cycle.
    ///
    /// If no command is queued, this method returns `Ok(())` and does not
    /// touch the transport.
    ///
    /// `buf` must be large enough to hold the 1-byte MCTP message-type prefix
    /// plus the largest PLDM message expected.  Byte 0 is reserved for that
    /// prefix; the PLDM payload occupies `buf[1..]`.
    ///
    /// A `timeout_millis` of `0` blocks indefinitely.
    ///
    /// # Errors
    ///
    /// Returns [`PldmServiceError::MsgHandler`] if the request cannot be
    /// encoded, the response cannot be decoded, or the responder reports a
    /// non-success completion code.
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
        let Some(command) = self.pending_command else {
            return Ok(());
        };

        let result = match command {
            PldmRequesterCommand::GetTid => {
                self.run_get_tid_once(transport, remote_eid, buf, timeout_millis)
            }
        };

        if result.is_ok() {
            self.pending_command = None;
        }

        result
    }

    fn run_get_tid_once<C: MctpClient>(
        &mut self,
        transport: &MctpPldmTransport<C>,
        remote_eid: u8,
        buf: &mut [u8],
        timeout_millis: u32,
    ) -> Result<(), PldmServiceError> {
        // Step 1 – encode the GetTID request into buf[1..]; buf[0] is reserved
        // for the MCTP framing byte that send_request stamps.
        let request = GetTidRequest::new(self.instance_id, PldmMsgType::Request);
        let pldm_req_len = {
            let pldm_buf = buf.get_mut(1..).ok_or(PldmServiceError::Overflow)?;
            request
                .encode(pldm_buf)
                .map_err(|e| PldmServiceError::MsgHandler(MsgHandlerError::Codec(e)))?
        };

        // Step 2 – send the request and receive the response into buf[1..].
        let pldm_resp_len =
            transport.send_request(remote_eid, pldm_req_len, buf, timeout_millis)?;

        // Step 3 – decode and validate the response.
        let resp_end = pldm_resp_len
            .checked_add(1)
            .ok_or(PldmServiceError::Overflow)?;
        let resp_buf = buf.get(1..resp_end).ok_or(PldmServiceError::Overflow)?;
        let response = GetTidResponse::decode(resp_buf)
            .map_err(|e| PldmServiceError::MsgHandler(MsgHandlerError::Codec(e)))?;

        // Copy out of the packed response before comparing.
        let completion_code = response.completion_code;
        if completion_code != PldmBaseCompletionCode::Success as u8 {
            return Err(PldmServiceError::MsgHandler(
                MsgHandlerError::FdInitiatorModeError,
            ));
        }

        // Advance the instance ID for the next exchange.
        self.instance_id = self.instance_id.wrapping_add(1);

        Ok(())
    }
}
