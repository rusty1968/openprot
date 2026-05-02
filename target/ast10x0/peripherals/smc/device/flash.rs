// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! SPI NOR facade with Phase 3A read support and Phase 3B API scaffolding.

use crate::smc::fmc::FmcReady;
use crate::smc::spi::SpiReady;
use crate::smc::types::{AddressWidth, ChipSelect, FlashConfig, SmcError, TransferMode};

/// Build a command byte array: opcode followed by address bytes selected by
/// `width`. Returns a fixed-size buffer and the valid length.
///
/// The caller slices `&buf[..len]` to get the exact command bytes.
/// This mirrors aspeed-rust's explicit `AddressWidth` selection rather than
/// relying on implicit `to_be_bytes()[1..]` slicing.
fn encode_addr_cmd(opcode: u8, offset: u32, width: AddressWidth) -> ([u8; 5], usize) {
    let be = offset.to_be_bytes();
    match width {
        AddressWidth::None => {
            let mut buf = [0u8; 5];
            buf[0] = opcode;
            (buf, 1)
        }
        AddressWidth::ThreeByte => {
            let mut buf = [0u8; 5];
            buf[0] = opcode;
            buf[1] = be[1];
            buf[2] = be[2];
            buf[3] = be[3];
            (buf, 4)
        }
        AddressWidth::FourByte => {
            let mut buf = [0u8; 5];
            buf[0] = opcode;
            buf[1] = be[0];
            buf[2] = be[1];
            buf[3] = be[2];
            buf[4] = be[3];
            (buf, 5)
        }
    }
}

/// Minimal read-only flash device API.
pub trait FlashDevice {
    /// Read bytes from flash at `offset` into `buf`.
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError>;

    /// Return configured flash capacity in bytes.
    fn capacity_bytes(&self) -> Result<usize, SmcError>;

    /// Erase one sector at `offset`.
    fn erase_sector(&mut self, offset: u32) -> Result<(), SmcError>;

    /// Program one page at `offset`.
    fn program_page(&mut self, offset: u32, data: &[u8]) -> Result<usize, SmcError>;

    /// Verify flash content against `expected` bytes.
    fn verify(&self, offset: u32, expected: &[u8]) -> Result<bool, SmcError>;

    /// Read status register.
    fn status(&self) -> Result<u8, SmcError>;
}

/// Standard SPI NOR opcodes used by Phase 3B operations.
pub mod commands {
    pub const WRITE_ENABLE: u8 = 0x06;
    pub const ERASE_SECTOR_4K: u8 = 0x20;
    pub const PAGE_PROGRAM: u8 = 0x02;
    pub const READ_STATUS: u8 = 0x05;
}

/// Compare `expected` against bytes produced by `read`, in chunks of at most
/// `chunk` bytes. `read(offset, dst)` fills `dst` with bytes at the given
/// device-local offset. Returns `Ok(true)` on full equality, `Ok(false)` on the
/// first mismatch, and propagates any error from `read`. Bounded scratch use:
/// the comparison is performed in stack-resident chunks (`chunk` bytes, capped
/// at 256) regardless of `expected.len()`.
fn compare_chunked<F>(
    mut read: F,
    offset: u32,
    expected: &[u8],
    chunk: usize,
) -> Result<bool, SmcError>
where
    F: FnMut(u32, &mut [u8]) -> Result<usize, SmcError>,
{
    let mut scratch = [0u8; 256];
    let step = core::cmp::min(chunk, scratch.len());
    if step == 0 {
        return Err(SmcError::InvalidCapacity);
    }
    let mut cursor = 0usize;
    while cursor < expected.len() {
        let n = core::cmp::min(step, expected.len() - cursor);
        let chunk_offset = offset
            .checked_add(cursor as u32)
            .ok_or(SmcError::InvalidCapacity)?;
        read(chunk_offset, &mut scratch[..n])?;
        if scratch[..n] != expected[cursor..cursor + n] {
            return Ok(false);
        }
        cursor += n;
    }
    Ok(true)
}

enum FlashBackend<'a> {
    Fmc(&'a FmcReady),
    Spi(&'a SpiReady),
}

/// Wrapper-aware SPI NOR flash facade.
pub struct SpiNorFlash<'a> {
    backend: FlashBackend<'a>,
    // Validated metadata for Phase 3B alignment/policy checks.
    cfg: FlashConfig,
    /// Chip select this flash device sits on.
    cs: ChipSelect,
}

impl<'a> SpiNorFlash<'a> {
    /// Build a flash facade from an initialized FMC controller wrapper.
    pub fn from_fmc(fmc: &'a mut FmcReady, cfg: FlashConfig) -> Result<Self, SmcError> {
        Self::from_fmc_cs(fmc, cfg, ChipSelect::Cs0)
    }

    /// Build a flash facade from an initialized FMC controller wrapper with explicit CS.
    pub fn from_fmc_cs(fmc: &'a mut FmcReady, cfg: FlashConfig, cs: ChipSelect) -> Result<Self, SmcError> {
        Self::validate_capacity_cfg(cfg, fmc.cs_config(cs)?)?;
        Ok(Self {
            backend: FlashBackend::Fmc(fmc),
            cfg,
            cs,
        })
    }

    /// Build a flash facade from an initialized SPI1/SPI2 controller wrapper.
    pub fn from_spi(spi: &'a mut SpiReady, cfg: FlashConfig) -> Result<Self, SmcError> {
        Self::from_spi_cs(spi, cfg, ChipSelect::Cs0)
    }

    /// Build a flash facade from an initialized SPI1/SPI2 controller wrapper with explicit CS.
    pub fn from_spi_cs(spi: &'a mut SpiReady, cfg: FlashConfig, cs: ChipSelect) -> Result<Self, SmcError> {
        Self::validate_capacity_cfg(cfg, spi.cs_config(cs)?)?;
        Ok(Self {
            backend: FlashBackend::Spi(spi),
            cfg,
            cs,
        })
    }

    fn validate_capacity_cfg(cfg: FlashConfig, expected: FlashConfig) -> Result<(), SmcError> {
        if cfg != expected {
            return Err(SmcError::InvalidCapacity);
        }
        Ok(())
    }

    fn cs_config_for(&self, cs: ChipSelect) -> Result<FlashConfig, SmcError> {
        match &self.backend {
            FlashBackend::Fmc(fmc) => fmc.cs_config(cs),
            FlashBackend::Spi(spi) => spi.cs_config(cs),
        }
    }

    /// Translate a device-local offset into the controller-window address that
    /// the segment-routed read path expects.
    ///
    /// Read traffic flows through the AHB flash window; the controller's
    /// segment registers map `[CS0_BASE, CS0_BASE+CS0_SIZE)` to CS0 and
    /// `[CS0_BASE+CS0_SIZE, CS0_BASE+TOTAL)` to CS1. The user-mode command
    /// path (`transceive_user`) is *not* segment-routed — its on-wire address
    /// bytes are already device-local from the chip's perspective — so this
    /// translation is read-only.
    fn device_to_controller_offset(&self, device_offset: u32) -> Result<u32, SmcError> {
        let cs_cap = self.capacity_bytes()?;
        if (device_offset as usize) >= cs_cap {
            return Err(SmcError::InvalidCapacity);
        }
        let base: u32 = match self.cs {
            ChipSelect::Cs0 => 0,
            ChipSelect::Cs1 => match self.cs_config_for(ChipSelect::Cs0) {
                Ok(cfg) => u32::try_from(
                    (cfg.capacity_mb as usize)
                        .checked_mul(1024 * 1024)
                        .ok_or(SmcError::InvalidCapacity)?,
                )
                .map_err(|_| SmcError::InvalidCapacity)?,
                Err(SmcError::InvalidChipSelect) => 0,
                Err(e) => return Err(e),
            },
        };
        base.checked_add(device_offset).ok_or(SmcError::InvalidCapacity)
    }

    fn validate_range(&self, offset: u32, len: usize) -> Result<(), SmcError> {
        let start = offset as usize;
        let end = start.checked_add(len).ok_or(SmcError::InvalidCapacity)?;
        if end > self.capacity_bytes()? {
            return Err(SmcError::InvalidCapacity);
        }
        Ok(())
    }

    fn validate_sector_erase(&self, offset: u32) -> Result<(), SmcError> {
        let sector_size = self.cfg.sector_size as usize;
        if sector_size == 0 || (offset as usize) % sector_size != 0 {
            return Err(SmcError::InvalidCapacity);
        }
        self.validate_range(offset, sector_size)
    }

    fn validate_page_program(&self, offset: u32, data: &[u8]) -> Result<(), SmcError> {
        let page_size = self.cfg.page_size as usize;
        if page_size == 0 || data.is_empty() || data.len() > page_size {
            return Err(SmcError::InvalidCapacity);
        }
        if (offset as usize) % page_size != 0 {
            return Err(SmcError::InvalidCapacity);
        }
        self.validate_range(offset, data.len())
    }

    fn issue_command(&mut self, _cmd: &[u8], _payload: &[u8]) -> Result<(), SmcError> {
        let cs = self.cs;
        match &self.backend {
            FlashBackend::Fmc(fmc) => fmc.transceive_user(cs, _cmd, _payload, &mut [], TransferMode::Mode111),
            FlashBackend::Spi(spi) => spi.transceive_user(cs, _cmd, _payload, &mut [], TransferMode::Mode111),
        }
    }

    fn read_status_impl(&self) -> Result<u8, SmcError> {
        let cs = self.cs;
        let mut status = [0u8; 1];
        match &self.backend {
            FlashBackend::Fmc(fmc) => {
                fmc.transceive_user(cs, &[commands::READ_STATUS], &[], &mut status, TransferMode::Mode111)?
            }
            FlashBackend::Spi(spi) => {
                spi.transceive_user(cs, &[commands::READ_STATUS], &[], &mut status, TransferMode::Mode111)?
            }
        }
        Ok(status[0])
    }

    fn wait_write_complete(&self, max_polls: u32) -> Result<(), SmcError> {
        let mut polls = 0u32;
        while polls < max_polls {
            let sr = self.read_status_impl()?;
            if (sr & 0x01) == 0 {
                return Ok(());
            }
            polls += 1;
        }
        Err(SmcError::Timeout)
    }
}

impl FlashDevice for SpiNorFlash<'_> {
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        // Bounds-check against the selected CS's capacity, then translate to
        // the controller-window address before issuing the segment-routed read.
        self.validate_range(offset, buf.len())?;
        let translated = self.device_to_controller_offset(offset)?;
        match &self.backend {
            FlashBackend::Fmc(fmc) => fmc.read(translated, buf),
            FlashBackend::Spi(spi) => spi.read(translated, buf),
        }
    }

    fn capacity_bytes(&self) -> Result<usize, SmcError> {
        match &self.backend {
            FlashBackend::Fmc(fmc) => fmc.cs_capacity_bytes(self.cs),
            FlashBackend::Spi(spi) => spi.cs_capacity_bytes(self.cs),
        }
    }

    fn erase_sector(&mut self, offset: u32) -> Result<(), SmcError> {
        self.validate_sector_erase(offset)?;

        self.issue_command(&[commands::WRITE_ENABLE], &[])?;
        let (cmd, len) = encode_addr_cmd(commands::ERASE_SECTOR_4K, offset, AddressWidth::ThreeByte);
        self.issue_command(&cmd[..len], &[])?;
        self.wait_write_complete(10_000)
    }

    fn program_page(&mut self, offset: u32, data: &[u8]) -> Result<usize, SmcError> {
        self.validate_page_program(offset, data)?;

        self.issue_command(&[commands::WRITE_ENABLE], &[])?;
        let (cmd, len) = encode_addr_cmd(commands::PAGE_PROGRAM, offset, AddressWidth::ThreeByte);
        self.issue_command(&cmd[..len], data)?;
        self.wait_write_complete(10_000)?;
        Ok(data.len())
    }

    fn verify(&self, offset: u32, expected: &[u8]) -> Result<bool, SmcError> {
        self.validate_range(offset, expected.len())?;
        compare_chunked(|o, b| self.read(o, b), offset, expected, 256)
    }

    fn status(&self) -> Result<u8, SmcError> {
        self.read_status_impl()
    }
}

#[cfg(test)]
mod tests {
    use super::{compare_chunked, encode_addr_cmd};
    use crate::smc::types::{AddressWidth, SmcError};

    #[test]
    fn encode_addr_cmd_none_emits_opcode_only() {
        let (buf, len) = encode_addr_cmd(0x06, 0x1234_5678, AddressWidth::None);
        assert_eq!(len, 1);
        assert_eq!(&buf[..len], &[0x06]);
    }

    #[test]
    fn encode_addr_cmd_three_byte_emits_low_24_bits() {
        let (buf, len) = encode_addr_cmd(0x20, 0x1234_5678, AddressWidth::ThreeByte);
        assert_eq!(len, 4);
        assert_eq!(&buf[..len], &[0x20, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn encode_addr_cmd_four_byte_emits_full_32_bits() {
        let (buf, len) = encode_addr_cmd(0x13, 0x1234_5678, AddressWidth::FourByte);
        assert_eq!(len, 5);
        assert_eq!(&buf[..len], &[0x13, 0x12, 0x34, 0x56, 0x78]);
    }

    fn fixture_1kb() -> [u8; 1024] {
        let mut out = [0u8; 1024];
        let mut i = 0;
        while i < out.len() {
            out[i] = (i as u8).wrapping_mul(31).wrapping_add(7);
            i += 1;
        }
        out
    }

    #[test]
    fn compare_chunked_matches_buffer_above_scratch_size() {
        let expected = fixture_1kb();
        let backing = expected;
        let read = |offset: u32, dst: &mut [u8]| -> Result<usize, SmcError> {
            let off = offset as usize;
            dst.copy_from_slice(&backing[off..off + dst.len()]);
            Ok(dst.len())
        };
        assert_eq!(compare_chunked(read, 0, &expected, 256), Ok(true));
    }

    #[test]
    fn compare_chunked_detects_mismatch_in_later_chunk() {
        let expected = fixture_1kb();
        let mut backing = expected;
        backing[800] = backing[800].wrapping_add(1);
        let read = |offset: u32, dst: &mut [u8]| -> Result<usize, SmcError> {
            let off = offset as usize;
            dst.copy_from_slice(&backing[off..off + dst.len()]);
            Ok(dst.len())
        };
        assert_eq!(compare_chunked(read, 0, &expected, 256), Ok(false));
    }

    #[test]
    fn compare_chunked_propagates_read_error() {
        let expected = fixture_1kb();
        let read = |_offset: u32, _dst: &mut [u8]| -> Result<usize, SmcError> {
            Err(SmcError::Timeout)
        };
        assert_eq!(compare_chunked(read, 0, &expected, 256), Err(SmcError::Timeout));
    }
}
