// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use crate::smbus_protocol::{NotificationResult, ProtocolError, ProtocolEvent, SmbusProtocol, Source};
use i2c_api::seam::SevenBitAddress;
use i2c_api::{SlaveEvent, Transport};
use i2c_client::{ClientError as I2cClientError, I2cClient};

const MAILBOX_SIZE: usize = 256;
const RX_BUFFER_SIZE: usize = 256;

/// Source-address mapping for writes received over the i2c service slave path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceAddressMap {
    pub bmc: SevenBitAddress,
    pub pch_cpu: SevenBitAddress,
    pub fallback: Option<Source>,
}

/// Errors returned by the i2c-service-backed SMBus client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum I2cPfrClientError {
    I2c(I2cClientError),
    Protocol(ProtocolError),
    UnknownSource,
    InvalidAddress,
}

impl From<I2cClientError> for I2cPfrClientError {
    fn from(value: I2cClientError) -> Self {
        Self::I2c(value)
    }
}

impl From<ProtocolError> for I2cPfrClientError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}

/// Concrete client that consumes i2c service slave notifications and applies
/// SMBus protocol policy to a local mailbox shadow.
pub struct I2cPfrSmbusClient<T: Transport> {
    i2c: I2cClient<T>,
    protocol: SmbusProtocol,
    sources: SourceAddressMap,
    mailbox: [u8; MAILBOX_SIZE],
    read_cursor: u8,
}

impl<T: Transport> I2cPfrSmbusClient<T> {
    /// Creates a new i2c-backed PFR SMBus client.
    pub const fn new(i2c: I2cClient<T>, protocol: SmbusProtocol, sources: SourceAddressMap) -> Self {
        Self {
            i2c,
            protocol,
            sources,
            mailbox: [0; MAILBOX_SIZE],
            read_cursor: 0,
        }
    }

    /// Configures and enables target mode for the given slave address.
    pub fn start(&mut self, slave_address: SevenBitAddress) -> Result<(), I2cPfrClientError> {
        self.i2c.configure_slave(slave_address)?;
        self.i2c.enable_slave()?;
        self.i2c.enable_notification()?;
        Ok(())
    }

    /// Disables notifications and leaves target mode.
    pub fn stop(&mut self) -> Result<(), I2cPfrClientError> {
        self.i2c.disable_notification()?;
        self.i2c.disable_slave()?;
        Ok(())
    }

    /// Returns the mailbox shadow byte.
    pub fn mailbox_byte(&self, addr: u8) -> Result<u8, I2cPfrClientError> {
        self.mailbox
            .get(addr as usize)
            .copied()
            .ok_or(I2cPfrClientError::InvalidAddress)
    }

    /// Processes one pending i2c slave event (if available) and emits decoded protocol events.
    pub fn poll_once<F>(&mut self, mut emit: F) -> Result<(), I2cPfrClientError>
    where
        F: FnMut(ProtocolEvent),
    {
        let mut rx = [0u8; RX_BUFFER_SIZE];
        let event = match self.i2c.slave_receive(&mut rx) {
            Ok(ev) => ev,
            Err(I2cClientError::ServerError(i2c_api::I2cError::NoData)) => return Ok(()),
            Err(e) => return Err(I2cPfrClientError::I2c(e)),
        };

        match event.kind {
            SlaveEvent::DataReceived => {
                self.handle_data_received(&rx[..event.data_len], event.source_address, &mut emit)
            }
            SlaveEvent::ReadRequest => self.handle_read_request(),
            SlaveEvent::Stop => Ok(()),
            _ => Ok(()),
        }
    }

    fn resolve_source(&self, source_addr: Option<SevenBitAddress>) -> Result<Source, I2cPfrClientError> {
        match source_addr {
            Some(addr) if addr == self.sources.bmc => Ok(Source::Bmc),
            Some(addr) if addr == self.sources.pch_cpu => Ok(Source::PchCpu),
            Some(_) => self.sources.fallback.ok_or(I2cPfrClientError::UnknownSource),
            None => self.sources.fallback.ok_or(I2cPfrClientError::UnknownSource),
        }
    }

    fn handle_read_request(&mut self) -> Result<(), I2cPfrClientError> {
        let value = self.mailbox[self.read_cursor as usize];
        self.i2c.slave_set_response(&[value])?;
        self.read_cursor = self.read_cursor.wrapping_add(1);
        Ok(())
    }

    fn handle_data_received<F>(
        &mut self,
        data: &[u8],
        source_addr: Option<SevenBitAddress>,
        emit: &mut F,
    ) -> Result<(), I2cPfrClientError>
    where
        F: FnMut(ProtocolEvent),
    {
        if data.is_empty() {
            return Ok(());
        }

        let base_addr = data[0];
        self.read_cursor = base_addr;

        if data.len() == 1 {
            return Ok(());
        }

        let source = self.resolve_source(source_addr)?;
        for (i, raw) in data[1..].iter().enumerate() {
            let addr = base_addr.wrapping_add(i as u8);
            let filtered = self.protocol.filter_write(source, addr, *raw)?;
            self.mailbox[addr as usize] = filtered;

            let result = self.protocol.on_notification(addr, filtered);
            self.apply_result(addr, result, emit);
        }

        Ok(())
    }

    fn apply_result<F>(&mut self, addr: u8, result: NotificationResult, emit: &mut F)
    where
        F: FnMut(ProtocolEvent),
    {
        if let Some(write_back) = result.write_back {
            self.mailbox[addr as usize] = write_back;
        }

        if let Some(event) = result.event {
            emit(event);
        }
    }
}
