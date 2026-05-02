// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SMC multi-CS command routing tests (QEMU-only).
//!
//! Verifies that `transceive_user` correctly dispatches to CS0 and CS1
//! independently, and that requesting CS1 on a CS1-unconfigured controller
//! returns `SmcError::InvalidChipSelect`.
//!
//! QEMU provides a single `w25q80bl` model on CS0 of the FMC controller.
//! CS1 is configured in the HAL but the QEMU model does not attach a device
//! there — reads return undefined bytes.  The test only asserts that the
//! transport completes without error (CS line toggled, no hardware fault),
//! mirroring aspeed-rust's treatment of CS availability as a HAL concern
//! rather than a device-presence check.
//!
//! Tests (in order):
//!
//! 1. **Init** — construct FMC with cs0 + cs1 configured, run init.
//! 2. **RDSR CS0** — read status register via CS0; assert command succeeds.
//! 3. **RDSR CS1** — read status register via CS1; assert command succeeds
//!    (byte value not asserted; QEMU may return 0x00 or undefined).
//! 4. **InvalidChipSelect guard** — build a CS1-unconfigured controller;
//!    assert `transceive_user(Cs1, …)` returns `InvalidChipSelect`.
//! 5. **TransferMode differential check** — run `RDSR` on CS0 for each
//!    supported mode (`111/112/122/114/144`) and assert transport completes.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, FmcUninit, SmcConfig, SmcController, SmcError, TransferMode,
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

const CMD_READ_STATUS: u8 = 0x05;

fn run_multi_cs_test() -> Result<(), SmcError> {
    // --- 1. Init with cs0 + cs1 both configured ---
    let config = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(FLASH_CFG),
        cs1: Some(FLASH_CFG),
        dma_enabled: false,
        enable_interrupts: false,
    };

    let uninit = unsafe { FmcUninit::new(config)? };
    let fmc = uninit.init()?;

    if !fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // --- 2. RDSR on CS0 — must succeed ---
    let mut sr_cs0 = [0u8; 1];
    fmc.transceive_user(
        ChipSelect::Cs0,
        &[CMD_READ_STATUS],
        &[],
        &mut sr_cs0,
        TransferMode::Mode111,
    )?;

    // --- 3. RDSR on CS1 — must complete without transport error ---
    let mut sr_cs1 = [0u8; 1];
    fmc.transceive_user(
        ChipSelect::Cs1,
        &[CMD_READ_STATUS],
        &[],
        &mut sr_cs1,
        TransferMode::Mode111,
    )?;

    // --- 4. InvalidChipSelect guard: CS1-unconfigured controller ---
    let config_cs0_only = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(FLASH_CFG),
        cs1: None,
        dma_enabled: false,
        enable_interrupts: false,
    };
    let uninit2 = unsafe { FmcUninit::new(config_cs0_only)? };
    let fmc2 = uninit2.init()?;

    match fmc2.transceive_user(
        ChipSelect::Cs1,
        &[CMD_READ_STATUS],
        &[],
        &mut [0u8; 1],
        TransferMode::Mode111,
    ) {
        Err(SmcError::InvalidChipSelect) => {}
        other => {
            let _ = other;
            return Err(SmcError::HardwareError);
        }
    }

    // --- 5. TransferMode differential: all supported mode variants ---
    //
    // QEMU data semantics for dual/quad are model-dependent; this check focuses
    // on transport-path stability and error-free per-phase register sequencing.
    let all_modes = [
        TransferMode::Mode111,
        TransferMode::Mode112,
        TransferMode::Mode122,
        TransferMode::Mode114,
        TransferMode::Mode144,
    ];
    for mode in all_modes {
        let mut sr = [0u8; 1];
        fmc.transceive_user(ChipSelect::Cs0, &[CMD_READ_STATUS], &[], &mut sr, mode)?;
    }

    Ok(())
}

declare_target!(Target);

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SMC Multi-CS Command Routing Test";

    fn main() -> ! {
        let exit_status = match run_multi_cs_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}
