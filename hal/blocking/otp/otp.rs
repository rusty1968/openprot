// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Composable OTP interface, built from synchronous capability traits.

#![cfg_attr(not(test), no_std)]

pub use hal_otp_driver::{
    Error, ErrorKind, ErrorType, OtpBitIndex, OtpBitProgram, OtpBitProgramBatch,
    OtpBitRegionLayout, OtpBitRead, OtpBitStatus, OtpBitStatusAccess, OtpDwordProgram,
    OtpDwordRead, OtpLock, OtpOffset, OtpProgram, OtpProgramBytes, OtpRead, OtpReadBytes,
    OtpRegion, OtpRegionLayout, OtpRegionStatus, OtpRegionStatusAccess, OtpWordProgram,
    OtpWordRead,
};

/// Error returned by the [`OtpPolicy`] wrapper.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum OtpError<E> {
    /// A policy check failed before hardware access was attempted.
    Policy(ErrorKind),
    /// The underlying device reported an error.
    Device(E),
}

/// Policy-enforcing wrapper around an OTP device.
///
/// Unlike a `BlockingFlash`-style adapter, this does no protocol or
/// concurrency bridging — the device it wraps is already fully synchronous.
/// It only validates a request (region status, offset/width match, bounds)
/// before passing it through to the device unchanged; a rejected request
/// never reaches hardware.
pub struct OtpPolicy<D> {
    device: D,
}

impl<D> OtpPolicy<D> {
    /// Wrap a device.
    pub const fn new(device: D) -> Self {
        Self { device }
    }

    /// Borrow the underlying device.
    pub fn device(&self) -> &D {
        &self.device
    }

    /// Recover the underlying device.
    pub fn into_inner(self) -> D {
        self.device
    }

    /// Read after checking that the region is readable, the offset is aligned,
    /// and the access stays within the region capacity.
    pub fn read_checked<T, R>(
        &self,
        region: R,
        offset: OtpOffset,
    ) -> Result<T, OtpError<D::Error>>
    where
        T: Copy,
        R: OtpRegion,
        D: OtpRead<T, Region = R>
            + OtpRegionLayout<Region = R>
            + OtpRegionStatusAccess<Region = R>,
    {
        match self.device.region_status(region).map_err(OtpError::Device)? {
            OtpRegionStatus::Readable => {}
            OtpRegionStatus::ReadProtected => {
                return Err(OtpError::Policy(ErrorKind::RegionProtected));
            }
            OtpRegionStatus::Error => return Err(OtpError::Policy(ErrorKind::Hardware)),
        }

        self.check_access::<T, R>(region, offset)?;
        self.device.read(region, offset).map_err(OtpError::Device)
    }

    /// Program after checking hardware status, offset alignment, and bounds.
    ///
    /// `ReadProtected` does not block a write — it's a read-specific status
    /// (per [`OtpRegionStatus`]'s own naming), not a write-lock — but
    /// `Error` does: a device already reporting a hardware fault shouldn't
    /// be written to.
    pub fn write_checked<T, R>(
        &mut self,
        region: R,
        offset: OtpOffset,
        data: T,
    ) -> Result<(), OtpError<D::Error>>
    where
        T: Copy,
        R: OtpRegion,
        D: OtpProgram<T, Region = R>
            + OtpRegionLayout<Region = R>
            + OtpRegionStatusAccess<Region = R>,
    {
        self.check_write_status(region)?;
        self.check_access::<T, R>(region, offset)?;
        self.device
            .write(region, offset, data)
            .map_err(OtpError::Device)
    }

    /// Read bytes after checking that the region is readable and the access
    /// stays within the region capacity.
    ///
    /// Unlike [`Self::read_checked`], there is no fixed-width alignment
    /// check here: [`OtpReadBytes`] exists precisely so the device can
    /// handle whatever native access width a region requires internally.
    pub fn read_bytes_checked<R>(
        &self,
        region: R,
        offset: OtpOffset,
        buf: &mut [u8],
    ) -> Result<(), OtpError<D::Error>>
    where
        R: OtpRegion,
        D: OtpReadBytes<Region = R>
            + OtpRegionLayout<Region = R>
            + OtpRegionStatusAccess<Region = R>,
    {
        match self.device.region_status(region).map_err(OtpError::Device)? {
            OtpRegionStatus::Readable => {}
            OtpRegionStatus::ReadProtected => {
                return Err(OtpError::Policy(ErrorKind::RegionProtected));
            }
            OtpRegionStatus::Error => return Err(OtpError::Policy(ErrorKind::Hardware)),
        }

        self.check_bounds(region, offset, buf.len())?;
        self.device
            .read_bytes(region, offset, buf)
            .map_err(OtpError::Device)
    }

    /// Program bytes after checking hardware status and bounds. See
    /// [`Self::read_bytes_checked`] for why there's no fixed-width alignment
    /// check here, and [`Self::write_checked`] for why `ReadProtected`
    /// doesn't block a write while `Error` does.
    pub fn program_bytes_checked<R>(
        &mut self,
        region: R,
        offset: OtpOffset,
        data: &[u8],
    ) -> Result<(), OtpError<D::Error>>
    where
        R: OtpRegion,
        D: OtpProgramBytes<Region = R>
            + OtpRegionLayout<Region = R>
            + OtpRegionStatusAccess<Region = R>,
    {
        self.check_write_status(region)?;
        self.check_bounds(region, offset, data.len())?;
        self.device
            .program_bytes(region, offset, data)
            .map_err(OtpError::Device)
    }

    fn check_write_status<R>(&self, region: R) -> Result<(), OtpError<D::Error>>
    where
        R: OtpRegion,
        D: OtpRegionStatusAccess<Region = R>,
    {
        match self.device.region_status(region).map_err(OtpError::Device)? {
            OtpRegionStatus::Error => Err(OtpError::Policy(ErrorKind::Hardware)),
            OtpRegionStatus::Readable | OtpRegionStatus::ReadProtected => Ok(()),
        }
    }

    fn check_access<T, R>(&self, region: R, offset: OtpOffset) -> Result<(), OtpError<D::Error>>
    where
        T: Copy,
        R: OtpRegion,
        D: OtpRegionLayout<Region = R>,
    {
        let alignment = self.device.read_alignment(region);
        if alignment == 0 || offset.bytes() % alignment != 0 {
            return Err(OtpError::Policy(ErrorKind::AlignmentError));
        }
        if core::mem::size_of::<T>() != alignment {
            return Err(OtpError::Policy(ErrorKind::AlignmentError));
        }
        self.check_bounds(region, offset, core::mem::size_of::<T>())
    }

    fn check_bounds<R>(
        &self,
        region: R,
        offset: OtpOffset,
        len: usize,
    ) -> Result<(), OtpError<D::Error>>
    where
        R: OtpRegion,
        D: OtpRegionLayout<Region = R>,
    {
        let end = offset
            .bytes()
            .checked_add(len)
            .ok_or(OtpError::Policy(ErrorKind::InvalidAddress))?;
        if end > self.device.region_capacity(region) {
            return Err(OtpError::Policy(ErrorKind::InvalidAddress));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    struct MockError;

    impl Error for MockError {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Hardware
        }
    }

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    struct MockRegion(u8);

    impl OtpRegion for MockRegion {}

    struct MockOtp;

    impl ErrorType for MockOtp {
        type Error = MockError;
    }

    impl OtpRead<u32> for MockOtp {
        type Region = MockRegion;

        fn read(&self, region: Self::Region, offset: OtpOffset) -> Result<u32, Self::Error> {
            if region.0 == 0 && offset.bytes() == 0 {
                Ok(0x1234_5678)
            } else {
                Err(MockError)
            }
        }
    }

    impl OtpWordRead for MockOtp {}

    impl OtpReadBytes for MockOtp {
        type Region = MockRegion;

        fn read_bytes(
            &self,
            region: Self::Region,
            offset: OtpOffset,
            buf: &mut [u8],
        ) -> Result<(), Self::Error> {
            if region.0 == 0 && offset.bytes() == 0 && buf.len() <= 4 {
                let bytes = 0x1234_5678u32.to_le_bytes();
                buf.copy_from_slice(&bytes[..buf.len()]);
                Ok(())
            } else {
                Err(MockError)
            }
        }
    }

    impl OtpLock<MockRegion> for MockOtp {
        fn lock_region(&mut self, _region: MockRegion) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl OtpProgram<u32> for MockOtp {
        fn write(
            &mut self,
            _region: Self::Region,
            _offset: OtpOffset,
            _data: u32,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl OtpProgramBytes for MockOtp {
        fn program_bytes(
            &mut self,
            _region: Self::Region,
            _offset: OtpOffset,
            _data: &[u8],
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl OtpRegionLayout for MockOtp {
        type Region = MockRegion;

        fn region_capacity(&self, _region: Self::Region) -> usize {
            4
        }

        fn read_alignment(&self, _region: Self::Region) -> usize {
            4
        }
    }

    impl OtpRegionStatusAccess for MockOtp {
        type Region = MockRegion;

        fn region_status(&self, region: Self::Region) -> Result<OtpRegionStatus, Self::Error> {
            Ok(match region.0 {
                0 => OtpRegionStatus::Readable,
                1 => OtpRegionStatus::ReadProtected,
                _ => OtpRegionStatus::Error,
            })
        }
    }

    #[test]
    fn caliptra_ss_read_capabilities_compose() {
        let device = MockOtp;
        let region = MockRegion(0);

        assert_eq!(device.read(region, OtpOffset::new(0)).unwrap(), 0x1234_5678);
        assert_eq!(device.region_capacity(region), 4);
        assert_eq!(device.read_alignment(region), 4);
        assert_eq!(device.region_status(region).unwrap(), OtpRegionStatus::Readable);
    }

    #[test]
    fn read_checked_reads_readable_aligned_in_bounds() {
        let otp = OtpPolicy::new(MockOtp);
        let value: u32 = otp.read_checked(MockRegion(0), OtpOffset::new(0)).unwrap();
        assert_eq!(value, 0x1234_5678);
    }

    #[test]
    fn read_checked_rejects_misaligned_offset() {
        let otp = OtpPolicy::new(MockOtp);
        assert_eq!(
            otp.read_checked::<u32, _>(MockRegion(0), OtpOffset::new(1)),
            Err(OtpError::Policy(ErrorKind::AlignmentError)),
        );
    }

    #[test]
    fn read_checked_rejects_out_of_bounds() {
        let otp = OtpPolicy::new(MockOtp);
        assert_eq!(
            otp.read_checked::<u32, _>(MockRegion(0), OtpOffset::new(4)),
            Err(OtpError::Policy(ErrorKind::InvalidAddress)),
        );
    }

    #[test]
    fn read_checked_rejects_protected_region() {
        let otp = OtpPolicy::new(MockOtp);
        assert_eq!(
            otp.read_checked::<u32, _>(MockRegion(1), OtpOffset::new(0)),
            Err(OtpError::Policy(ErrorKind::RegionProtected)),
        );
    }

    #[test]
    fn read_bytes_fills_buffer() {
        let device = MockOtp;
        let mut buf = [0u8; 4];
        device
            .read_bytes(MockRegion(0), OtpOffset::new(0), &mut buf)
            .unwrap();
        assert_eq!(u32::from_le_bytes(buf), 0x1234_5678);
    }

    #[test]
    fn read_bytes_checked_fills_buffer_when_readable() {
        let otp = OtpPolicy::new(MockOtp);
        let mut buf = [0u8; 4];
        otp.read_bytes_checked(MockRegion(0), OtpOffset::new(0), &mut buf)
            .unwrap();
        assert_eq!(u32::from_le_bytes(buf), 0x1234_5678);
    }

    #[test]
    fn read_bytes_checked_rejects_protected_region() {
        let otp = OtpPolicy::new(MockOtp);
        let mut buf = [0u8; 4];
        assert_eq!(
            otp.read_bytes_checked(MockRegion(1), OtpOffset::new(0), &mut buf),
            Err(OtpError::Policy(ErrorKind::RegionProtected)),
        );
    }

    #[test]
    fn read_bytes_checked_rejects_out_of_bounds() {
        let otp = OtpPolicy::new(MockOtp);
        let mut buf = [0u8; 4];
        assert_eq!(
            otp.read_bytes_checked(MockRegion(0), OtpOffset::new(1), &mut buf),
            Err(OtpError::Policy(ErrorKind::InvalidAddress)),
        );
    }

    #[test]
    fn program_bytes_checked_rejects_out_of_bounds() {
        let mut otp = OtpPolicy::new(MockOtp);
        let data = [0u8; 4];
        assert_eq!(
            otp.program_bytes_checked(MockRegion(0), OtpOffset::new(1), &data),
            Err(OtpError::Policy(ErrorKind::InvalidAddress)),
        );
    }

    #[test]
    fn write_checked_writes_when_readable() {
        let mut otp = OtpPolicy::new(MockOtp);
        assert_eq!(
            otp.write_checked(MockRegion(0), OtpOffset::new(0), 0x1234_5678u32),
            Ok(()),
        );
    }

    #[test]
    fn write_checked_allows_read_protected_region() {
        // ReadProtected is a read-specific status; it must not block a write.
        let mut otp = OtpPolicy::new(MockOtp);
        assert_eq!(
            otp.write_checked(MockRegion(1), OtpOffset::new(0), 0x1234_5678u32),
            Ok(()),
        );
    }

    #[test]
    fn write_checked_rejects_hardware_error_region() {
        let mut otp = OtpPolicy::new(MockOtp);
        assert_eq!(
            otp.write_checked(MockRegion(2), OtpOffset::new(0), 0x1234_5678u32),
            Err(OtpError::Policy(ErrorKind::Hardware)),
        );
    }

    #[test]
    fn program_bytes_checked_allows_read_protected_region() {
        let mut otp = OtpPolicy::new(MockOtp);
        let data = [0u8; 4];
        assert_eq!(
            otp.program_bytes_checked(MockRegion(1), OtpOffset::new(0), &data),
            Ok(()),
        );
    }

    #[test]
    fn program_bytes_checked_rejects_hardware_error_region() {
        let mut otp = OtpPolicy::new(MockOtp);
        let data = [0u8; 4];
        assert_eq!(
            otp.program_bytes_checked(MockRegion(2), OtpOffset::new(0), &data),
            Err(OtpError::Policy(ErrorKind::Hardware)),
        );
    }

    /// A device whose region requires 64-bit ("secret partition") DAI access.
    struct MockDwordOtp;

    impl ErrorType for MockDwordOtp {
        type Error = MockError;
    }

    impl OtpRead<u64> for MockDwordOtp {
        type Region = MockRegion;

        fn read(&self, region: Self::Region, offset: OtpOffset) -> Result<u64, Self::Error> {
            if region.0 == 0 && offset.bytes() == 0 {
                Ok(0x1122_3344_5566_7788)
            } else {
                Err(MockError)
            }
        }
    }

    impl OtpDwordRead for MockDwordOtp {}

    // Real controllers (e.g. `OtpController`) implement `OtpRead<u32>`
    // unconditionally for the whole device, since Rust impls aren't scoped
    // per-region — only `read_alignment(region)` distinguishes a 32-bit
    // region from a 64-bit one at runtime. Mirror that here so the
    // width-mismatch test below reflects how misuse is actually possible.
    impl OtpRead<u32> for MockDwordOtp {
        type Region = MockRegion;

        fn read(&self, _region: Self::Region, _offset: OtpOffset) -> Result<u32, Self::Error> {
            unreachable!("check_access must reject this before the device is called")
        }
    }

    impl OtpRegionLayout for MockDwordOtp {
        type Region = MockRegion;

        fn region_capacity(&self, _region: Self::Region) -> usize {
            8
        }

        fn read_alignment(&self, _region: Self::Region) -> usize {
            8
        }
    }

    impl OtpRegionStatusAccess for MockDwordOtp {
        type Region = MockRegion;

        fn region_status(&self, _region: Self::Region) -> Result<OtpRegionStatus, Self::Error> {
            Ok(OtpRegionStatus::Readable)
        }
    }

    #[test]
    fn dword_read_capabilities_compose() {
        let device = MockDwordOtp;
        let region = MockRegion(0);

        let value: u64 = device.read(region, OtpOffset::new(0)).unwrap();
        assert_eq!(value, 0x1122_3344_5566_7788);
        assert_eq!(device.read_alignment(region), 8);
    }

    #[test]
    fn read_checked_reads_dword_region() {
        let otp = OtpPolicy::new(MockDwordOtp);
        let value: u64 = otp.read_checked(MockRegion(0), OtpOffset::new(0)).unwrap();
        assert_eq!(value, 0x1122_3344_5566_7788);
    }

    #[test]
    fn read_checked_rejects_width_mismatch_against_dword_region() {
        let otp = OtpPolicy::new(MockDwordOtp);
        // A 32-bit read against an 8-byte-granularity region is aligned
        // (0 % 8 == 0) but must still be rejected: `u32` doesn't match the
        // region's native 64-bit access width.
        assert_eq!(
            otp.read_checked::<u32, _>(MockRegion(0), OtpOffset::new(0)),
            Err(OtpError::Policy(ErrorKind::AlignmentError)),
        );
    }

    /// A bit-addressed device (e.g. a strap region), each bit backed by a
    /// small number of redundant physical fuses.
    const MOCK_STRAP_BITS: usize = 8;
    const MOCK_STRAP_ATTEMPTS: u8 = 3;

    struct MockStrapOtp {
        values: [bool; MOCK_STRAP_BITS],
        remaining_attempts: [u8; MOCK_STRAP_BITS],
        protected: [bool; MOCK_STRAP_BITS],
    }

    impl MockStrapOtp {
        fn new() -> Self {
            Self {
                values: [false; MOCK_STRAP_BITS],
                remaining_attempts: [MOCK_STRAP_ATTEMPTS; MOCK_STRAP_BITS],
                protected: [false; MOCK_STRAP_BITS],
            }
        }
    }

    /// Mock error carrying a real `kind`, unlike `MockError` — needed here
    /// because the bit family distinguishes `RegionProtected` from
    /// `AttemptsExhausted`, which a single hardcoded kind can't represent.
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    struct MockStrapError(ErrorKind);

    impl Error for MockStrapError {
        fn kind(&self) -> ErrorKind {
            self.0
        }
    }

    impl ErrorType for MockStrapOtp {
        type Error = MockStrapError;
    }

    impl OtpBitRead for MockStrapOtp {
        type Region = MockRegion;

        fn read_bit(&self, _region: Self::Region, bit: OtpBitIndex) -> Result<bool, Self::Error> {
            self.values
                .get(bit.bit())
                .copied()
                .ok_or(MockStrapError(ErrorKind::InvalidAddress))
        }
    }

    impl OtpBitStatusAccess for MockStrapOtp {
        type Region = MockRegion;

        fn bit_status(
            &self,
            _region: Self::Region,
            bit: OtpBitIndex,
        ) -> Result<OtpBitStatus, Self::Error> {
            let i = bit.bit();
            if i >= MOCK_STRAP_BITS {
                return Err(MockStrapError(ErrorKind::InvalidAddress));
            }
            Ok(OtpBitStatus {
                value: self.values[i],
                protected: self.protected[i],
                remaining_attempts: self.remaining_attempts[i],
            })
        }
    }

    impl OtpBitRegionLayout for MockStrapOtp {
        type Region = MockRegion;

        fn region_bit_capacity(&self, _region: Self::Region) -> usize {
            MOCK_STRAP_BITS
        }
    }

    impl OtpLock<MockRegion> for MockStrapOtp {
        fn lock_region(&mut self, _region: MockRegion) -> Result<(), Self::Error> {
            self.protected = [true; MOCK_STRAP_BITS];
            Ok(())
        }
    }

    impl OtpBitProgram for MockStrapOtp {
        fn write_bit(
            &mut self,
            _region: Self::Region,
            bit: OtpBitIndex,
            value: bool,
        ) -> Result<(), Self::Error> {
            let i = bit.bit();
            if i >= MOCK_STRAP_BITS {
                return Err(MockStrapError(ErrorKind::InvalidAddress));
            }
            // Protection is checked before the no-op short-circuit: a
            // locked bit refuses the write command itself, regardless of
            // whether the requested value already matches.
            if self.protected[i] {
                return Err(MockStrapError(ErrorKind::RegionProtected));
            }
            if self.values[i] == value {
                return Ok(());
            }
            if self.remaining_attempts[i] == 0 {
                return Err(MockStrapError(ErrorKind::AttemptsExhausted));
            }
            self.values[i] = value;
            self.remaining_attempts[i] -= 1;
            Ok(())
        }
    }

    impl OtpBitProgramBatch for MockStrapOtp {
        fn program_bits(
            &mut self,
            _region: Self::Region,
            start: OtpBitIndex,
            bits: &[bool],
        ) -> Result<(), Self::Error> {
            // Validate the whole batch before touching anything, so a
            // rejected batch never partially burns bits. Protection is
            // checked for every bit in range, even ones that are already at
            // their desired value; attempt-exhaustion only matters for bits
            // that actually need to change.
            for (offset, &desired) in bits.iter().enumerate() {
                let i = start.bit() + offset;
                if i >= MOCK_STRAP_BITS {
                    return Err(MockStrapError(ErrorKind::InvalidAddress));
                }
                if self.protected[i] {
                    return Err(MockStrapError(ErrorKind::RegionProtected));
                }
                if self.values[i] != desired && self.remaining_attempts[i] == 0 {
                    return Err(MockStrapError(ErrorKind::AttemptsExhausted));
                }
            }
            for (offset, &desired) in bits.iter().enumerate() {
                let i = start.bit() + offset;
                if self.values[i] != desired {
                    self.values[i] = desired;
                    self.remaining_attempts[i] -= 1;
                }
            }
            Ok(())
        }
    }

    #[test]
    fn bit_read_and_status_compose() {
        let device = MockStrapOtp::new();
        let region = MockRegion(0);

        assert!(!device.read_bit(region, OtpBitIndex::new(0)).unwrap());
        assert_eq!(
            device.bit_status(region, OtpBitIndex::new(0)).unwrap(),
            OtpBitStatus {
                value: false,
                protected: false,
                remaining_attempts: MOCK_STRAP_ATTEMPTS,
            }
        );
    }

    #[test]
    fn write_bit_updates_value_and_decrements_remaining_attempts() {
        let mut device = MockStrapOtp::new();
        let region = MockRegion(0);

        device.write_bit(region, OtpBitIndex::new(2), true).unwrap();
        assert!(device.read_bit(region, OtpBitIndex::new(2)).unwrap());
        assert_eq!(
            device
                .bit_status(region, OtpBitIndex::new(2))
                .unwrap()
                .remaining_attempts,
            MOCK_STRAP_ATTEMPTS - 1
        );
    }

    #[test]
    fn write_bit_rejects_when_attempts_exhausted() {
        let mut device = MockStrapOtp::new();
        let region = MockRegion(0);
        let bit = OtpBitIndex::new(0);

        for _ in 0..MOCK_STRAP_ATTEMPTS {
            let current = device.read_bit(region, bit).unwrap();
            device.write_bit(region, bit, !current).unwrap();
        }
        assert_eq!(
            device.bit_status(region, bit).unwrap().remaining_attempts,
            0
        );

        let current = device.read_bit(region, bit).unwrap();
        assert_eq!(
            device.write_bit(region, bit, !current).unwrap_err().kind(),
            ErrorKind::AttemptsExhausted,
        );
    }

    #[test]
    fn write_bit_rejects_protected_bit_distinctly_from_exhausted() {
        let mut device = MockStrapOtp::new();
        let region = MockRegion(0);
        let bit = OtpBitIndex::new(0);

        device.protected[0] = true;
        assert_eq!(
            device.write_bit(region, bit, true).unwrap_err().kind(),
            ErrorKind::RegionProtected,
        );
    }

    #[test]
    fn write_bit_rejects_protected_bit_even_as_a_noop() {
        let mut device = MockStrapOtp::new();
        let region = MockRegion(0);
        let bit = OtpBitIndex::new(0);

        // The bit's current value (false) already matches the requested
        // value — a naive no-op short-circuit would let this through
        // without checking protection at all.
        device.protected[0] = true;
        assert_eq!(
            device.write_bit(region, bit, false).unwrap_err().kind(),
            ErrorKind::RegionProtected,
        );
    }

    #[test]
    fn program_bits_batch_rejects_protected_bit_even_as_a_noop() {
        let mut device = MockStrapOtp::new();
        let region = MockRegion(0);
        device.protected[3] = true;

        // Bit 3's requested value (false) already matches its current
        // value, but the batch must still be rejected because it's
        // protected.
        let result = device.program_bits(region, OtpBitIndex::new(2), &[true, false, true]);
        assert!(result.is_err());
        assert!(!device.read_bit(region, OtpBitIndex::new(2)).unwrap());
    }

    #[test]
    fn bit_region_capacity_reports_bit_count() {
        let device = MockStrapOtp::new();
        assert_eq!(
            device.region_bit_capacity(MockRegion(0)),
            MOCK_STRAP_BITS
        );
    }

    #[test]
    fn program_bits_batch_rejects_whole_batch_if_any_bit_invalid() {
        let mut device = MockStrapOtp::new();
        let region = MockRegion(0);
        device.protected[3] = true;

        // Bits 2, 3, 4: bit 3 is protected, so the whole batch must be
        // rejected without programming bit 2.
        let result = device.program_bits(region, OtpBitIndex::new(2), &[true, true, true]);
        assert!(result.is_err());
        assert!(!device.read_bit(region, OtpBitIndex::new(2)).unwrap());
    }
}
