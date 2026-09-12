// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! OTP (One-Time-Programmable fuse) controller driver for the Caliptra
//! Subsystem OTP macro.
//!
//! The controller is driven through its Direct Access Interface (DAI): software
//! programs an address, issues a read/write/digest command, and polls the
//! `DaiIdle` status bit for completion. This module wraps that protocol behind
//! the composable blocking OTP HAL traits in [`hal_otp_driver`].

mod controller;
mod types;

pub use controller::OtpController;
pub use types::{OtpError, Partition};

pub use hal_otp_driver::{ErrorKind, OtpOffset, OtpRegionStatus};
