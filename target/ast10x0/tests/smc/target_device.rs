// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SpiNorFlash device facade — portable smoke test.
//!
//! Safe to run on both QEMU and silicon.  Tests the device layer facade
//! (`SpiNorFlash`) built on top of the FMC wrapper.  Flash content is not
//! inspected so this is board-agnostic.
//!
//! Tests (in order):
//!
//! 1. **Init** — construct FmcUninit, initialize hardware, assert Ready.
//! 2. **from_fmc** — build SpiNorFlash from FmcReady.
//! 3. **capacity_bytes** — assert returns 1 MB (matches FlashConfig).
//! 4. **status** — issue `RDSR` and assert the command path succeeds.
//! 5. **read via facade — success path** — read 8 bytes from offset 0;
//!    assert count is returned correctly.  Content not inspected.
//! 6. **read via facade — bounds rejection** — assert InvalidCapacity before
//!    any MMIO for an out-of-range read.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    FlashConfig, SpiNorFlashDevice, FmcUninit, SmcConfig, SmcController, SmcError, SmcTopology, SpiNorFlash,
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

fn run_device_smoke_test() -> Result<(), SmcError> {
    // --- 1. Init ---
    let config = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(FLASH_CFG),
        cs1: None,
        dma_enabled: false,
        enable_interrupts: false,
        topology: SmcTopology::BootSpi { master_idx: 0 },
    };

    let uninit = unsafe { FmcUninit::new(config)? };
    let mut fmc = uninit.init()?;

    if !fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // --- 2. Build SpiNorFlash facade from FmcReady ---
    let flash = SpiNorFlash::from_fmc(&mut fmc, FLASH_CFG)?;

    // --- 3. capacity_bytes ---
    let cap = flash.capacity_bytes()?;
    if cap != 1 * 1024 * 1024 {
        return Err(SmcError::HardwareError);
    }

    // --- 4. status ---
    let _ = flash.status()?;

    // --- 5. read via facade — success path ---
    let mut buf = [0u8; 8];
    let n = flash.read(0, &mut buf)?;
    if n != 8 {
        return Err(SmcError::HardwareError);
    }

    // --- 6. read via facade — bounds rejection ---
    // 1 MB = 0x10_0000 bytes; offset 0x000F_FFFF + 8 bytes overflows.
    let mut overflow_buf = [0u8; 8];
    match flash.read(0x000F_FFFF, &mut overflow_buf) {
        Err(SmcError::InvalidCapacity) => {}
        Err(other) => return Err(other),
        Ok(_) => return Err(SmcError::HardwareError),
    }

    Ok(())
}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SpiNorFlash Device Smoke Test";

    fn main() -> ! {
        let exit_status = match run_device_smoke_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_e) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
