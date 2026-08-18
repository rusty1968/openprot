// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Low-level OTP capability traits.

#![no_std]

/// Represents the category of an OTP operation error.
#[non_exhaustive]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ErrorKind {
    /// The address is outside the selected region.
    InvalidAddress,
    /// The address or operation is not aligned for the hardware.
    AlignmentError,
    /// The requested region is protected from access.
    RegionProtected,
    /// The OTP controller reported an integrity or access error.
    Hardware,
    /// The operation timed out.
    Timeout,
    /// The operation is not supported by this device.
    Unsupported,
}

/// Error contract shared by all OTP capabilities.
pub trait Error: core::fmt::Debug {
    /// Classify the hardware-specific error.
    fn kind(&self) -> ErrorKind;
}

/// Associates an error type with an OTP capability.
pub trait ErrorType {
    /// Hardware-specific error type.
    type Error: Error;
}

/// A byte offset relative to an OTP region or address-space base.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct OtpOffset(usize);

impl OtpOffset {
    /// Construct an offset measured in bytes.
    pub const fn new(bytes: usize) -> Self {
        Self(bytes)
    }

    /// Return the offset in bytes.
    pub const fn bytes(self) -> usize {
        self.0
    }
}

/// Identifier for a logical OTP or fuse region.
pub trait OtpRegion: Copy + core::fmt::Debug + PartialEq {}

/// Read-only access to an OTP region.
pub trait OtpRead<T>: ErrorType
where
    T: Copy,
{
    /// Region identifier type.
    type Region: OtpRegion;

    /// Read one value at a byte offset within a region.
    fn read(&self, region: Self::Region, offset: OtpOffset) -> Result<T, Self::Error>;
}

/// Fixed-width 32-bit read refinement for register/window interfaces.
pub trait OtpWordRead: OtpRead<u32> {}

/// Bulk byte-oriented read access.
///
/// The natural primitive for partition- or window-based controllers that
/// transfer multiple words per operation. Independent of [`OtpRead`]: a device
/// may implement either or both.
pub trait OtpReadBytes: ErrorType {
    /// Region identifier type.
    type Region: OtpRegion;

    /// Read bytes starting at a byte offset within a region into `buf`.
    fn read_bytes(
        &self,
        region: Self::Region,
        offset: OtpOffset,
        buf: &mut [u8],
    ) -> Result<(), Self::Error>;
}

/// Region geometry exposed by an OTP controller.
pub trait OtpRegionLayout: ErrorType {
    /// Region identifier type.
    type Region: OtpRegion;

    /// Return the region capacity in bytes.
    fn region_capacity(&self, region: Self::Region) -> usize;

    /// Return the required alignment in bytes for reads.
    fn read_alignment(&self, region: Self::Region) -> usize;
}

/// Hardware access state for an OTP region.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum OtpRegionStatus {
    /// Reads are allowed in the current lifecycle state.
    Readable,
    /// The region exists but reads are blocked.
    ReadProtected,
    /// The controller reported an integrity or access error.
    Error,
}

/// Region protection/status capability.
pub trait OtpRegionStatusAccess: ErrorType {
    /// Region identifier type.
    type Region: OtpRegion;

    /// Read the hardware status of a region.
    fn region_status(&self, region: Self::Region) -> Result<OtpRegionStatus, Self::Error>;
}

/// Optional programming capability for provisioning or test firmware.
pub trait OtpProgram<T>: OtpRead<T>
where
    T: Copy,
{
    /// Program one value at a byte offset within a region.
    fn write(&mut self, region: Self::Region, offset: OtpOffset, data: T)
    -> Result<(), Self::Error>;

    /// Permanently lock programming for a region.
    fn lock_region(&mut self, region: Self::Region) -> Result<(), Self::Error>;
}

/// Fixed-width 32-bit programming refinement.
pub trait OtpWordProgram: OtpProgram<u32> + OtpWordRead {}

/// Bulk byte-oriented programming capability.
pub trait OtpProgramBytes: OtpReadBytes {
    /// Program `data` starting at a byte offset within a region.
    fn program_bytes(
        &mut self,
        region: Self::Region,
        offset: OtpOffset,
        data: &[u8],
    ) -> Result<(), Self::Error>;
}
