// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! FMC-specialized wrapper around the generic SMC controller.
//!
//! # FMC vs SPI1/SPI2
//!
//! This module provides an abstraction **exclusively for the FMC (Firmware Memory Controller)**
//! block. The FMC connects directly to flash without SPI-monitor interception.
//!
//! **SPI1 and SPI2** (application SPI controllers that route through SPIPF monitor) are
//! handled by the separate [`crate::smc::spi`] module, not here.
//!
//! - **FMC**: Single dedicated flash controller, no SPI-monitor support, boot-time device
//! - **SPI1/SPI2**: Multi-instance application controllers with optional SPIPF monitoring
//!
//! See [`crate::smc`] module-level documentation for the full taxonomy.
//!
//! # Lifecycle
//!
//! [`FmcUninit`] → `init()` → [`FmcReady`] → `calibrate()` / `open_calibrated()`
//! → [`FmcCalibrated`]. I/O is only available on the calibrated handle. Run
//! `calibrate()` once during bring-up; the driver path uses `open_calibrated()`.

use crate::smc::controller::{CalibratedSmc, CalibrationScratch, ReadySmc, UninitSmc};
use crate::smc::interrupts::SmcInterrupt;
use crate::smc::types::{
    ChipSelect, FlashConfig, SmcConfig, SmcController, SmcError, TransferMode,
};

/// FMC handle before hardware initialization.
pub struct FmcUninit {
    inner: UninitSmc,
}

/// FMC handle after hardware initialization, before calibration.
pub struct FmcReady {
    inner: ReadySmc,
}

/// FMC handle after calibration: the operational, I/O-capable state.
pub struct FmcCalibrated {
    inner: CalibratedSmc,
}

impl FmcUninit {
    /// Construct an uninitialized FMC controller.
    ///
    /// # Safety
    /// Caller must ensure unique ownership of the FMC hardware block.
    pub unsafe fn new(mut config: SmcConfig) -> Result<Self, SmcError> {
        config.controller_id = SmcController::Fmc;
        // SAFETY: Caller upholds controller ownership requirements.
        let inner = unsafe { UninitSmc::new(config)? };
        Ok(Self { inner })
    }

    /// Initialize FMC hardware and transition to the ready (uncalibrated) state.
    pub fn init(self) -> Result<FmcReady, SmcError> {
        Ok(FmcReady {
            inner: self.inner.init()?,
        })
    }
}

impl FmcReady {
    /// Bring-up path: calibrate read timing and transition to operational state.
    ///
    /// `scratch` must live off the small driver stack (see
    /// [`CalibrationScratch`]). See [`crate::smc::Smc::calibrate`].
    pub fn calibrate(self, scratch: &mut CalibrationScratch) -> Result<FmcCalibrated, SmcError> {
        Ok(FmcCalibrated {
            inner: self.inner.calibrate(scratch)?,
        })
    }

    /// Driver path: open a controller calibrated earlier (e.g. in `board_init`).
    ///
    /// Returns `SmcError::NotCalibrated` if calibration has not been performed.
    /// See [`crate::smc::Smc::open_calibrated`].
    pub fn open_calibrated(self) -> Result<FmcCalibrated, SmcError> {
        Ok(FmcCalibrated {
            inner: self.inner.open_calibrated()?,
        })
    }

    /// Report whether read timing is calibrated in hardware for `cs`.
    pub fn is_calibrated(&self, cs: ChipSelect) -> bool {
        self.inner.is_calibrated(cs)
    }

    /// Get the controller identifier.
    pub fn controller_id(&self) -> SmcController {
        self.inner.controller_id()
    }

    /// Return configured flash capacity in bytes.
    pub fn capacity_bytes(&self) -> Result<usize, SmcError> {
        self.inner.capacity_bytes()
    }

    /// Return configured flash capacity in bytes for the given chip select.
    pub fn cs_capacity_bytes(&self, cs: ChipSelect) -> Result<usize, SmcError> {
        self.inner.cs_capacity_bytes(cs)
    }

    /// Return the configured `FlashConfig` for the requested chip select.
    pub fn cs_config(&self, cs: ChipSelect) -> Result<FlashConfig, SmcError> {
        self.inner.cs_config(cs)
    }
}

impl FmcCalibrated {
    /// Perform a programmed I/O read via the FMC flash window.
    pub fn read(&self, cs: ChipSelect, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        self.inner.read(cs, offset, buf)
    }

    /// Initiate a DMA read operation.
    pub fn dma_read(
        &mut self,
        cs: ChipSelect,
        flash_offset: u32,
        dram_addr: usize,
        len: u32,
    ) -> Result<(), SmcError> {
        self.inner.dma_read(cs, flash_offset, dram_addr, len)
    }

    /// Read raw DMA/interrupt status bits from FMC008.
    pub fn dma_status(&self) -> u32 {
        self.inner.dma_status()
    }

    /// Clear DMA-related status bits in FMC008 (write-1-to-clear).
    pub fn clear_dma_status(&self, clear_mask: u32) {
        self.inner.clear_dma_status(clear_mask)
    }

    /// Handle DMA completion/error from IRQ status and finalize controller state.
    pub fn handle_dma_irq(&mut self) -> Result<SmcInterrupt, SmcError> {
        self.inner.handle_dma_irq()
    }

    /// Poll for DMA completion without requiring an IRQ. See `Smc::poll_dma_completion`.
    pub fn poll_dma_completion(&mut self) -> core::task::Poll<Result<(), SmcError>> {
        self.inner.poll_dma_completion()
    }

    /// Check if FMC is idle and ready for a new operation.
    pub fn is_ready(&self) -> bool {
        self.inner.is_ready()
    }

    #[doc(hidden)]
    pub fn test_force_dma_in_flight(&mut self) {
        self.inner.test_force_dma_in_flight();
    }

    /// Get the controller identifier.
    pub fn controller_id(&self) -> SmcController {
        self.inner.controller_id()
    }

    /// Return configured flash capacity in bytes.
    pub fn capacity_bytes(&self) -> Result<usize, SmcError> {
        self.inner.capacity_bytes()
    }

    /// Return configured flash capacity in bytes for the given chip select.
    pub fn cs_capacity_bytes(&self, cs: ChipSelect) -> Result<usize, SmcError> {
        self.inner.cs_capacity_bytes(cs)
    }

    /// Return the configured `FlashConfig` for the requested chip select.
    pub fn cs_config(&self, cs: ChipSelect) -> Result<FlashConfig, SmcError> {
        self.inner.cs_config(cs)
    }

    /// Execute a raw user-mode SPI transfer on the selected FMC chip select.
    ///
    /// `cs` selects CS0 or CS1; `mode` controls the per-phase IO width.
    /// Returns `SmcError::InvalidChipSelect` if CS1 is requested but not configured.
    pub fn transceive_user(
        &self,
        cs: ChipSelect,
        cmd: &[u8],
        tx_payload: &[u8],
        rx: &mut [u8],
        mode: TransferMode,
    ) -> Result<(), SmcError> {
        self.inner.transceive_user(cs, cmd, tx_payload, rx, mode)
    }

    /// Convenience wrapper: execute a user-mode transfer on CS0.
    pub fn transceive_user_cs0(
        &self,
        cmd: &[u8],
        tx_payload: &[u8],
        rx: &mut [u8],
        mode: TransferMode,
    ) -> Result<(), SmcError> {
        self.inner
            .transceive_user(ChipSelect::Cs0, cmd, tx_payload, rx, mode)
    }

    /// Access the underlying generic calibrated controller.
    pub fn as_inner(&self) -> &CalibratedSmc {
        &self.inner
    }

    /// Mutable access to the underlying generic calibrated controller.
    pub fn as_inner_mut(&mut self) -> &mut CalibratedSmc {
        &mut self.inner
    }
}
