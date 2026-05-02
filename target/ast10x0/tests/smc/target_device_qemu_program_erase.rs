// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SpiNorFlash device facade — QEMU program/erase integration test.
//!
//! This test validates write path behavior end-to-end on QEMU's volatile flash
//! model for the FMC-backed facade:
//! 1. Program one page at a sector-aligned offset.
//! 2. Verify programmed bytes.
//! 3. Erase the containing sector.
//! 4. Verify erased bytes (0xFF).

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    FlashConfig, FlashDevice, FmcUninit, SmcConfig, SmcController, SmcError, SpiNorFlash,
};
use cortex_m_semihosting::debug::{EXIT_FAILURE, EXIT_SUCCESS, exit};
use target_common::{TargetInterface, declare_target};
use {console_backend as _, entry as _};

pub struct Target {}

const FLASH_CFG: FlashConfig = FlashConfig {
    capacity_mb: 1,
    page_size: 256,
    sector_size: 4096,
    block_size: 65536,
    spi_clock_mhz: 25,
};

fn run_device_program_erase_test() -> Result<(), SmcError> {
    let config = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(FLASH_CFG),
        cs1: None,
        dma_enabled: false,
        enable_interrupts: false,
    };

    let uninit = unsafe { FmcUninit::new(config)? };
    let mut fmc = uninit.init()?;

    if !fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    let mut flash = SpiNorFlash::from_fmc(&mut fmc, FLASH_CFG)?;

    let test_offset = 0x0000_1000u32;
    let mut page = [0u8; 256];
    for (i, byte) in page.iter_mut().enumerate() {
        *byte = (i as u8) ^ 0xA5;
    }

    let written = flash.program_page(test_offset, &page)?;
    if written != page.len() {
        return Err(SmcError::HardwareError);
    }

    if !flash.verify(test_offset, &page)? {
        return Err(SmcError::HardwareError);
    }

    flash.erase_sector(test_offset)?;

    let erased = [0xFFu8; 256];
    if !flash.verify(test_offset, &erased)? {
        return Err(SmcError::HardwareError);
    }

    Ok(())
}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SpiNorFlash QEMU Program/Erase Test";

    fn main() -> ! {
        let exit_status = match run_device_program_erase_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_e) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);