// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

#[cfg(serial_direct)]
mod direct;
#[cfg(serial_ipc)]
mod ipc;

#[cfg(serial_direct)]
pub use direct::Ast10x0DirectSerial as Backend;
#[cfg(serial_ipc)]
pub use ipc::Ast10x0IpcSerial as Backend;

#[cfg(serial_direct)]
pub use direct::Ast10x0DirectSerial;
#[cfg(serial_ipc)]
pub use ipc::{Ast10x0IpcSerial, IpcSerialError};
