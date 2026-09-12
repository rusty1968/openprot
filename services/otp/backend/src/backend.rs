// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Region-keyed byte adapter over the Caliptra SS OTP DAI driver.

use caliptra_ss_registers::fuses;
use hal_otp_driver::{ErrorKind, ErrorType, OtpOffset, OtpProgramBytes, OtpReadBytes};
use otp_api::{RegionId, REGION_SVN, REGION_VENDOR_HASHES_MANUF};
use veer_peripherals::otp::{OtpController, OtpError, Partition};

/// Adapts [`OtpController`] to the wire-stable [`RegionId`] address space the
/// OTP service dispatch requires, resolving each region to its Caliptra fuse
/// partition and delegating byte transfers to the DAI driver.
pub struct CaliptraOtpBackend {
    ctrl: OtpController,
}

impl CaliptraOtpBackend {
    /// Wrap an OTP controller as the service backend.
    pub const fn new(ctrl: OtpController) -> Self {
        Self { ctrl }
    }

    /// Resolve a wire region id to its Caliptra fuse partition, or `None` for a
    /// region this SoC does not expose to the service.
    pub fn partition(region: RegionId) -> Option<Partition> {
        let (base, size) = match region.0 {
            REGION_SVN => (
                fuses::SVN_PARTITION_BYTE_OFFSET,
                fuses::SVN_PARTITION_BYTE_SIZE,
            ),
            REGION_VENDOR_HASHES_MANUF => (
                fuses::VENDOR_HASHES_MANUF_PARTITION_BYTE_OFFSET,
                fuses::VENDOR_HASHES_MANUF_PARTITION_BYTE_SIZE,
            ),
            _ => return None,
        };
        Some(Partition::new(base, size))
    }
}

impl ErrorType for CaliptraOtpBackend {
    type Error = OtpError;
}

impl OtpReadBytes for CaliptraOtpBackend {
    type Region = RegionId;

    fn read_bytes(
        &self,
        region: RegionId,
        offset: OtpOffset,
        buf: &mut [u8],
    ) -> Result<(), Self::Error> {
        let part = Self::partition(region).ok_or(OtpError::new(ErrorKind::InvalidAddress))?;
        self.ctrl.read_bytes(part, offset, buf)
    }
}

impl OtpProgramBytes for CaliptraOtpBackend {
    fn program_bytes(
        &mut self,
        region: RegionId,
        offset: OtpOffset,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        let part = Self::partition(region).ok_or(OtpError::new(ErrorKind::InvalidAddress))?;
        self.ctrl.program_bytes(part, offset, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svn_region_maps_to_svn_partition() {
        let p = CaliptraOtpBackend::partition(RegionId(REGION_SVN)).unwrap();
        assert_eq!(p.base(), fuses::SVN_PARTITION_BYTE_OFFSET);
        assert_eq!(p.size(), fuses::SVN_PARTITION_BYTE_SIZE);
    }

    #[test]
    fn vendor_hashes_region_maps_to_partition() {
        let p = CaliptraOtpBackend::partition(RegionId(REGION_VENDOR_HASHES_MANUF)).unwrap();
        assert_eq!(p.base(), fuses::VENDOR_HASHES_MANUF_PARTITION_BYTE_OFFSET);
        assert_eq!(p.size(), fuses::VENDOR_HASHES_MANUF_PARTITION_BYTE_SIZE);
    }

    #[test]
    fn unknown_region_has_no_partition() {
        assert!(CaliptraOtpBackend::partition(RegionId(0xFFFF)).is_none());
    }
}
