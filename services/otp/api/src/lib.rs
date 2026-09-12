// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Wire protocol for the OTP userspace service.
//!
//! The service exposes named fuse fields and raw region access over IPC. The
//! sensitive policy — authorization, anti-rollback monotonicity, and fuse
//! encoding — is never on the wire; it lives in `otp-server`. This crate holds
//! only the marshalling vocabulary shared by client and server, and builds on
//! the host with no kernel dependency.

#![no_std]

pub mod protocol;

#[doc(inline)]
pub use protocol::{
    FieldId, OtpOp, OtpRequestHeader, OtpResponseHeader, OtpWireError, RegionId, MAX_FIELD,
    MAX_PAYLOAD_SIZE, REGION_SVN, REGION_VENDOR_HASHES_MANUF,
};
