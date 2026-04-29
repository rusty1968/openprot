// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 IPC-based serial transport implementation.
//!
//! This module implements the `SerialPort` trait for the AST10x0 microkernel
//! architecture, providing access to the userspace USART driver via IPC syscalls.

use embedded_io::Write;
use mctp::Result;
use openprot_mctp_transport_serial::{SerialError, SerialPort};
use usart_client::UsartClient;

impl SerialPort for UsartClient {
    fn configure(&mut self, baud_rate: u32) -> Result<(), SerialError> {
        self.configure(baud_rate).map_err(|_| SerialError::Io)
    }
}

impl Write for UsartClient {
    fn write(&mut self, buf: &[u8]) -> Result<usize, embedded_io::Error> {
        self.write(buf)
            .map_err(|_| embedded_io::Error::Other)
            .map(|_| buf.len())
    }

    fn flush(&mut self) -> Result<(), embedded_io::Error> {
        // USART driver handles write buffering; nothing to do locally
        Ok(())
    }
}
