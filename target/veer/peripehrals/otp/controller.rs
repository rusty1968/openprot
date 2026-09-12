// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Direct Access Interface (DAI) driver for the Caliptra Subsystem OTP macro.

use core::marker::PhantomData;

use caliptra_ss_registers::otp_ctrl::bits::{DirectAccessCmd, OtpStatus};
use caliptra_ss_registers::otp_ctrl::regs::OtpCtrl;
use tock_registers::interfaces::{Readable, Writeable};

use hal_otp_driver::{
    ErrorKind, ErrorType, OtpOffset, OtpProgram, OtpProgramBytes, OtpRead, OtpReadBytes,
    OtpRegionLayout, OtpRegionStatus, OtpRegionStatusAccess, OtpWordProgram, OtpWordRead,
};

use super::types::{OtpError, Partition};

/// Status bits `[0, 21]` are error / FSM / bus-integrity flags; bit 22 is
/// `DaiIdle` and bit 23 is `CheckPending`.
const OTP_STATUS_ERROR_MASK: u32 = (1 << 22) - 1;

/// Upper bound on DAI busy-wait iterations before reporting a timeout.
const DAI_SPIN_LIMIT: u32 = 1_000_000;

/// Read access is aligned to 32-bit words.
const WORD_ALIGNMENT: usize = 4;

/// Blocking driver for the OTP controller's Direct Access Interface.
pub struct OtpController {
    base: *const OtpCtrl,
    /// MMIO register blocks must not cross threads or be shared by reference.
    _not_send_sync: PhantomData<*const ()>,
}

impl OtpController {
    /// Create a controller from a raw pointer to the OTP register block.
    ///
    /// # Safety
    /// - `base` must point to a valid, mapped OTP controller register block.
    /// - Access to the controller instance must be serialized by the caller.
    pub const unsafe fn new(base: *const OtpCtrl) -> Self {
        Self {
            base,
            _not_send_sync: PhantomData,
        }
    }

    /// Create a controller from the physical base address of the register block.
    ///
    /// # Safety
    /// See [`OtpController::new`].
    pub const unsafe fn from_addr(addr: usize) -> Self {
        // SAFETY: forwarded to the caller of this `unsafe` constructor.
        unsafe { Self::new(addr as *const OtpCtrl) }
    }

    fn regs(&self) -> &OtpCtrl {
        // SAFETY: `base` validity is a construction-time invariant.
        unsafe { &*self.base }
    }

    fn wait_idle(&self) -> Result<(), OtpError> {
        for _ in 0..DAI_SPIN_LIMIT {
            if self.regs().otp_status.is_set(OtpStatus::DaiIdle) {
                return Ok(());
            }
        }
        Err(OtpError::new(ErrorKind::Timeout))
    }

    fn check_error(&self) -> Result<(), OtpError> {
        if self.regs().otp_status.get() & OTP_STATUS_ERROR_MASK != 0 {
            Err(OtpError::new(ErrorKind::Hardware))
        } else {
            Ok(())
        }
    }

    /// Read a single 32-bit word at a byte address in the OTP address space.
    pub fn read_word(&self, byte_addr: usize) -> Result<u32, OtpError> {
        self.wait_idle()?;
        self.regs().direct_access_address.set(byte_addr as u32);
        self.regs().direct_access_cmd.write(DirectAccessCmd::Rd::SET);
        self.wait_idle()?;
        self.check_error()?;
        Ok(self.regs().dai_rdata_rf_direct_access_rdata_0.get())
    }

    /// Program a single 32-bit word at a byte address in the OTP address space.
    ///
    /// OTP is write-once: bits already programmed to `1` cannot be cleared. The
    /// controller reports such violations through the status error bits, which
    /// this method surfaces as [`ErrorKind::Hardware`].
    pub fn write_word(&self, byte_addr: usize, data: u32) -> Result<(), OtpError> {
        self.wait_idle()?;
        self.regs().dai_wdata_rf_direct_access_wdata_0.set(data);
        self.regs().direct_access_address.set(byte_addr as u32);
        self.regs().direct_access_cmd.write(DirectAccessCmd::Wr::SET);
        self.wait_idle()?;
        self.check_error()
    }

    /// Compute and program a partition's integrity digest, locking further
    /// writes to it. `base_byte_addr` is the partition base offset. The lock
    /// takes full effect after the next reset.
    pub fn lock_partition(&self, base_byte_addr: usize) -> Result<(), OtpError> {
        self.wait_idle()?;
        self.regs().direct_access_address.set(base_byte_addr as u32);
        self.regs()
            .direct_access_cmd
            .write(DirectAccessCmd::Digest::SET);
        self.wait_idle()?;
        self.check_error()
    }
}

impl ErrorType for OtpController {
    type Error = OtpError;
}

impl OtpRead<u32> for OtpController {
    type Region = Partition;

    fn read(&self, region: Self::Region, offset: OtpOffset) -> Result<u32, Self::Error> {
        self.read_word(region.base() + offset.bytes())
    }
}

impl OtpWordRead for OtpController {}

impl OtpRegionLayout for OtpController {
    type Region = Partition;

    fn region_capacity(&self, region: Self::Region) -> usize {
        region.size()
    }

    fn read_alignment(&self, _region: Self::Region) -> usize {
        WORD_ALIGNMENT
    }
}

impl OtpRegionStatusAccess for OtpController {
    type Region = Partition;

    fn region_status(&self, _region: Self::Region) -> Result<OtpRegionStatus, Self::Error> {
        // Readability is governed globally by lifecycle state; a set error bit
        // is the only per-access signal exposed through the DAI status word.
        if self.regs().otp_status.get() & OTP_STATUS_ERROR_MASK != 0 {
            Ok(OtpRegionStatus::Error)
        } else {
            Ok(OtpRegionStatus::Readable)
        }
    }
}

impl OtpProgram<u32> for OtpController {
    fn write(
        &mut self,
        region: Self::Region,
        offset: OtpOffset,
        data: u32,
    ) -> Result<(), Self::Error> {
        self.write_word(region.base() + offset.bytes(), data)
    }

    fn lock_region(&mut self, region: Self::Region) -> Result<(), Self::Error> {
        self.lock_partition(region.base())
    }
}

impl OtpWordProgram for OtpController {}

impl OtpReadBytes for OtpController {
    type Region = Partition;

    fn read_bytes(
        &self,
        region: Self::Region,
        offset: OtpOffset,
        buf: &mut [u8],
    ) -> Result<(), Self::Error> {
        let start = offset.bytes();
        let end = start
            .checked_add(buf.len())
            .ok_or(OtpError::new(ErrorKind::InvalidAddress))?;
        if end > region.size() {
            return Err(OtpError::new(ErrorKind::InvalidAddress));
        }
        // The DAI transfers whole 32-bit words at word-aligned addresses; copy
        // out the requested byte span, tolerating an unaligned head and a
        // partial trailing word.
        let mut pos = start;
        let mut written = 0;
        while written < buf.len() {
            let word_base = pos & !(WORD_ALIGNMENT - 1);
            let word = self.read_word(region.base() + word_base)?;
            let word_bytes = word.to_le_bytes();
            let within = pos - word_base;
            let n = core::cmp::min(WORD_ALIGNMENT - within, buf.len() - written);
            buf[written..written + n].copy_from_slice(&word_bytes[within..within + n]);
            written += n;
            pos += n;
        }
        Ok(())
    }
}

impl OtpProgramBytes for OtpController {
    fn program_bytes(
        &mut self,
        region: Self::Region,
        offset: OtpOffset,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        let start = offset.bytes();
        // OTP is write-once; a sub-word write would require read-modify-write
        // and risk clobbering already-programmed bits. Require word alignment.
        if start % WORD_ALIGNMENT != 0 || data.len() % WORD_ALIGNMENT != 0 {
            return Err(OtpError::new(ErrorKind::AlignmentError));
        }
        let end = start
            .checked_add(data.len())
            .ok_or(OtpError::new(ErrorKind::InvalidAddress))?;
        if end > region.size() {
            return Err(OtpError::new(ErrorKind::InvalidAddress));
        }
        for (i, chunk) in data.chunks_exact(WORD_ALIGNMENT).enumerate() {
            let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            self.write_word(region.base() + start + i * WORD_ALIGNMENT, word)?;
        }
        Ok(())
    }
}
