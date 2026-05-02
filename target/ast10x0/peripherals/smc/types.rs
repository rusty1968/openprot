// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Type definitions and error handling

use embedded_storage::nor_flash::{NorFlashError, NorFlashErrorKind};

/// Terminal errors: operation failed, don't retry
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcError {
    /// Hardware error
    HardwareError,
    /// Timeout waiting for operation
    Timeout,
    /// DMA transfer aborted
    DmaAborted,
    /// DMA length doesn't match expected
    DmaLengthMismatch,
    /// Invalid chip select number
    InvalidChipSelect,
    /// Invalid or unsupported capacity
    InvalidCapacity,
    /// Device not supported
    DeviceNotSupported,
    /// Flash is write-protected
    WriteProtected,
    /// Write operation in progress
    WriteInProgress,
}

impl NorFlashError for SmcError {
    fn kind(&self) -> NorFlashErrorKind {
        NorFlashErrorKind::Other
    }
}

/// Retryable errors (returned as WouldBlock in nb::Result)
#[derive(Clone, Copy, Debug)]
pub enum SmcRetryable {
    /// Controller not ready
    NotReady,
    /// DMA transfer still in-flight
    DmaTransferPending,
}

impl From<SmcRetryable> for nb::Error<SmcError> {
    fn from(_: SmcRetryable) -> Self {
        nb::Error::WouldBlock
    }
}

/// SMC Controller identifier
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcController {
    /// Firmware Memory Controller (boot flash, primary)
    Fmc = 0,
    /// SPI Flash Controller 1 (secondary)
    Spi1 = 1,
    /// SPI Flash Controller 2 (tertiary)
    Spi2 = 2,
}

impl SmcController {
    /// Get the base hardware address for this controller
    pub fn base_address(&self) -> usize {
        match self {
            Self::Fmc => 0x7E620000,
            Self::Spi1 => 0x7E630000,
            Self::Spi2 => 0x7E640000,
        }
    }

    /// Get the memory-mapped flash window address
    pub fn flash_window_address(&self) -> usize {
        match self {
            Self::Fmc => 0x80000000,
            Self::Spi1 => 0x90000000,
            Self::Spi2 => 0xB0000000,
        }
    }

    /// Get the IRQ vector number
    pub fn irq_number(&self) -> u32 {
        match self {
            Self::Fmc => 39,
            Self::Spi1 => 65,
            Self::Spi2 => 66,
        }
    }
}

/// Configuration for a single flash device
#[derive(Clone, Copy, Debug)]
pub struct FlashConfig {
    /// Device capacity in MB
    pub capacity_mb: u32,
    /// Page size in bytes (typically 256)
    pub page_size: u32,
    /// Sector size in bytes (typically 4096)
    pub sector_size: u32,
    /// Block size in bytes (typically 65536)
    pub block_size: u32,
    /// Desired SPI clock frequency in MHz
    pub spi_clock_mhz: u32,
}

impl FlashConfig {
    /// Winbond W25Q64 (8 MB) configuration
    pub const fn winbond_w25q64() -> Self {
        Self {
            capacity_mb: 8,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 25,
        }
    }

    /// Winbond W25Q256 (32 MB) configuration
    pub const fn winbond_w25q256() -> Self {
        Self {
            capacity_mb: 32,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 25,
        }
    }
}

/// Per-controller configuration
#[derive(Clone, Copy, Debug)]
pub struct SmcConfig {
    /// Which controller to configure
    pub controller_id: SmcController,
    /// Optional configuration for CS0 flash device
    pub cs0: Option<FlashConfig>,
    /// Optional configuration for CS1 flash device
    pub cs1: Option<FlashConfig>,
    /// Enable DMA transfers
    pub dma_enabled: bool,
    /// Enable interrupt handlers
    pub enable_interrupts: bool,
}
