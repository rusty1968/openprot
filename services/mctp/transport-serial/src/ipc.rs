// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! IPC-based serial port implementation for microkernel architectures.
//!
//! This module provides synchronous (from caller's perspective) serial access
//! via Inter-Process Communication (IPC) syscalls to a userspace driver.
//!
//! # Implementation Example
//!
//! Implement `SerialPort` for your IPC client type:
//!
//! ```ignore
//! use embedded_io::Write;
//! use openprot_mctp_transport_serial::{SerialPort, SerialError};
//! use mctp::Result;
//! use usart_client::UsartClient;
//!
//! impl SerialPort for UsartClient {
//!     fn configure(&mut self, baud_rate: u32) -> Result<(), SerialError> {
//!         // IPC syscall to USART driver server
//!         self.configure(baud_rate)
//!             .map_err(|_| SerialError::Io)
//!     }
//! }
//!
//! impl Write for UsartClient {
//!     fn write(&mut self, buf: &[u8]) -> Result<usize, embedded_io::Error> {
//!         // Blocking IPC syscall to server
//!         self.write(buf)
//!             .map_err(|_| embedded_io::Error::Other)
//!             .map(|_| buf.len())
//!     }
//!
//!     fn flush(&mut self) -> Result<(), embedded_io::Error> {
//!         // Server handles buffering; nothing to do locally
//!         Ok(())
//!     }
//! }
//! ```
//!
//! # Characteristics
//!
//! - **IPC-Based**: Communicates with userspace driver via syscalls
//! - **Blocking Syscalls**: Caller blocks until driver responds (async underneath)
//! - **Separated Concerns**: Hardware driver in separate process
//! - **Multiprocess**: Enables privilege separation and isolation
//! - **Deterministic Latency**: Microkernel scheduling controls driver response

// Placeholder: actual implementations exist in platform-specific targets
// (e.g., target/ast10x0/serial/ipc.rs with actual types like UsartClient)
