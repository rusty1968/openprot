// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! SPI NOR facade with Phase 3A read support and Phase 3B API scaffolding.

use crate::smc::fmc::FmcReady;
use crate::smc::helpers::flash_capacity_bytes;
use crate::smc::spi::SpiReady;
use crate::smc::types::{FlashConfig, SmcError, TransferMode};

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

enum FlashBackend<'a> {
    Fmc(&'a FmcReady),
    Spi(&'a SpiReady),
}

/// Wrapper-aware SPI NOR flash facade.
pub struct SpiNorFlash<'a> {
    backend: FlashBackend<'a>,
    // Validated metadata for Phase 3B alignment/policy checks.
    cfg: FlashConfig,
}

impl<'a> SpiNorFlash<'a> {
    /// Build a flash facade from an initialized FMC controller wrapper.
    pub fn from_fmc(fmc: &'a mut FmcReady, cfg: FlashConfig) -> Result<Self, SmcError> {
        Self::validate_capacity_cfg(cfg, fmc.capacity_bytes()?)?;
        Ok(Self {
            backend: FlashBackend::Fmc(fmc),
            cfg,
        })
    }

    /// Build a flash facade from an initialized SPI1/SPI2 controller wrapper.
    pub fn from_spi(spi: &'a mut SpiReady, cfg: FlashConfig) -> Result<Self, SmcError> {
        Self::validate_capacity_cfg(cfg, spi.capacity_bytes()?)?;
        Ok(Self {
            backend: FlashBackend::Spi(spi),
            cfg,
        })
    }

    fn validate_capacity_cfg(cfg: FlashConfig, controller_capacity: usize) -> Result<(), SmcError> {
        let cfg_capacity = flash_capacity_bytes(Some(cfg))?;
        if cfg_capacity != controller_capacity {
            return Err(SmcError::InvalidCapacity);
        }
        Ok(())
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
        match &self.backend {
            FlashBackend::Fmc(fmc) => fmc.transceive_user(_cmd, _payload, &mut [], TransferMode::Mode111),
            FlashBackend::Spi(spi) => spi.transceive_user(_cmd, _payload, &mut [], TransferMode::Mode111),
        }
    }

    fn read_status_impl(&self) -> Result<u8, SmcError> {
        let mut status = [0u8; 1];
        match &self.backend {
            FlashBackend::Fmc(fmc) => {
                fmc.transceive_user(&[commands::READ_STATUS], &[], &mut status, TransferMode::Mode111)?
            }
            FlashBackend::Spi(spi) => {
                spi.transceive_user(&[commands::READ_STATUS], &[], &mut status, TransferMode::Mode111)?
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
        match &self.backend {
            FlashBackend::Fmc(fmc) => fmc.read(offset, buf),
            FlashBackend::Spi(spi) => spi.read(offset, buf),
        }
    }

    fn capacity_bytes(&self) -> Result<usize, SmcError> {
        match &self.backend {
            FlashBackend::Fmc(fmc) => fmc.capacity_bytes(),
            FlashBackend::Spi(spi) => spi.capacity_bytes(),
        }
    }

    fn erase_sector(&mut self, offset: u32) -> Result<(), SmcError> {
        self.validate_sector_erase(offset)?;

        self.issue_command(&[commands::WRITE_ENABLE], &[])?;
        let addr = offset.to_be_bytes();
        let cmd = [commands::ERASE_SECTOR_4K, addr[1], addr[2], addr[3]];
        self.issue_command(&cmd, &[])?;
        self.wait_write_complete(10_000)
    }

    fn program_page(&mut self, offset: u32, data: &[u8]) -> Result<usize, SmcError> {
        self.validate_page_program(offset, data)?;

        self.issue_command(&[commands::WRITE_ENABLE], &[])?;
        let addr = offset.to_be_bytes();
        let cmd = [commands::PAGE_PROGRAM, addr[1], addr[2], addr[3]];
        self.issue_command(&cmd, data)?;
        self.wait_write_complete(10_000)?;
        Ok(data.len())
    }

    fn verify(&self, offset: u32, expected: &[u8]) -> Result<bool, SmcError> {
        self.validate_range(offset, expected.len())?;
        let mut scratch = [0u8; 256];
        if expected.len() > scratch.len() {
            return Err(SmcError::InvalidCapacity);
        }
        self.read(offset, &mut scratch[..expected.len()])?;
        Ok(&scratch[..expected.len()] == expected)
    }

    fn status(&self) -> Result<u8, SmcError> {
        self.read_status_impl()
    }
}
