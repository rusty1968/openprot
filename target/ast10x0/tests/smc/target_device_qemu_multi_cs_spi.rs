// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SPI1/SPI2 multi-CS coverage tests (QEMU-only).
//!
//! Extends the FMC multi-CS suite to the SPI wrapper paths. This verifies:
//!
//! 1. `transceive_user` succeeds on CS0 and CS1 for both SPI1 and SPI2 when
//!    both chip selects are configured.
//! 2. `transceive_user(Cs1, ..)` returns `InvalidChipSelect` when CS1 is not
//!    configured on SPI1/SPI2.
//! 3. `SpiNorFlash::from_spi_cs` validates `FlashConfig` against the selected
//!    CS for both SPI1 and SPI2.
//!
//! The test intentionally avoids asserting on returned byte values for CS1,
//! since that is model-dependent in QEMU. The point here is peripheral-layer
//! routing and CS-specific validation on the SPI wrappers.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, SmcConfig, SmcController, SmcError, SpiNorFlash,
    SpiNorFlashDevice, SpiReady, SpiUninit, TransferMode,
};
use cortex_m_semihosting::debug::{exit, EXIT_FAILURE, EXIT_SUCCESS};
use target_common::{declare_target, TargetInterface};
use {console_backend as _, entry as _};

pub struct Target {}

const CS0_CFG: FlashConfig = FlashConfig {
    capacity_mb: 2,
    page_size: 256,
    sector_size: 4096,
    block_size: 65536,
    spi_clock_mhz: 25,
};

const CS1_CFG: FlashConfig = FlashConfig {
    capacity_mb: 1,
    page_size: 256,
    sector_size: 4096,
    block_size: 65536,
    spi_clock_mhz: 25,
};

const CMD_READ_STATUS: u8 = 0x05;

fn init_spi(
    controller_id: SmcController,
    cs0: Option<FlashConfig>,
    cs1: Option<FlashConfig>,
) -> Result<SpiReady, SmcError> {
    let config = SmcConfig {
        controller_id,
        cs0,
        cs1,
        dma_enabled: false,
        enable_interrupts: false,
    };

    let uninit = unsafe { SpiUninit::new(controller_id, config)? };
    uninit.init()
}

fn run_controller_multi_cs_test(controller_id: SmcController) -> Result<(), SmcError> {
    let mut spi = init_spi(controller_id, Some(CS0_CFG), Some(CS1_CFG))?;

    if !spi.is_ready() {
        return Err(SmcError::HardwareError);
    }

    let mut sr_cs0 = [0u8; 1];
    spi.transceive_user(
        ChipSelect::Cs0,
        &[CMD_READ_STATUS],
        &[],
        &mut sr_cs0,
        TransferMode::Mode111,
    )?;

    let mut sr_cs1 = [0u8; 1];
    spi.transceive_user(
        ChipSelect::Cs1,
        &[CMD_READ_STATUS],
        &[],
        &mut sr_cs1,
        TransferMode::Mode111,
    )?;

    let spi_cs0 = SpiNorFlash::from_spi_cs(&mut spi, CS0_CFG, ChipSelect::Cs0)?;
    let _ = spi_cs0.status()?;

    match SpiNorFlash::from_spi_cs(&mut spi, CS0_CFG, ChipSelect::Cs1) {
        Err(SmcError::InvalidCapacity) => {}
        _ => return Err(SmcError::HardwareError),
    }

    let spi_cs1 = SpiNorFlash::from_spi_cs(&mut spi, CS1_CFG, ChipSelect::Cs1)?;
    let _ = spi_cs1.status()?;

    let spi_cs0_only = init_spi(controller_id, Some(CS0_CFG), None)?;
    match spi_cs0_only.transceive_user(
        ChipSelect::Cs1,
        &[CMD_READ_STATUS],
        &[],
        &mut [0u8; 1],
        TransferMode::Mode111,
    ) {
        Err(SmcError::InvalidChipSelect) => {}
        _ => return Err(SmcError::HardwareError),
    }

    Ok(())
}

fn run_multi_cs_spi_test() -> Result<(), SmcError> {
    run_controller_multi_cs_test(SmcController::Spi1)?;
    run_controller_multi_cs_test(SmcController::Spi2)?;
    Ok(())
}

declare_target!(Target);

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SPI Multi-CS Wrapper Test";

    fn main() -> ! {
        let exit_status = match run_multi_cs_spi_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}