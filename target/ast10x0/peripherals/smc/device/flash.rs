// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Read-only SPI NOR facade for Phase 3A.

use crate::smc::fmc::FmcReady;
use crate::smc::helpers::flash_capacity_bytes;
use crate::smc::spi::SpiReady;
use crate::smc::types::{FlashConfig, SmcError};

/// Minimal read-only flash device API.
pub trait FlashDevice {
    /// Read bytes from flash at `offset` into `buf`.
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError>;

    /// Return configured flash capacity in bytes.
    fn capacity_bytes(&self) -> Result<usize, SmcError>;
}

enum FlashBackend<'a> {
    Fmc(&'a FmcReady),
    Spi(&'a SpiReady),
}

/// Wrapper-aware SPI NOR flash facade.
pub struct SpiNorFlash<'a> {
    backend: FlashBackend<'a>,
    cfg: FlashConfig,
}

impl<'a> SpiNorFlash<'a> {
    /// Build a flash facade from an initialized FMC controller wrapper.
    pub fn from_fmc(fmc: &'a mut FmcReady, cfg: FlashConfig) -> Self {
        Self {
            backend: FlashBackend::Fmc(fmc),
            cfg,
        }
    }

    /// Build a flash facade from an initialized SPI1/SPI2 controller wrapper.
    pub fn from_spi(spi: &'a mut SpiReady, cfg: FlashConfig) -> Self {
        Self {
            backend: FlashBackend::Spi(spi),
            cfg,
        }
    }

    fn capacity_from_config(cfg: FlashConfig) -> Result<usize, SmcError> {
        flash_capacity_bytes(Some(cfg))
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
        Self::capacity_from_config(self.cfg)
    }
}
