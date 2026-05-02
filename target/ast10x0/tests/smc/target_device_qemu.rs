// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SpiNorFlash device facade — QEMU-only erase-state test.
//!
//! Relies on QEMU-specific behaviour: the volatile `w25q80bl` model always
//! boots in the fully-erased state, so every byte in the flash window reads as
//! 0xFF.  Not safe to assert on silicon where flash is pre-programmed.
//!
//! Tagged "integration" in BUILD.bazel.  Invoke explicitly:
//!
//!   bazelisk test --config=virt_ast10x0 --test_tag_filters= \
//!     //target/ast10x0/tests/smc:smc_device_qemu_erase_state_test
//!
//! Tests (in order):
//!
//! 1. **Init** — construct FmcUninit, initialize, assert Ready.
//! 2. **from_fmc** — build SpiNorFlash facade.
//! 3. **capacity_bytes** — assert 1 MB.
//! 4. **status** — issue `RDSR` and assert the command path succeeds.
//! 5. **read via facade — erase-state** — read 8 bytes from offset 0 and
//!    assert every byte is 0xFF, confirming the full path from facade through
//!    FmcReady → ReadySmc → flash window → m25p80 model.
//! 6. **read via facade — bounds rejection** — assert InvalidCapacity.
//! 7. **from_spi path** — initialize SPI1, build facade with `from_spi`, then
//!    validate capacity/read/bounds behavior for the SPI constructor path.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    FlashConfig, FlashDevice, FmcUninit, SmcConfig, SmcController, SmcError, SpiNorFlash,
    SpiUninit,
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

fn run_device_qemu_test() -> Result<(), SmcError> {
    // --- 1. Init ---
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

    // --- 2. Build SpiNorFlash facade ---
    let flash = SpiNorFlash::from_fmc(&mut fmc, FLASH_CFG)?;

    // --- 3. capacity_bytes ---
    let cap = flash.capacity_bytes()?;
    if cap != 1 * 1024 * 1024 {
        return Err(SmcError::HardwareError);
    }

    // --- 4. status ---
    let _ = flash.status()?;

    // --- 5. read via facade — erase-state check (QEMU-specific) ---
    // QEMU's volatile w25q80bl returns 0xFF on every byte before any program
    // cycle.  Confirms: facade → FmcReady → flash window (0x8000_0000) → m25p80.
    let mut buf = [0u8; 8];
    let n = flash.read(0, &mut buf)?;
    if n != 8 {
        return Err(SmcError::HardwareError);
    }
    for byte in buf.iter() {
        if *byte != 0xFF {
            return Err(SmcError::HardwareError);
        }
    }

    // --- 6. read via facade — bounds rejection ---
    let mut overflow_buf = [0u8; 8];
    match flash.read(0x000F_FFFF, &mut overflow_buf) {
        Err(SmcError::InvalidCapacity) => {}
        Err(other) => return Err(other),
        Ok(_) => return Err(SmcError::HardwareError),
    }

    // --- 7. from_spi path (SPI1) ---
    let spi_cfg = SmcConfig {
        controller_id: SmcController::Spi1,
        cs0: Some(FLASH_CFG),
        cs1: None,
        dma_enabled: false,
        enable_interrupts: false,
    };

    let spi_uninit = unsafe { SpiUninit::new(SmcController::Spi1, spi_cfg)? };
    let mut spi = spi_uninit.init()?;

    if !spi.is_ready() {
        return Err(SmcError::HardwareError);
    }

    let spi_flash = SpiNorFlash::from_spi(&mut spi, FLASH_CFG)?;
    let spi_cap = spi_flash.capacity_bytes()?;
    if spi_cap != 1 * 1024 * 1024 {
        return Err(SmcError::HardwareError);
    }

    let _ = spi_flash.status()?;

    let mut spi_buf = [0u8; 8];
    let spi_n = spi_flash.read(0, &mut spi_buf)?;
    if spi_n != 8 {
        return Err(SmcError::HardwareError);
    }

    let mut spi_overflow = [0u8; 8];
    match spi_flash.read(0x000F_FFFF, &mut spi_overflow) {
        Err(SmcError::InvalidCapacity) => {}
        Err(other) => return Err(other),
        Ok(_) => return Err(SmcError::HardwareError),
    }

    Ok(())
}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SpiNorFlash QEMU Erase-State Test";

    fn main() -> ! {
        let exit_status = match run_device_qemu_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_e) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
