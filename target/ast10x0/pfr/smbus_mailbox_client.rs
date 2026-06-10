// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use crate::swmbx_ctrl::{SwmbxCtrl, SwmbxError};
use i2c_api::seam::SevenBitAddress;
use i2c_api::{SlaveEvent, Transport};
use i2c_client::{ClientError as I2cClientError, I2cClient};

const RX_BUFFER_SIZE: usize = 256;

/// Logical source port for mailbox transactions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// Baseboard management controller source.
    Bmc,
    /// Host CPU/PCH source.
    PchCpu,
}

/// Source-address mapping for writes received over the i2c service slave path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceAddressMap {
    /// Seven-bit source address used to identify BMC-originated writes.
    pub bmc: SevenBitAddress,
    /// Seven-bit source address used to identify PCH/CPU-originated writes.
    pub pch_cpu: SevenBitAddress,
}

/// Errors returned by the i2c-service-backed SMBus client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum I2cPfrClientError {
    /// Underlying I2C client/service failure.
    I2c(I2cClientError),
    /// SW mailbox controller operation failure.
    Swmbx(SwmbxError),
    /// Source address did not match any configured source mapping.
    UnknownSource,
}

impl From<I2cClientError> for I2cPfrClientError {
    fn from(value: I2cClientError) -> Self {
        Self::I2c(value)
    }
}

impl From<SwmbxError> for I2cPfrClientError {
    fn from(value: SwmbxError) -> Self {
        Self::Swmbx(value)
    }
}

/// Concrete client that consumes i2c service slave notifications and applies
/// mailbox transactions to the SWMBX controller.
pub struct I2cPfrSmbusClient<T: Transport> {
    i2c: I2cClient<T>,
    swmbx: SwmbxCtrl,
    sources: SourceAddressMap,
    read_cursor: u8,
    active_port: usize,
    first_write: bool,
}

impl<T: Transport> I2cPfrSmbusClient<T> {
    /// Creates a new i2c-backed PFR SMBus client.
    pub fn new(i2c: I2cClient<T>, swmbx: SwmbxCtrl, sources: SourceAddressMap) -> Self {
        Self {
            i2c,
            swmbx,
            sources,
            read_cursor: 0,
            active_port: 0,
            first_write: true,
        }
    }

    /// Returns a mutable handle to the underlying SWMBX controller.
    pub fn controller_mut(&mut self) -> &mut SwmbxCtrl {
        &mut self.swmbx
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

    /// Returns one byte from the software mailbox controller buffer path.
    pub fn mailbox_byte(&mut self, addr: u8) -> Result<u8, I2cPfrClientError> {
        Ok(self.swmbx.swmbx_read(false, addr)?)
    }

    /// Processes all currently pending I2C slave events.
    ///
    /// This mirrors the Zephyr SWMBX target callback flow:
    ///
    /// - `DataReceived`: consumes received bytes through `handle_data_received`.
    ///   The first byte is treated as mailbox offset and opens a transaction
    ///   with `send_start`; following bytes are written via `send_msg` with
    ///   wrapping cursor semantics.
    /// - `ReadRequest`: reads one byte from the active port/cursor via
    ///   `get_msg`, publishes it with `slave_set_response`, then advances the
    ///   cursor.
    /// - `Stop`: finalizes the transaction via `send_stop` and resets
    ///   first-write state.
    ///
    /// Returns `Ok(true)` if at least one event was handled, `Ok(false)` if
    /// the queue is empty (`NoData`). The method drains the queue fully so a
    /// `DataReceived`/`Stop` burst cannot be split across two caller wake-ups.
    /// That keeps `first_write`, `active_port`, and `read_cursor` consistent
    /// for the next FIFO-backed access.
    ///
    /// # Errors
    /// Returns `I2cPfrClientError::I2c` for transport failures and propagates
    /// `Swmbx` errors encountered while handling an event.
    pub fn process_one_event(&mut self) -> Result<bool, I2cPfrClientError> {
        let mut rx = [0u8; RX_BUFFER_SIZE];
        let mut handled = false;

        loop {
            let event = match self.i2c.slave_receive(&mut rx) {
                Ok(ev) => ev,
                Err(I2cClientError::ServerError(i2c_api::I2cError::NoData)) => {
                    return Ok(handled);
                }
                Err(e) => return Err(I2cPfrClientError::I2c(e)),
            };

            handled = true;
            match event.kind {
                SlaveEvent::DataReceived => {
                    self.handle_data_received(&rx[..event.data_len], event.source_address)?
                }
                SlaveEvent::ReadRequest => self.handle_read_request()?,
                SlaveEvent::Stop => {
                    self.swmbx.send_stop(self.active_port)?;
                    // The next write transaction starts with an offset byte.
                    self.first_write = true;
                }
                _ => {}
            }
        }
    }

    /// Resolve the originating port. A recognized MCTP source address selects
    /// its port; a plain offset-addressed SMBus access (no source header)
    /// defaults to port 0 so it is served, not dropped.
    fn resolve_source(&self, source_addr: Option<SevenBitAddress>) -> Source {
        match source_addr {
            Some(addr) if addr == self.sources.pch_cpu => Source::PchCpu,
            _ => Source::Bmc,
        }
    }

    fn handle_read_request(&mut self) -> Result<(), I2cPfrClientError> {
        let value = self.swmbx.get_msg(self.active_port, self.read_cursor)?;
        self.i2c.slave_set_response(&[value])?;
        self.read_cursor = self.read_cursor.wrapping_add(1);
        Ok(())
    }

    fn handle_data_received(
        &mut self,
        data: &[u8],
        source_addr: Option<SevenBitAddress>,
    ) -> Result<(), I2cPfrClientError> {
        if data.is_empty() {
            return Ok(());
        }

        let source = self.resolve_source(source_addr);
        self.active_port = source_to_port(source);

        // Mirrors Zephyr swmbx_target callback sequencing:
        // first byte selects address and opens transaction; following bytes write.
        for byte in data {
            if self.first_write {
                self.read_cursor = *byte;
                self.swmbx.send_start(self.active_port, self.read_cursor)?;
                self.first_write = false;
            } else {
                self.swmbx.send_msg(self.active_port, self.read_cursor, *byte)?;
                self.read_cursor = self.read_cursor.wrapping_add(1);
            }
        }

        Ok(())
    }
}

fn source_to_port(source: Source) -> usize {
    match source {
        Source::Bmc => 0,
        Source::PchCpu => 1,
    }
}
