// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Static Memory Controller (SMC) HAL
//!
//! Provides safe abstractions over the FMC (Firmware Memory Controller)
//! and SPI1/SPI2 flash controllers.

pub mod fmc_backend;
pub mod register_traits;
pub mod spi_backend;
pub mod types;
mod helpers;
pub mod controller;
pub mod fmc;
pub mod spi;
pub mod interrupts;
pub mod device;

pub use types::{SmcError, SmcController, ChipSelect, FlashConfig, SmcConfig, SmcRetryable, TransferMode, AddressWidth};
pub use register_traits::SmcRegisterBackend;
pub use fmc_backend::FmcRegisterBackend;
pub use spi_backend::SpiRegisterBackend;
pub use controller::{Ready, ReadySmc, Smc, UninitSmc, Uninitialized};
pub use fmc::{FmcReady, FmcUninit};
pub use spi::{SpiReady, SpiUninit};
pub use interrupts::{SmcInterrupt, SmcInterruptDecoder};
pub use device::{
	BlockDeviceInfo, FlashAddressingPolicy, FlashCommandProfile, JedecId, SpiNorBlockDevice,
	SpiNorFlash, SpiNorFlashDevice,
};

/// Result type for SMC operations
pub type Result<T> = core::result::Result<T, SmcError>;
