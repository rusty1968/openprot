// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use core::num::NonZero;

use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, FmcReady, SmcController, SmcError, SpiNorFlash, SpiNorFlashDevice,
    SpiReady,
};
use hal_flash::{Flash, FlashAddress};
use util_error::{self as error, ErrorCode};
use util_types::PowerOf2Usize;

pub const MAX_DEVICES: usize = 6;

pub trait SmcNotify {
    fn wait_dma_complete(&self);
    fn yield_poll(&self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmcDeviceCfg {
    pub controller: SmcController,
    pub cs: ChipSelect,
    pub cfg: FlashConfig,
    pub base: u32,
    pub len: u32,
}

pub struct SmcFlashMux<TNotifier: SmcNotify> {
    fmc: Option<FmcReady>,
    spi1: Option<SpiReady>,
    spi2: Option<SpiReady>,
    map: [Option<SmcDeviceCfg>; MAX_DEVICES],
    device_count: usize,
    _notify: TNotifier,
    geometry0: (NonZero<usize>, PowerOf2Usize, u32),
}

impl<TNotifier: SmcNotify> SmcFlashMux<TNotifier> {
    pub fn new(
        fmc: Option<FmcReady>,
        spi1: Option<SpiReady>,
        spi2: Option<SpiReady>,
        map: [Option<SmcDeviceCfg>; MAX_DEVICES],
        notify: TNotifier,
    ) -> Result<Self, ErrorCode> {
        let mut device_count = 0usize;
        let mut seen_none = false;
        let mut i = 0usize;
        while i < MAX_DEVICES {
            let slot = map
                .get(i)
                .copied()
                .ok_or(error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)?;
            if slot.is_some() {
                if seen_none {
                    return Err(error::FLASH_GENERIC_INVALID_SIZE);
                }
                device_count = device_count
                    .checked_add(1)
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
            } else {
                seen_none = true;
            }
            i = i.checked_add(1).ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
        }

        let first = map
            .first()
            .copied()
            .flatten()
            .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
        let geometry0 = geometry_of(&first.cfg)?;

        Ok(Self {
            fmc,
            spi1,
            spi2,
            map,
            device_count,
            _notify: notify,
            geometry0,
        })
    }

    fn device_cfg(&self, device_id: u32) -> Result<SmcDeviceCfg, ErrorCode> {
        let idx = usize::try_from(device_id).map_err(|_| error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)?;
        if idx >= self.device_count {
            return Err(error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS);
        }
        self.map
            .get(idx)
            .copied()
            .flatten()
            .ok_or(error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)
    }

    fn resolve(&self, addr: FlashAddress, len: usize) -> Result<(SmcDeviceCfg, u32), ErrorCode> {
        let cfg = self.device_cfg(addr.device_id())?;

        let end = u64::from(addr.offset())
            .checked_add(u64::try_from(len).map_err(|_| error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)?)
            .ok_or(error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)?;
        if end > u64::from(cfg.len) {
            return Err(error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS);
        }

        let cs_off = cfg
            .base
            .checked_add(addr.offset())
            .ok_or(error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)?;
        Ok((cfg, cs_off))
    }
}

impl<TNotifier: SmcNotify> Flash for SmcFlashMux<TNotifier> {
    fn geometry(&self) -> (NonZero<usize>, PowerOf2Usize, u32) {
        self.geometry0
    }

    fn read(&mut self, addr: FlashAddress, buf: &mut [u8]) -> Result<(), ErrorCode> {
        let (cfg, off) = self.resolve(addr, buf.len())?;
        match cfg.controller {
            SmcController::Fmc => {
                let fmc = self
                    .fmc
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let flash = SpiNorFlash::from_fmc_cs(fmc, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.read(off, buf).map(|_| ()).map_err(smc_to_error)
            }
            SmcController::Spi1 => {
                let spi = self
                    .spi1
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let flash = SpiNorFlash::from_spi_cs(spi, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.read(off, buf).map(|_| ()).map_err(smc_to_error)
            }
            SmcController::Spi2 => {
                let spi = self
                    .spi2
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let flash = SpiNorFlash::from_spi_cs(spi, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.read(off, buf).map(|_| ()).map_err(smc_to_error)
            }
        }
    }

    fn erase(&mut self, addr: FlashAddress, size: PowerOf2Usize) -> Result<(), ErrorCode> {
        let (cfg, off) = self.resolve(addr, size.get())?;
        match cfg.controller {
            SmcController::Fmc => {
                let fmc = self
                    .fmc
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let mut flash =
                    SpiNorFlash::from_fmc_cs(fmc, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.erase_range(off, size.get()).map_err(smc_to_error)
            }
            SmcController::Spi1 => {
                let spi = self
                    .spi1
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let mut flash =
                    SpiNorFlash::from_spi_cs(spi, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.erase_range(off, size.get()).map_err(smc_to_error)
            }
            SmcController::Spi2 => {
                let spi = self
                    .spi2
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let mut flash =
                    SpiNorFlash::from_spi_cs(spi, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.erase_range(off, size.get()).map_err(smc_to_error)
            }
        }
    }

    fn program(&mut self, addr: FlashAddress, data: &[u8]) -> Result<(), ErrorCode> {
        let (cfg, off) = self.resolve(addr, data.len())?;
        match cfg.controller {
            SmcController::Fmc => {
                let fmc = self
                    .fmc
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let mut flash =
                    SpiNorFlash::from_fmc_cs(fmc, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.program(off, data).map(|_| ()).map_err(smc_to_error)
            }
            SmcController::Spi1 => {
                let spi = self
                    .spi1
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let mut flash =
                    SpiNorFlash::from_spi_cs(spi, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.program(off, data).map(|_| ()).map_err(smc_to_error)
            }
            SmcController::Spi2 => {
                let spi = self
                    .spi2
                    .as_mut()
                    .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
                let mut flash =
                    SpiNorFlash::from_spi_cs(spi, cfg.cfg, cfg.cs).map_err(smc_to_error)?;
                flash.program(off, data).map(|_| ()).map_err(smc_to_error)
            }
        }
    }
}

fn geometry_of(cfg: &FlashConfig) -> Result<(NonZero<usize>, PowerOf2Usize, u32), ErrorCode> {
    let total = usize::try_from(cfg.capacity_mb)
        .map_err(|_| error::FLASH_GENERIC_INVALID_SIZE)?
        .checked_mul(1024 * 1024)
        .ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
    let total = NonZero::new(total).ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
    let page = PowerOf2Usize::new(cfg.sector_size as usize).ok_or(error::FLASH_GENERIC_INVALID_SIZE)?;
    Ok((total, page, cfg.sector_size))
}

fn smc_to_error(err: SmcError) -> ErrorCode {
    match err {
        SmcError::InvalidCapacity => error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS,
        SmcError::WriteInProgress => error::FLASH_GENERIC_BUSY,
        SmcError::WriteProtected => error::FLASH_GENERIC_BUSY,
        SmcError::Timeout => error::FLASH_GENERIC_BUSY,
        SmcError::ControllerNotReady => error::FLASH_GENERIC_BUSY,
        SmcError::InvalidChipSelect => error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS,
        SmcError::DeviceNotSupported => error::FLASH_GENERIC_INVALID_SIZE,
        SmcError::HardwareError
        | SmcError::DmaAborted
        | SmcError::DmaLengthMismatch
        | SmcError::DmaNotEnabled => error::FLASH_GENERIC_BUSY,
    }
}
