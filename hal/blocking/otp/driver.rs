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
    /// The address/bit is programmable but has no programming attempts
    /// left (e.g. all redundant backing fuses for this bit are consumed).
    /// Distinct from `RegionProtected`: this location was allowed, and used
    /// up, rather than never allowed.
    AttemptsExhausted,
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
    ///
    /// For OTP this is also the region's exact native access width (there is
    /// no partial/sub-word transfer concept as there is for e.g. flash): a
    /// typed access must have `size_of::<T>()` equal to this value, not just
    /// an offset that happens to be a multiple of it.
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

/// Region-locking capability.
///
/// Split out from [`OtpProgram`] because locking doesn't depend on the
/// fixed-width type used to program a region: a device that supports both
/// [`OtpProgram<u32>`] and [`OtpProgram<u64>`] needs only one `OtpLock` impl,
/// not one per width.
pub trait OtpLock<R: OtpRegion>: ErrorType {
    /// Permanently lock programming for a region.
    fn lock_region(&mut self, region: R) -> Result<(), Self::Error>;
}

/// Optional programming capability for provisioning or test firmware.
pub trait OtpProgram<T>: OtpRead<T> + OtpLock<<Self as OtpRead<T>>::Region>
where
    T: Copy,
{
    /// Program one value at a byte offset within a region.
    fn write(&mut self, region: Self::Region, offset: OtpOffset, data: T)
    -> Result<(), Self::Error>;
}

/// Fixed-width 32-bit programming refinement.
pub trait OtpWordProgram: OtpProgram<u32> + OtpWordRead {}

/// Fixed-width 64-bit read refinement for controllers whose DAI transfers a
/// full dword (two 32-bit registers) per operation, as required by some OTP
/// macro partitions (e.g. secret/lifecycle partitions).
pub trait OtpDwordRead: OtpRead<u64> {}

/// Fixed-width 64-bit programming refinement.
pub trait OtpDwordProgram: OtpProgram<u64> + OtpDwordRead {}

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

/// A bit position within a bit-addressed OTP region (e.g. straps).
///
/// Deliberately not [`OtpOffset`] — that type means "byte offset"; reusing
/// it here would conflate two different units the way some hardware's own
/// API already does between a byte-offset-like region and a word-index
/// region.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct OtpBitIndex(usize);

impl OtpBitIndex {
    /// Construct a bit position.
    pub const fn new(bit: usize) -> Self {
        Self(bit)
    }

    /// Return the bit position.
    pub const fn bit(self) -> usize {
        self.0
    }
}

/// Status of a single bit in a bit-addressed OTP region.
///
/// Bundles value/protection/remaining-attempts together (rather than
/// splitting into separate traits the way [`OtpRead`]/[`OtpRegionStatusAccess`]
/// are split) because a caller deciding whether to retry a failed bit write
/// needs all three at once — they aren't independently useful the way
/// region capacity and region status are.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct OtpBitStatus {
    /// Current programmed value.
    pub value: bool,
    /// Whether this bit is protected from further programming.
    pub protected: bool,
    /// Remaining programming attempts backed by redundant physical fuses;
    /// `0` once exhausted. Devices with no redundancy (a single physical
    /// fuse per logical bit) report `1` while unprogrammed, `0` after.
    pub remaining_attempts: u8,
}

/// Bit-count geometry for a bit-addressed OTP region.
///
/// Separate from [`OtpRegionLayout`] (which reports byte capacity) rather
/// than overloading one method to mean two different units depending on
/// which region family it's called on.
pub trait OtpBitRegionLayout: ErrorType {
    /// Region identifier type.
    type Region: OtpRegion;

    /// Return the number of addressable bits in this region.
    fn region_bit_capacity(&self, region: Self::Region) -> usize;
}

/// Read-only access to individual bits within a bit-addressed OTP region.
pub trait OtpBitRead: ErrorType {
    /// Region identifier type.
    type Region: OtpRegion;

    /// Read the value of one bit.
    fn read_bit(&self, region: Self::Region, bit: OtpBitIndex) -> Result<bool, Self::Error>;
}

/// Per-bit status capability: value, protection, and remaining programming
/// attempts.
pub trait OtpBitStatusAccess: ErrorType {
    /// Region identifier type.
    type Region: OtpRegion;

    /// Read the status of one bit.
    fn bit_status(
        &self,
        region: Self::Region,
        bit: OtpBitIndex,
    ) -> Result<OtpBitStatus, Self::Error>;
}

/// Single-bit programming capability, backed by per-bit redundant physical
/// fuses.
///
/// A protected bit must reject `write_bit` outright, even when `value`
/// already matches the bit's current value — protection means the hardware
/// refuses the programming command itself, not just that an actual change
/// would occur. If the bit is unprotected and already holds `value`, an
/// implementation may treat the call as a no-op that does not consume a
/// programming attempt.
///
/// Reuses [`OtpLock`] (already width/family-independent) for whole-region
/// locking rather than a second lock trait.
pub trait OtpBitProgram: OtpBitRead + OtpLock<<Self as OtpBitRead>::Region> {
    /// Program one bit. See the trait documentation for the
    /// protection-vs-no-op interaction.
    fn write_bit(
        &mut self,
        region: Self::Region,
        bit: OtpBitIndex,
        value: bool,
    ) -> Result<(), Self::Error>;
}

/// Bulk bit-programming capability that validates the whole batch (bounds,
/// protection, remaining attempts) before programming any bit in it — an
/// atomicity guarantee a per-bit [`OtpBitProgram::write_bit`] loop can't
/// offer. Independent of [`OtpBitProgram`]: a device may support one, both,
/// or neither, mirroring how [`OtpProgramBytes`] sits alongside
/// [`OtpProgram`].
pub trait OtpBitProgramBatch: OtpBitRead {
    /// Program bits `start.bit() .. start.bit() + bits.len()`, where
    /// `bits[i]` is the desired value of bit `start.bit() + i`. A bit
    /// already at its desired value is left untouched and does not consume
    /// a programming attempt — but, as with [`OtpBitProgram::write_bit`], a
    /// protected bit still causes the whole batch to be rejected even if
    /// its requested value already matches.
    fn program_bits(
        &mut self,
        region: Self::Region,
        start: OtpBitIndex,
        bits: &[bool],
    ) -> Result<(), Self::Error>;
}
