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
    /// Controller is not in the Ready state; call was made at wrong lifecycle stage
    ControllerNotReady,
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

/// Address width used when assembling SPI NOR command byte streams.
///
/// Matches aspeed-rust's `AddressWidth` enum. The device layer uses this when
/// building opcode+address slices for `transceive_user`, ensuring the correct
/// number of address bytes is emitted rather than relying on implicit slicing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressWidth {
    /// No address bytes (e.g. WREN, RDSR).
    None,
    /// 24-bit (3-byte) address — standard for ≤16 MB devices.
    ThreeByte,
    /// 32-bit (4-byte) address — required for >16 MB devices.
    FourByte,
}

impl AddressWidth {
    /// Number of address bytes this variant represents.
    #[must_use]
    pub const fn byte_count(self) -> usize {
        match self {
            Self::None => 0,
            Self::ThreeByte => 3,
            Self::FourByte => 4,
        }
    }
}
///
/// Matches the mode naming convention used by JESD216 and aspeed-rust.
/// The IO mode bits written to the CS control register per phase are derived
/// from this value by `TransferMode::cmd_io_bits()`, `addr_io_bits()`, and
/// `data_io_bits()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferMode {
    /// Single-wire command, address, and data (standard SPI)
    Mode111,
    /// Single-wire command and address, dual-wire data
    Mode112,
    /// Single-wire command, dual-wire address and data
    Mode122,
    /// Single-wire command and address, quad-wire data
    Mode114,
    /// Single-wire command, quad-wire address and data
    Mode144,
}

impl TransferMode {
    /// IO mode bits for the command phase (bits [29:28] of CS ctrl register).
    #[must_use]
    pub const fn cmd_io_bits(self) -> u32 {
        // Command phase is always single-wire across all supported modes.
        0x0000_0000
    }

    /// IO mode bits for the address phase.
    #[must_use]
    pub const fn addr_io_bits(self) -> u32 {
        match self {
            Self::Mode111 | Self::Mode112 | Self::Mode114 => 0x0000_0000,
            Self::Mode122 | Self::Mode144 => 0x2000_0000,
        }
    }

    /// IO mode bits for the data phase.
    #[must_use]
    pub const fn data_io_bits(self) -> u32 {
        match self {
            Self::Mode111 => 0x0000_0000,
            Self::Mode112 | Self::Mode122 => 0x2000_0000,
            Self::Mode114 | Self::Mode144 => 0x4000_0000,
        }
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
