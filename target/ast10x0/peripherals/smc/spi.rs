// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! SPI1/SPI2-specialized wrapper around the generic SMC controller.

use crate::smc::controller::{ReadySmc, UninitSmc};
use crate::smc::types::{SmcConfig, SmcController, SmcError};

/// SPI handle before hardware initialization.
pub struct SpiUninit {
    inner: UninitSmc,
}

/// SPI handle after hardware initialization.
pub struct SpiReady {
    inner: ReadySmc,
}

impl SpiUninit {
    /// Construct an uninitialized SPI controller for SPI1 or SPI2.
    ///
    /// # Safety
    /// Caller must ensure unique ownership of the selected SPI hardware block.
    pub unsafe fn new(controller_id: SmcController, mut config: SmcConfig) -> Result<Self, SmcError> {
        match controller_id {
            SmcController::Fmc => return Err(SmcError::InvalidChipSelect),
            SmcController::Spi1 | SmcController::Spi2 => {}
        }

        config.controller_id = controller_id;
        // SAFETY: Caller upholds controller ownership requirements.
        let inner = unsafe { UninitSmc::new(config)? };
        Ok(Self { inner })
    }

    /// Initialize SPI hardware and transition to ready state.
    pub fn init(self) -> Result<SpiReady, SmcError> {
        Ok(SpiReady {
            inner: self.inner.init()?,
        })
    }
}

impl SpiReady {
    /// Perform a programmed I/O read via the SPI flash window.
    pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        self.inner.read(offset, buf)
    }

    /// Initiate a DMA read operation.
    pub fn dma_read(&mut self, flash_offset: u32, dram_addr: usize, len: u32) -> Result<(), SmcError> {
        self.inner.dma_read(flash_offset, dram_addr, len)
    }

    /// Check if SPI controller is ready for operations.
    pub fn is_ready(&self) -> bool {
        self.inner.is_ready()
    }

    /// Return configured flash capacity in bytes.
    pub fn capacity_bytes(&self) -> Result<usize, SmcError> {
        self.inner.capacity_bytes()
    }

    /// Execute a raw user-mode SPI transfer on the SPI controller CS0 aperture.
    pub fn transceive_user(&self, cmd: &[u8], tx_payload: &[u8], rx: &mut [u8]) -> Result<(), SmcError> {
        self.inner.transceive_user(cmd, tx_payload, rx)
    }

    /// Access the underlying generic ready controller.
    pub fn as_inner(&self) -> &ReadySmc {
        &self.inner
    }

    /// Mutable access to the underlying generic ready controller.
    pub fn as_inner_mut(&mut self) -> &mut ReadySmc {
        &mut self.inner
    }
}
