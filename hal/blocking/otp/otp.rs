// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Composable blocking OTP interface.

#![cfg_attr(not(test), no_std)]

pub use hal_otp_driver::{
    Error, ErrorKind, ErrorType, OtpOffset, OtpProgram, OtpProgramBytes, OtpRead, OtpReadBytes,
    OtpRegion, OtpRegionLayout, OtpRegionStatus, OtpRegionStatusAccess, OtpWordProgram, OtpWordRead,
};

/// A read-only OTP device whose capabilities are composed from smaller traits.
///
/// Region status is intentionally not required here: controllers whose
/// readability is governed globally by lifecycle state (rather than per region)
/// can still be read devices. Callers that need status-checked reads use
/// [`BlockingOtp::read_checked`], which requires [`OtpRegionStatusAccess`].
pub trait OtpReadDevice<T>: OtpRead<T> + OtpRegionLayout
where
    T: Copy,
{
}

impl<T, D> OtpReadDevice<T> for D
where
    T: Copy,
    D: OtpRead<T> + OtpRegionLayout,
{
}

/// An OTP device with the optional provisioning programming capability.
pub trait OtpProvisioningDevice<T>: OtpReadDevice<T> + OtpProgram<T>
where
    T: Copy,
{
}

impl<T, D> OtpProvisioningDevice<T> for D
where
    T: Copy,
    D: OtpReadDevice<T> + OtpProgram<T>,
{
}

/// Error returned by the checked blocking OTP wrapper.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum OtpError<E> {
    /// A policy check failed before hardware access was attempted.
    Policy(ErrorKind),
    /// The underlying device reported an error.
    Device(E),
}

/// Blocking OTP wrapper that enforces region status, offset alignment, and
/// bounds before delegating to the device.
pub struct BlockingOtp<D> {
    device: D,
}

impl<D> BlockingOtp<D> {
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

    /// Program after checking offset alignment and bounds.
    pub fn write_checked<T, R>(
        &mut self,
        region: R,
        offset: OtpOffset,
        data: T,
    ) -> Result<(), OtpError<D::Error>>
    where
        T: Copy,
        R: OtpRegion,
        D: OtpProgram<T, Region = R> + OtpRegionLayout<Region = R>,
    {
        self.check_access::<T, R>(region, offset)?;
        self.device
            .write(region, offset, data)
            .map_err(OtpError::Device)
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
        let end = offset
            .bytes()
            .checked_add(core::mem::size_of::<T>())
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
        let otp = BlockingOtp::new(MockOtp);
        let value: u32 = otp.read_checked(MockRegion(0), OtpOffset::new(0)).unwrap();
        assert_eq!(value, 0x1234_5678);
    }

    #[test]
    fn read_checked_rejects_misaligned_offset() {
        let otp = BlockingOtp::new(MockOtp);
        assert_eq!(
            otp.read_checked::<u32, _>(MockRegion(0), OtpOffset::new(1)),
            Err(OtpError::Policy(ErrorKind::AlignmentError)),
        );
    }

    #[test]
    fn read_checked_rejects_out_of_bounds() {
        let otp = BlockingOtp::new(MockOtp);
        assert_eq!(
            otp.read_checked::<u32, _>(MockRegion(0), OtpOffset::new(4)),
            Err(OtpError::Policy(ErrorKind::InvalidAddress)),
        );
    }

    #[test]
    fn read_checked_rejects_protected_region() {
        let otp = BlockingOtp::new(MockOtp);
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
}
