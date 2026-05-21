// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! PLDM responder that processes incoming PLDM-over-MCTP messages.
//!
//! ## Buffer layout
//!
//! `CmdInterface` from `pldm-interface` operates on a single flat buffer
//! whose first byte is the MCTP message-type byte (0x01 for PLDM) followed
//! immediately by the PLDM header and payload:
//!
//! ```text
//! buf[0]          : MCTP message-type (0x01)
//! buf[1..]        : PLDM message (header + data)
//! ```
//!
//! The MCTP API's [`MctpListener::recv`] writes only the PLDM bytes (no
//! MCTP framing byte) into the supplied buffer.  [`PldmResponder::run_once`]
//! therefore receives into `buf[1..]` and sets `buf[0]` before handing the
//! whole slice to `CmdInterface`.  The PLDM response (also without the MCTP
//! framing byte) is then extracted from `buf[1..resp_len]` and sent back via
//! the response channel.

use openprot_mctp_api::MctpClient;
use pldm_common::util::mctp_transport::MCTP_PLDM_MSG_TYPE;
use pldm_interface::cmd_interface::CmdInterface;
use pldm_interface::control_context::ProtocolCapability;

use crate::error::PldmServiceError;
use crate::transport::MctpPldmTransport;

/// The MCTP message-type value used for PLDM (0x01).
pub const PLDM_MSG_TYPE: u8 = MCTP_PLDM_MSG_TYPE;

/// PLDM responder service.
///
/// Wraps a [`CmdInterface`] and processes incoming PLDM messages received
/// over an MCTP transport provided by a [`MctpPldmTransport`].
///
/// # Example
///
/// ```rust,ignore
/// use openprot_pldm_service::responder::{PldmResponder, PLDM_MSG_TYPE};
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
/// static CAPS: [ProtocolCapability<'static>; 1] = [
///     ProtocolCapability::new(PldmSupportedType::Base, "1.1.0", &CTRL_CMDS).unwrap(),
/// ];
///
/// let transport = MctpPldmTransport::new(client);
/// let mut responder = PldmResponder::new(&CAPS);
/// let mut buf = [0u8; 1024];
/// loop {
///     if let Err(e) = responder.run_once(&transport, &mut buf, 0) {
///         // handle error
///     }
/// }
/// ```
pub struct PldmResponder<'a> {
    cmd_interface: CmdInterface<'a>,
}

impl<'a> PldmResponder<'a> {
    /// Create a new PLDM responder with the given protocol capabilities.
    ///
    /// `protocol_capabilities` describes the PLDM types, versions, and
    /// commands that this responder advertises and handles.
    pub fn new(protocol_capabilities: &'a [ProtocolCapability<'a>]) -> Self {
        PldmResponder {
            cmd_interface: CmdInterface::new(protocol_capabilities),
        }
    }

    /// Receive and handle one incoming PLDM message.
    ///
    /// Delegates to [`MctpPldmTransport::recv_and_respond`]: opens a PLDM
    /// listener, waits up to `timeout_millis` milliseconds for one message,
    /// dispatches it through the PLDM command interface in-place, and sends
    /// the response back.  The MCTP listener is released before this method
    /// returns.
    ///
    /// `buf` must be large enough to hold the 1-byte MCTP message-type prefix
    /// plus the largest PLDM message expected.  Byte 0 is reserved for that
    /// prefix; the PLDM payload is placed in `buf[1..]`.
    ///
    /// A `timeout_millis` of `0` blocks indefinitely.
    ///
    /// # Errors
    ///
    /// Returns [`PldmServiceError::Mctp`] on any transport error (e.g.
    /// timeout, server restart).  Returns [`PldmServiceError::MsgHandler`] if
    /// the PLDM command interface cannot process the message.  Returns
    /// [`PldmServiceError::Overflow`] if `buf` is too small.
    pub fn run_once<C: MctpClient>(
        &mut self,
        transport: &MctpPldmTransport<C>,
        buf: &mut [u8],
        timeout_millis: u32,
    ) -> Result<(), PldmServiceError> {
        transport.recv_and_respond(buf, timeout_millis, |framed_buf| {
            self.cmd_interface
                .handle_responder_msg(framed_buf)
                .map_err(PldmServiceError::MsgHandler)
        })
    }
}
