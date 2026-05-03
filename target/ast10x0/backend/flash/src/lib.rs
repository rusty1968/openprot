// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use ast10x0_board_descriptors::Ast10x0BoardDescriptor;
use ast10x0_peripherals::smc::{
    FlashConfig, SpiNorFlashDevice, FmcReady, FmcUninit, SmcConfig, SmcController, SmcError, SpiNorFlash,
    SpiReady, SpiUninit,
};
use flash_api::backend::{BackendError, FlashBackend, FlashInfo, IrqMask};

const FLASH_CFG: FlashConfig = FlashConfig {
    capacity_mb: 1,
    page_size: 256,
    sector_size: 4096,
    block_size: 65536,
    spi_clock_mhz: 25,
};

fn smc_to_backend_error(err: SmcError) -> BackendError {
    match err {
        SmcError::InvalidChipSelect => BackendError::InvalidAddress,
        SmcError::InvalidCapacity => BackendError::InvalidLength,
        SmcError::WriteProtected => BackendError::NotPermitted,
        SmcError::ControllerNotReady => BackendError::Busy,
        SmcError::Timeout => BackendError::Timeout,
        SmcError::DmaAborted | SmcError::DmaLengthMismatch => BackendError::IoError,
        SmcError::DeviceNotSupported
        | SmcError::WriteInProgress
        | SmcError::HardwareError => BackendError::InternalError,
    }
}

pub struct Ast10x0FlashBackend {
    controller: ControllerBackend,
    cfg: FlashConfig,
}

enum ControllerBackend {
    Fmc(FmcReady),
    Spi(SpiReady),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ast10x0Controller {
    Fmc,
    Spi1,
    Spi2,
}

impl Ast10x0FlashBackend {
    pub fn new() -> Self {
        Self::new_for_controller(Ast10x0Controller::Fmc)
    }

    pub fn new_fmc() -> Self {
        Self::new_for_controller(Ast10x0Controller::Fmc)
    }

    pub fn new_spi1() -> Self {
        Self::new_for_controller(Ast10x0Controller::Spi1)
    }

    pub fn new_spi2() -> Self {
        Self::new_for_controller(Ast10x0Controller::Spi2)
    }

    pub fn new_for_controller(controller: Ast10x0Controller) -> Self {
        // Use internal helper for default path - safe because FLASH_CFG is static and known.
        Self::build_controller(controller, FLASH_CFG)
    }

    /// Construct backend from a board descriptor skeleton.
    ///
    /// Current backend path only supports a single CS0 flash profile.
    /// Returns error if descriptor is missing CS0 or specifies unsupported CS1.
    pub fn new_with_descriptor(descriptor: Ast10x0BoardDescriptor) -> Result<Self, &'static str> {
        let cfg = descriptor
            .cs0
            .ok_or("board descriptor must provide CS0 flash configuration")?;
        
        if descriptor.cs1.is_some() {
            return Err("descriptor CS1 is not yet supported by this flash backend");
        }

        let controller = match descriptor.controller {
            SmcController::Fmc => Ast10x0Controller::Fmc,
            SmcController::Spi1 => Ast10x0Controller::Spi1,
            SmcController::Spi2 => Ast10x0Controller::Spi2,
        };

        Self::new_with_cfg_internal(controller, cfg).map_err(|_| "failed to initialize flash backend from descriptor")
    }

    /// Internal helper: build controller without exposing Result to public API paths.
    fn build_controller(controller: Ast10x0Controller, cfg: FlashConfig) -> Self {
        let config = SmcConfig {
            controller_id: match controller {
                Ast10x0Controller::Fmc => SmcController::Fmc,
                Ast10x0Controller::Spi1 => SmcController::Spi1,
                Ast10x0Controller::Spi2 => SmcController::Spi2,
            },
            cs0: Some(cfg),
            cs1: None,
            dma_enabled: false,
            enable_interrupts: false,
        };

        let backend = match controller {
            Ast10x0Controller::Fmc => {
                // SAFETY: This backend owns the FMC controller for the process lifetime.
                match (unsafe { FmcUninit::new(config) }).and_then(|u| u.init()) {
                    Ok(fmc) => ControllerBackend::Fmc(fmc),
                    Err(_) => loop {}, // Hang: FMC init failed with default config
                }
            }
            Ast10x0Controller::Spi1 => {
                // SAFETY: This backend owns SPI1 for the process lifetime.
                match (unsafe { SpiUninit::new(SmcController::Spi1, config) }).and_then(|u| u.init()) {
                    Ok(spi) => ControllerBackend::Spi(spi),
                    Err(_) => loop {}, // Hang: SPI1 init failed with default config
                }
            }
            Ast10x0Controller::Spi2 => {
                // SAFETY: This backend owns SPI2 for the process lifetime.
                match (unsafe { SpiUninit::new(SmcController::Spi2, config) }).and_then(|u| u.init()) {
                    Ok(spi) => ControllerBackend::Spi(spi),
                    Err(_) => loop {}, // Hang: SPI2 init failed with default config
                }
            }
        };

        Self { controller: backend, cfg }
    }

    /// Internal cfg constructor that returns Result for error handling during init.
    fn new_with_cfg_internal(controller: Ast10x0Controller, cfg: FlashConfig) -> Result<Self, ()> {
        let config = SmcConfig {
            controller_id: match controller {
                Ast10x0Controller::Fmc => SmcController::Fmc,
                Ast10x0Controller::Spi1 => SmcController::Spi1,
                Ast10x0Controller::Spi2 => SmcController::Spi2,
            },
            cs0: Some(cfg),
            cs1: None,
            dma_enabled: false,
            enable_interrupts: false,
        };

        let controller = match controller {
            Ast10x0Controller::Fmc => {
                // SAFETY: This backend owns the FMC controller for the process lifetime.
                let uninit = unsafe { FmcUninit::new(config) }.map_err(|_| ())?;
                let fmc = uninit.init().map_err(|_| ())?;
                ControllerBackend::Fmc(fmc)
            }
            Ast10x0Controller::Spi1 => {
                // SAFETY: This backend owns SPI1 for the process lifetime.
                let uninit = unsafe { SpiUninit::new(SmcController::Spi1, config) }.map_err(|_| ())?;
                let spi = uninit.init().map_err(|_| ())?;
                ControllerBackend::Spi(spi)
            }
            Ast10x0Controller::Spi2 => {
                // SAFETY: This backend owns SPI2 for the process lifetime.
                let uninit = unsafe { SpiUninit::new(SmcController::Spi2, config) }.map_err(|_| ())?;
                let spi = uninit.init().map_err(|_| ())?;
                ControllerBackend::Spi(spi)
            }
        };

        Ok(Self { controller, cfg })
    }

    fn with_flash<R>(
        &mut self,
        f: impl FnOnce(&mut SpiNorFlash<'_>) -> Result<R, SmcError>,
    ) -> Result<R, BackendError> {
        let mut flash = match &mut self.controller {
            ControllerBackend::Fmc(fmc) => SpiNorFlash::from_fmc(fmc, self.cfg),
            ControllerBackend::Spi(spi) => SpiNorFlash::from_spi(spi, self.cfg),
        }
        .map_err(smc_to_backend_error)?;
        f(&mut flash).map_err(smc_to_backend_error)
    }
}

impl Default for Ast10x0FlashBackend {
    fn default() -> Self {
        Self::new()
    }
}

pub type Backend = Ast10x0FlashBackend;

impl FlashBackend for Ast10x0FlashBackend {
    fn info(&self) -> FlashInfo {
        FlashInfo {
            capacity: self.cfg.capacity_mb * 1024 * 1024,
            chunk_size: self.cfg.page_size,
            erase_size: self.cfg.sector_size,
        }
    }

    fn exists(&mut self) -> Result<bool, BackendError> {
        let id = self.with_flash(|flash| flash.jedec_id())?;
        Ok(id != [0x00, 0x00, 0x00] && id != [0xFF, 0xFF, 0xFF])
    }

    fn read(&mut self, address: u32, out: &mut [u8]) -> Result<usize, BackendError> {
        self.with_flash(|flash| flash.read(address, out))
    }

    fn write(&mut self, address: u32, data: &[u8]) -> Result<usize, BackendError> {
        if data.is_empty() {
            return Ok(0);
        }

        let page_size = self.cfg.page_size as usize;
        if (address as usize) % page_size != 0 {
            return Err(BackendError::InvalidAddress);
        }

        self.with_flash(|flash| flash.program(address, data))
    }

    fn erase(&mut self, address: u32, length: u32) -> Result<(), BackendError> {
        if length == 0 {
            return Ok(());
        }

        let erase_size = self.cfg.sector_size;
        if !address.is_multiple_of(erase_size) || !length.is_multiple_of(erase_size) {
            return Err(BackendError::InvalidLength);
        }

        self.with_flash(|flash| flash.erase_range(address, length as usize))
    }

    fn enable_interrupts(&mut self, _mask: IrqMask) -> Result<(), BackendError> {
        Ok(())
    }

    fn disable_interrupts(&mut self, _mask: IrqMask) -> Result<(), BackendError> {
        Ok(())
    }
}
