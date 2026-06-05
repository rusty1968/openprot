// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use crate::smbus_protocol::{NotificationResult, ProtocolError, SmbusProtocol, Source};

/// Error returned by the SMB mailbox adapter when protocol policy or register
/// access fails.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterError {
    /// Protocol-layer validation or policy error.
    Protocol(ProtocolError),
    /// Low-level mailbox register access error.
    Mailbox(ast10x0_peripherals::smb_mbox::MailboxRegError),
}

impl From<ProtocolError> for AdapterError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}

impl From<ast10x0_peripherals::smb_mbox::MailboxRegError> for AdapterError {
    fn from(value: ast10x0_peripherals::smb_mbox::MailboxRegError) -> Self {
        Self::Mailbox(value)
    }
}

/// Bridges protocol policy (`SmbusProtocol`) to concrete SMB mailbox register I/O.
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

    /// Applies source-aware write filtering and commits the resulting value to
    /// the mailbox register.
    pub fn write_from_source<TMmio>(
        &self,
        mailbox: &ast10x0_peripherals::smb_mbox::Mailbox<'_, TMmio>,
        source: Source,
        addr: u8,
        value: u8,
    ) -> Result<u8, AdapterError>
    where
        TMmio: ureg::MmioMut + core::borrow::Borrow<TMmio>,
    {
        let filtered = self.protocol.filter_write(source, addr, value)?;
        mailbox.write_byte(addr, filtered)?;
        Ok(filtered)
    }

    /// Handles a mailbox notification by reading the current value, translating
    /// it into a protocol result, and applying any required write-back.
    pub fn handle_notification<TMmio>(
        &self,
        mailbox: &ast10x0_peripherals::smb_mbox::Mailbox<'_, TMmio>,
        addr: u8,
    ) -> Result<NotificationResult, AdapterError>
    where
        TMmio: ureg::MmioMut + core::borrow::Borrow<TMmio>,
    {
        let value = mailbox.read_byte(addr)?;
        let result = self.protocol.on_notification(addr, value);

        if let Some(write_back) = result.write_back {
            mailbox.write_byte(addr, write_back)?;
        }

        Ok(result)
    }
}
