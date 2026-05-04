// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use ast10x0_board_descriptors::{Ast10x0BoardDescriptor, SpimWiringError, apply_spim_wiring};
use ast10x0_peripherals::scu::ScuRegisters;
use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, FmcReady, FmcUninit, SmcConfig, SmcController, SmcError, SpiNorFlash,
    SpiNorFlashDevice, SpiReady, SpiUninit,
};

/// Re-export so test binaries can name the route key without taking a
/// separate dependency on `ast10x0_peripherals`.
pub use ast10x0_peripherals::smc::ChipSelect as Cs;
use ast10x0_peripherals::spimonitor::LockedSpiMonitor;
use flash_api::backend::{BackendError, FlashBackend, FlashInfo, IrqMask};

/// Errors raised while constructing the flash backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendInitError {
    /// Descriptor did not provide a CS0 flash configuration.
    MissingCs0Config,
    /// FMC controller cannot have SPIM wiring; descriptor must set `spim_wiring: None`.
    FmcWithSpimWiring,
    /// SPI controller requires SPIM wiring; descriptor must set `spim_wiring: Some(_)`.
    SpiWithoutSpimWiring,
    /// Applying SCU/SPIPF wiring failed.
    SpimWiring(SpimWiringError),
    /// SMC controller construction or init failed.
    Smc(SmcError),
}

impl From<SpimWiringError> for BackendInitError {
    fn from(value: SpimWiringError) -> Self {
        Self::SpimWiring(value)
    }
}

impl From<SmcError> for BackendInitError {
    fn from(value: SmcError) -> Self {
        Self::Smc(value)
    }
}

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
    cs0_cfg: FlashConfig,
    cs1_cfg: Option<FlashConfig>,
    /// SPIPF lock witness held for the lifetime of the backend. `None` for
    /// FMC; `Some(_)` for SPI controllers. Dropping does not unlock — the
    /// SPIPF lock is one-way per silicon spec.
    _monitor: Option<LockedSpiMonitor>,
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
    /// Construct backend for the FMC controller with the default flash config.
    pub fn new() -> Result<Self, BackendInitError> {
        Self::new_for_controller(Ast10x0Controller::Fmc)
    }

    /// Construct an FMC-backed backend.
    pub fn new_fmc() -> Result<Self, BackendInitError> {
        Self::new_for_controller(Ast10x0Controller::Fmc)
    }

    /// Construct an SPI1-backed backend with default SPIM0 wiring and the
    /// BMC default opcode allow-list policy.
    pub fn new_spi1() -> Result<Self, BackendInitError> {
        Self::new_with_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_spi1())
    }

    /// Construct an SPI2-backed backend with default SPIM2 wiring and the
    /// BMC default opcode allow-list policy.
    pub fn new_spi2() -> Result<Self, BackendInitError> {
        Self::new_with_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_spi2())
    }

    /// Construct an FMC-backed backend exposing both CS0 and CS1.
    pub fn new_fmc_dual_cs() -> Result<Self, BackendInitError> {
        Self::new_with_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_dual_cs())
    }

    /// Construct an SPI1-backed dual-CS backend.
    pub fn new_spi1_dual_cs() -> Result<Self, BackendInitError> {
        Self::new_with_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_spi1_dual_cs())
    }

    /// Construct an SPI2-backed dual-CS backend.
    pub fn new_spi2_dual_cs() -> Result<Self, BackendInitError> {
        Self::new_with_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_spi2_dual_cs())
    }

    /// SPI1-backed dual-CS backend assuming kernel-side pre-wiring.
    pub fn new_spi1_dual_cs_pre_wired() -> Result<Self, BackendInitError> {
        Self::new_with_pre_wired_descriptor(
            Ast10x0BoardDescriptor::ast10x0_qemu_default_spi1_dual_cs(),
        )
    }

    /// SPI2-backed dual-CS backend assuming kernel-side pre-wiring.
    pub fn new_spi2_dual_cs_pre_wired() -> Result<Self, BackendInitError> {
        Self::new_with_pre_wired_descriptor(
            Ast10x0BoardDescriptor::ast10x0_qemu_default_spi2_dual_cs(),
        )
    }

    /// Construct a backend for the requested controller using a built-in
    /// default descriptor. SPI controllers route through their default SPIM
    /// instance with `presets::bmc_default_policy()`.
    pub fn new_for_controller(controller: Ast10x0Controller) -> Result<Self, BackendInitError> {
        match controller {
            Ast10x0Controller::Fmc => {
                Self::new_with_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default())
            }
            Ast10x0Controller::Spi1 => {
                Self::new_with_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_spi1())
            }
            Ast10x0Controller::Spi2 => {
                Self::new_with_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_spi2())
            }
        }
    }

    /// Construct a backend from a board descriptor.
    ///
    /// For SPI controllers, applies SCU mux + SPIPF policy + SPIPF lock
    /// before initializing the SMC controller. For FMC, no SPIM step runs.
    /// The SPIPF lock is one-way; choose the policy carefully.
    pub fn new_with_descriptor(
        descriptor: Ast10x0BoardDescriptor,
    ) -> Result<Self, BackendInitError> {
        let cs0_cfg = descriptor.cs0.ok_or(BackendInitError::MissingCs0Config)?;
        let cs1_cfg = descriptor.cs1;

        let monitor = match (descriptor.controller, descriptor.spim_wiring.as_ref()) {
            (SmcController::Fmc, None) => None,
            (SmcController::Fmc, Some(_)) => {
                return Err(BackendInitError::FmcWithSpimWiring);
            }
            (SmcController::Spi1, None) | (SmcController::Spi2, None) => {
                return Err(BackendInitError::SpiWithoutSpimWiring);
            }
            (controller_id, Some(wiring)) => {
                // SAFETY: the backend takes exclusive ownership of the SCU
                // block and the routed SPIPF block for its lifetime; one
                // backend instance exists per server process.
                let scu = unsafe { ScuRegisters::new_global() };
                let locked = unsafe {
                    apply_spim_wiring(&scu, controller_id, *wiring, &descriptor.monitor_policy)
                }?;
                Some(locked)
            }
        };

        let controller = build_smc_controller(descriptor.controller, cs0_cfg, cs1_cfg)?;
        Ok(Self {
            controller,
            cs0_cfg,
            cs1_cfg,
            _monitor: monitor,
        })
    }

    /// Construct a backend assuming SPIM wiring + SPIPF policy + SPIPF
    /// lock have already been programmed by trusted setup code (typically
    /// the kernel target's `main` before `codegen::start()`). Does not
    /// touch SCU or SPIPF blocks.
    ///
    /// Lets the SPI server processes run without MMIO access to SCU or
    /// SPIPF, preserving per-process isolation.
    pub fn new_with_pre_wired_descriptor(
        descriptor: Ast10x0BoardDescriptor,
    ) -> Result<Self, BackendInitError> {
        let cs0_cfg = descriptor.cs0.ok_or(BackendInitError::MissingCs0Config)?;
        let cs1_cfg = descriptor.cs1;

        match (descriptor.controller, descriptor.spim_wiring.as_ref()) {
            (SmcController::Fmc, None) => {}
            (SmcController::Fmc, Some(_)) => {
                return Err(BackendInitError::FmcWithSpimWiring);
            }
            (SmcController::Spi1, None) | (SmcController::Spi2, None) => {
                return Err(BackendInitError::SpiWithoutSpimWiring);
            }
            (_, Some(_)) => {}
        }

        let controller = build_smc_controller(descriptor.controller, cs0_cfg, cs1_cfg)?;
        Ok(Self {
            controller,
            cs0_cfg,
            cs1_cfg,
            _monitor: None,
        })
    }

    /// SPI1-backed backend assuming kernel-side pre-wiring.
    pub fn new_spi1_pre_wired() -> Result<Self, BackendInitError> {
        Self::new_with_pre_wired_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_spi1())
    }

    /// SPI2-backed backend assuming kernel-side pre-wiring.
    pub fn new_spi2_pre_wired() -> Result<Self, BackendInitError> {
        Self::new_with_pre_wired_descriptor(Ast10x0BoardDescriptor::ast10x0_qemu_default_spi2())
    }

    /// Look up the per-CS flash configuration. Returns `InvalidOperation`
    /// when the requested CS slot was not populated at construction time —
    /// this is the last-line guard against a misrouted channel.
    fn cfg_for(&self, cs: ChipSelect) -> Result<FlashConfig, BackendError> {
        match cs {
            ChipSelect::Cs0 => Ok(self.cs0_cfg),
            ChipSelect::Cs1 => self.cs1_cfg.ok_or(BackendError::InvalidOperation),
        }
    }

    fn with_flash<R>(
        &mut self,
        cs: ChipSelect,
        f: impl FnOnce(&mut SpiNorFlash<'_>) -> Result<R, SmcError>,
    ) -> Result<R, BackendError> {
        let cfg = self.cfg_for(cs)?;
        let mut flash = match &mut self.controller {
            ControllerBackend::Fmc(fmc) => SpiNorFlash::from_fmc_cs(fmc, cfg, cs),
            ControllerBackend::Spi(spi) => SpiNorFlash::from_spi_cs(spi, cfg, cs),
        }
        .map_err(smc_to_backend_error)?;
        f(&mut flash).map_err(smc_to_backend_error)
    }
}

fn build_smc_controller(
    controller: SmcController,
    cs0_cfg: FlashConfig,
    cs1_cfg: Option<FlashConfig>,
) -> Result<ControllerBackend, SmcError> {
    let config = SmcConfig {
        controller_id: controller,
        cs0: Some(cs0_cfg),
        cs1: cs1_cfg,
        dma_enabled: false,
        enable_interrupts: false,
    };

    match controller {
        SmcController::Fmc => {
            // SAFETY: backend owns the FMC controller for the process lifetime.
            let uninit = unsafe { FmcUninit::new(config) }?;
            let fmc = uninit.init()?;
            Ok(ControllerBackend::Fmc(fmc))
        }
        SmcController::Spi1 | SmcController::Spi2 => {
            // SAFETY: backend owns the SPI controller for the process lifetime.
            let uninit = unsafe { SpiUninit::new(controller, config) }?;
            let spi = uninit.init()?;
            Ok(ControllerBackend::Spi(spi))
        }
    }
}

pub type Backend = Ast10x0FlashBackend;

impl FlashBackend for Ast10x0FlashBackend {
    type RouteKey = ChipSelect;

    fn info(&self, key: ChipSelect) -> FlashInfo {
        // `info` is infallible by trait shape; for a CS slot that is not
        // configured we surface a zero-capacity descriptor so clients see
        // an empty device rather than a misleading sum.
        let cfg = match self.cfg_for(key) {
            Ok(cfg) => cfg,
            Err(_) => {
                return FlashInfo {
                    capacity: 0,
                    chunk_size: 0,
                    erase_size: 0,
                };
            }
        };
        FlashInfo {
            capacity: cfg.capacity_mb * 1024 * 1024,
            chunk_size: cfg.page_size,
            erase_size: cfg.sector_size,
        }
    }

    fn exists(&mut self, key: ChipSelect) -> Result<bool, BackendError> {
        let id = self.with_flash(key, |flash| flash.jedec_id())?;
        Ok(id != [0x00, 0x00, 0x00] && id != [0xFF, 0xFF, 0xFF])
    }

    fn read(
        &mut self,
        key: ChipSelect,
        address: u32,
        out: &mut [u8],
    ) -> Result<usize, BackendError> {
        self.with_flash(key, |flash| flash.read(address, out))
    }

    fn write(
        &mut self,
        key: ChipSelect,
        address: u32,
        data: &[u8],
    ) -> Result<usize, BackendError> {
        if data.is_empty() {
            return Ok(0);
        }

        let cfg = self.cfg_for(key)?;
        let page_size = cfg.page_size as usize;
        if (address as usize) % page_size != 0 {
            return Err(BackendError::InvalidAddress);
        }

        self.with_flash(key, |flash| flash.program(address, data))
    }

    fn erase(
        &mut self,
        key: ChipSelect,
        address: u32,
        length: u32,
    ) -> Result<(), BackendError> {
        if length == 0 {
            return Ok(());
        }

        let cfg = self.cfg_for(key)?;
        let erase_size = cfg.sector_size;
        if !address.is_multiple_of(erase_size) || !length.is_multiple_of(erase_size) {
            return Err(BackendError::InvalidLength);
        }

        self.with_flash(key, |flash| flash.erase_range(address, length as usize))
    }

    fn enable_interrupts(&mut self, _mask: IrqMask) -> Result<(), BackendError> {
        Ok(())
    }

    fn disable_interrupts(&mut self, _mask: IrqMask) -> Result<(), BackendError> {
        Ok(())
    }
}
