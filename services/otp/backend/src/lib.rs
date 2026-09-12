// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Caliptra Subsystem backend for the OTP userspace service.
//!
//! Bridges the vendor-neutral OTP service ([`otp_server::dispatch`]) to the
//! Caliptra SS OTP macro. Two adapters live here:
//!
//! - [`CaliptraOtpBackend`] wraps the [`veer_peripherals`] DAI driver and
//!   implements the byte-oriented HAL traits keyed by the wire-stable
//!   [`otp_api::RegionId`], translating each region to its Caliptra fuse
//!   partition.
//! - [`board`] supplies the SoC-specific policy the wire deliberately does not
//!   carry: authorization, SVN fuse encoding, and fuse geometry.

#![no_std]

mod backend;
pub mod board;

pub use backend::CaliptraOtpBackend;
pub use board::{
    CaliptraAccessPolicy, CaliptraFieldMap, CaliptraSvnCodec, FIELD_SOC_MANIFEST_SVN,
    FIELD_VENDOR_PK_HASH_MANUF, PROVISIONER,
};
