// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Direct HAL-based serial port implementation.
//!
//! This module provides synchronous, direct hardware access to a UART device
//! via a platform's Hardware Abstraction Layer (HAL).
//!
//! # Implementation Example
//!
//! Implement `SerialPort` for your HAL's UART type:
//!
//! ```ignore
//! use embedded_io::Write;
//! use openprot_mctp_transport_serial::{SerialPort, SerialError};
//! use mctp::Result;
//!
//! // Your platform's HAL UART type
//! pub struct MyUart { /* ... */ }
//!
//! impl SerialPort for MyUart {
//!     fn configure(&mut self, baud_rate: u32) -> Result<(), SerialError> {
//!         // Configure hardware baud rate
//!         self.set_baud(baud_rate)
//!             .map_err(|_| SerialError::Io)
//!     }
//! }
//!
//! impl Write for MyUart {
//!     fn write(&mut self, buf: &[u8]) -> Result<usize, embedded_io::Error> {
//!         // Direct hardware write
//!         self.write_all(buf)?;
//!         Ok(buf.len())
//!     }
//!
//!     fn flush(&mut self) -> Result<(), embedded_io::Error> {
//!         self.hw_flush()
//!     }
//! }
//! ```
//!
//! # Characteristics
//!
//! - **Synchronous**: Hardware operations block until complete
//! - **Direct Access**: No intermediary processes or IPC
//! - **Low Latency**: Minimal overhead compared to IPC
//! - **Single Context**: All operations in kernel or single process context

// Placeholder: actual implementations exist in platform-specific targets
// (e.g., target/ast10x0/serial/direct.rs with actual types like Uart)
