// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

mod common;

/// Direct HAL-based serial implementation guide and examples.
pub mod direct;

/// IPC-based (microkernel) serial implementation guide and examples.
pub mod ipc;

pub use common::{SerialError, SerialPort};
