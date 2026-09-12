// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! OTP service wire protocol.
//!
//! ```text
//! request : [OtpRequestHeader]  (+ inline payload for ProgramBytes / CommitSvnFloor)
//! response: [OtpResponseHeader] (+ read payload)
//! ```
//!
//! Fields carry no raw geometry across the boundary for named reads: the
//! server resolves a [`FieldId`] to `(region, offset, len)`. Raw
//! [`OtpOp::ReadBytes`]/[`OtpOp::ProgramBytes`] name a `region` + `offset`
//! explicitly for callers that already know the layout.

use hal_otp_driver::{OtpOffset, OtpRegion};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// Max read/write payload carried in one message.
pub const MAX_PAYLOAD_SIZE: usize = 256;

/// Largest single fuse field this schema addresses (bytes). Bounds the
/// server's on-stack scratch for read-modify-write of a field.
pub const MAX_FIELD: usize = 64;

/// Wire-stable region ids. The board maps hardware partitions onto these.
pub const REGION_SVN: u16 = 0;
pub const REGION_VENDOR_HASHES_MANUF: u16 = 1;

/// Concrete, wire-stable region identifier. The HAL's `OtpRegion` is an
/// associated type; over IPC it must be a concrete value.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RegionId(pub u16);

impl OtpRegion for RegionId {}

/// Operations the service understands.
#[non_exhaustive]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtpOp {
    /// Raw read: `region` + `offset` + `len` → bytes.
    ReadBytes = 0x01,
    /// Named read: `field` → bytes.
    ReadField = 0x02,
    /// Raw program (privileged): `region` + `offset` + inline payload.
    ProgramBytes = 0x10,
    /// Advance the anti-rollback floor for `field` (privileged). The inline
    /// payload carries the candidate value in the board-agreed form; the
    /// server owns the monotonic guard and the fuse encoding.
    CommitSvnFloor = 0x20,
}

impl TryFrom<u8> for OtpOp {
    type Error = OtpWireError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::ReadBytes),
            0x02 => Ok(Self::ReadField),
            0x10 => Ok(Self::ProgramBytes),
            0x20 => Ok(Self::CommitSvnFloor),
            _ => Err(OtpWireError::Unsupported),
        }
    }
}

/// Opaque, wire-stable fuse field identifier. The wire carries only this
/// scalar; the server assigns each id its meaning and `(region, offset, len)`
/// geometry per SoC. Client and server must agree on the id assignment out of
/// band — the protocol itself ascribes no semantics.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct FieldId(pub u16);

/// Status / error code carried in [`OtpResponseHeader`].
#[non_exhaustive]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtpWireError {
    InvalidAddress = 0x01,
    Alignment = 0x02,
    RegionProtected = 0x03,
    Hardware = 0x04,
    Timeout = 0x05,
    Unsupported = 0x06,
    /// Caller lacks the capability for the requested operation.
    NotAuthorized = 0x07,
    Internal = 0xFF,
}

impl From<u8> for OtpWireError {
    fn from(value: u8) -> Self {
        match value {
            0x01 => Self::InvalidAddress,
            0x02 => Self::Alignment,
            0x03 => Self::RegionProtected,
            0x04 => Self::Hardware,
            0x05 => Self::Timeout,
            0x06 => Self::Unsupported,
            0x07 => Self::NotAuthorized,
            _ => Self::Internal,
        }
    }
}

impl core::fmt::Display for OtpWireError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidAddress => "otp: address outside region",
            Self::Alignment => "otp: misaligned access",
            Self::RegionProtected => "otp: region protected",
            Self::Hardware => "otp: hardware error",
            Self::Timeout => "otp: timeout",
            Self::Unsupported => "otp: unsupported operation",
            Self::NotAuthorized => "otp: not authorized",
            Self::Internal => "otp: internal error",
        })
    }
}

impl core::error::Error for OtpWireError {}

/// Fixed-size request header. `region`/`field`/`offset`/`len` are
/// interpreted per [`OtpOp`]; unused fields are zero.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct OtpRequestHeader {
    op_code: u8,
    flags: u8,
    region: u16,
    field: u16,
    len: u16,
    offset: u32,
}

impl OtpRequestHeader {
    pub const SIZE: usize = core::mem::size_of::<Self>();

    pub fn new(op: OtpOp, region: u16, field: u16, len: u16, offset: u32) -> Self {
        Self {
            op_code: op as u8,
            flags: 0,
            region: region.to_le(),
            field: field.to_le(),
            len: len.to_le(),
            offset: offset.to_le(),
        }
    }

    pub fn op(&self) -> Result<OtpOp, OtpWireError> {
        OtpOp::try_from(self.op_code)
    }

    pub fn region(&self) -> RegionId {
        RegionId(u16::from_le(self.region))
    }

    pub fn field(&self) -> FieldId {
        FieldId(u16::from_le(self.field))
    }

    pub fn len(&self) -> usize {
        u16::from_le(self.len) as usize
    }

    pub fn offset(&self) -> OtpOffset {
        OtpOffset::new(u32::from_le(self.offset) as usize)
    }
}

/// Fixed-size response header, followed by `payload_len` bytes.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct OtpResponseHeader {
    status: u8,
    flags: u8,
    payload_len: u16,
}

impl OtpResponseHeader {
    pub const SIZE: usize = core::mem::size_of::<Self>();

    pub fn success(payload_len: u16) -> Self {
        Self {
            status: 0,
            flags: 0,
            payload_len: payload_len.to_le(),
        }
    }

    pub fn error(error: OtpWireError) -> Self {
        Self {
            status: error as u8,
            flags: 0,
            payload_len: 0,
        }
    }

    pub fn is_success(&self) -> bool {
        self.status == 0
    }

    pub fn error_code(&self) -> Option<OtpWireError> {
        if self.is_success() {
            None
        } else {
            Some(OtpWireError::from(self.status))
        }
    }

    pub fn payload_length(&self) -> usize {
        u16::from_le(self.payload_len) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::{FromBytes, IntoBytes};

    #[test]
    fn request_header_roundtrips() {
        let h = OtpRequestHeader::new(OtpOp::CommitSvnFloor, REGION_SVN, 0x0002, 4, 0);
        let bytes = h.as_bytes();
        assert_eq!(bytes.len(), OtpRequestHeader::SIZE);
        let decoded = OtpRequestHeader::ref_from_bytes(bytes).unwrap();
        assert_eq!(decoded.op(), Ok(OtpOp::CommitSvnFloor));
        assert_eq!(decoded.field(), FieldId(0x0002));
        assert_eq!(decoded.len(), 4);
    }
}
