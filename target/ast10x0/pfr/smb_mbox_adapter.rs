// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use crate::smbus_protocol::{NotificationResult, ProtocolError, SmbusProtocol, Source};
use crate::swmbx_ctrl::{SwmbxCtrl, SwmbxError};

/// Error returned by the SMB mailbox adapter when protocol policy or software
/// mailbox access fails.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterError {
    /// Protocol-layer validation or policy error.
    Protocol(ProtocolError),
    /// Software mailbox controller error.
    Swmbx(SwmbxError),
}

impl From<ProtocolError> for AdapterError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}

impl From<SwmbxError> for AdapterError {
    fn from(value: SwmbxError) -> Self {
        Self::Swmbx(value)
    }
}

/// Bridges protocol policy (`SmbusProtocol`) to the software mailbox controller
/// ([`SwmbxCtrl`]).
///
/// The adapter is stateless: the mailbox storage and its protect/notify/FIFO
/// state live in the caller-owned [`SwmbxCtrl`], which is borrowed per call.
pub struct SmbMboxAdapter {
    protocol: SmbusProtocol,
}

impl SmbMboxAdapter {
    /// Creates a mailbox adapter from a protocol policy instance.
    pub const fn new(protocol: SmbusProtocol) -> Self {
        Self { protocol }
    }

    /// Returns the protocol policy used by this adapter.
    pub const fn protocol(&self) -> &SmbusProtocol {
        &self.protocol
    }

    /// Maps a write source domain onto its [`SwmbxCtrl`] port index.
    ///
    /// Mirrors the controller's port layout (see the crate README): port 0 is
    /// the BMC domain, port 1 is the PCH/CPU domain.
    const fn port_for_source(source: Source) -> usize {
        match source {
            Source::Bmc => 0,
            Source::PchCpu => 1,
        }
    }

    /// Applies source-aware write filtering and commits the resulting value to
    /// the software mailbox through the controller's port-aware write path.
    ///
    /// The write is routed through [`SwmbxCtrl::send_msg`], so the controller's
    /// per-node write protection, change-notification, and FIFO remapping all
    /// apply on top of the protocol-layer filtering. If a FIFO transaction is
    /// open on the resolved port (via [`SwmbxCtrl::send_start`]), the byte is
    /// appended to that FIFO; otherwise it lands in the flat buffer.
    pub fn write_from_source(
        &self,
        ctrl: &mut SwmbxCtrl,
        source: Source,
        addr: u8,
        value: u8,
    ) -> Result<u8, AdapterError> {
        let filtered = self.protocol.filter_write(source, addr, value)?;
        let port = Self::port_for_source(source);
        ctrl.send_msg(port, addr, filtered)?;
        Ok(filtered)
    }

    /// Handles a mailbox notification by reading the current register value,
    /// translating it into a protocol result, and applying any required
    /// write-back.
    ///
    /// Both the read and the write-back use the controller's direct register
    /// helpers ([`SwmbxCtrl::swmbx_read`]/[`SwmbxCtrl::swmbx_write`]) so that the
    /// decode observes the committed register byte and firmware-initiated
    /// write-backs (for example, clearing a reset-communication request to `0`)
    /// bypass per-node write protection.
    pub fn handle_notification(
        &self,
        ctrl: &mut SwmbxCtrl,
        addr: u8,
    ) -> Result<NotificationResult, AdapterError> {
        let value = ctrl.swmbx_read(false, addr)?;
        let result = self.protocol.on_notification(addr, value);

        if let Some(write_back) = result.write_back {
            ctrl.swmbx_write(false, addr, write_back)?;
        }

        Ok(result)
    }
}
