// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Region and error types for the OTP driver.

use hal_otp_driver::{Error, ErrorKind, OtpRegion};

/// Error reported by the OTP controller.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct OtpError {
    kind: ErrorKind,
}

impl OtpError {
    /// Construct an error of the given kind.
    pub const fn new(kind: ErrorKind) -> Self {
        Self { kind }
    }
}

impl Error for OtpError {
    fn kind(&self) -> ErrorKind {
        self.kind
    }
}

/// A logical OTP partition addressed through the DAI.
///
/// `base` is the byte offset of the partition within the OTP address space
/// (also the value passed to the digest command when locking), and `size` is
/// the partition capacity in bytes. Access offsets are relative to `base`.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Partition {
    base: usize,
    size: usize,
}

impl Partition {
    /// Construct a partition from its byte base offset and capacity in bytes.
    pub const fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }

    /// Byte base offset of the partition within the OTP address space.
    pub const fn base(&self) -> usize {
        self.base
    }

    /// Partition capacity in bytes.
    pub const fn size(&self) -> usize {
        self.size
    }
}

impl OtpRegion for Partition {}
