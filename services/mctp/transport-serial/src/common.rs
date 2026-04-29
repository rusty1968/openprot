// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use embedded_io::Write;

/// Generic serial port trait for transporting MCTP over serial.
///
/// Implement this for your platform's serial backend (either direct HAL or IPC).
pub trait SerialPort: Write {
    /// Configure the serial port baud rate.
    fn configure(&mut self, baud_rate: u32) -> Result<(), SerialError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialError {
    Io,
    Timeout,
    Invalid,
}
