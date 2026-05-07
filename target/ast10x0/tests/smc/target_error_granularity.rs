// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SMC error-granularity negative tests.
//!
//! Verifies that operations attempted while the controller is not in the
//! `Ready` state return `SmcError::ControllerNotReady` rather than the generic
//! `SmcError::HardwareError`, making transport-state errors distinguishable
//! from hardware faults in diagnostic output.
//!
//! Strategy: `dma_read` — a non-blocking DMA kick-off — transitions the
//! internal controller state to `DmaInFlight` and returns `Ok(())`.  Any
//! subsequent call to `transceive_user` or `dma_read` must find the state
//! non-ready and immediately return `Err(SmcError::ControllerNotReady)`.
//!
//! Tests (in order):
//!
//! 1. **Init** — construct FMC controller, run hardware init, assert Ready.
//! 2. **DMA kick-off** — issue a valid `dma_read` so state → `DmaInFlight`.
//! 3. **`transceive_user` while not-ready** — assert `ControllerNotReady`.
//! 4. **`dma_read` while not-ready** — assert `ControllerNotReady`.
//! 5. **`dma_read` args-validation bypass check** — confirm invalid args
//!    also return `ControllerNotReady` (not any args-error) when not-ready,
//!    since state is checked before arg validation.
//! 6. **error precedence check** — with controller non-ready and invalid CS
//!    (`Cs1` while `cs1: None`), assert `ControllerNotReady` wins over
//!    `InvalidChipSelect`.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, FmcUninit, SmcConfig, SmcController, SmcError, SmcTopology, TransferMode,
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

/// DRAM address used to satisfy `validate_dma_read` alignment requirements.
///
/// Must be 4-byte aligned and fall within the hardware DMA_DRAM_MASK
/// (`0x000BFFFC`).  `0x0008_0000` is the value used by the controller unit
/// tests for the same purpose.
const DMA_DRAM_ADDR: usize = 0x0008_0000;

fn run_error_granularity_test() -> Result<(), SmcError> {
    // --- 1. Init ---
    let config = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(FLASH_CFG),
        cs1: None,
        dma_enabled: true,
        enable_interrupts: false,
        topology: SmcTopology::BootSpi { master_idx: 0 },
    };

    let uninit = unsafe { FmcUninit::new(config)? };
    let mut fmc = uninit.init()?;

    if !fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // --- 2. DMA kick-off: transitions state → DmaInFlight ---
    //
    // `dma_read` writes DMA registers and marks state DmaInFlight.  QEMU does
    // not model DMA completion, so the controller stays non-ready for the
    // remainder of this test — which is exactly what we need.
    fmc.dma_read(0, DMA_DRAM_ADDR, 256)?;

    // Controller should no longer report Ready.
    if fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // --- 3. transceive_user while not-ready → ControllerNotReady ---
    let cmd = [0x05u8]; // RDSR — arbitrary; we expect rejection before execution
    let mut rx = [0u8; 1];
    match fmc.transceive_user(ChipSelect::Cs0, &cmd, &[], &mut rx, TransferMode::Mode111) {
        Err(SmcError::ControllerNotReady) => {}
        other => {
            let _ = other;
            return Err(SmcError::HardwareError);
        }
    }

    // --- 4. dma_read while not-ready → ControllerNotReady ---
    match fmc.dma_read(0, DMA_DRAM_ADDR, 256) {
        Err(SmcError::ControllerNotReady) => {}
        other => {
            let _ = other;
            return Err(SmcError::HardwareError);
        }
    }

    // --- 5. dma_read with invalid args while not-ready → ControllerNotReady ---
    //
    // State is checked before arg validation, so even a bad DRAM address
    // should yield ControllerNotReady, not InvalidCapacity.
    match fmc.dma_read(0, 0x1000_0000 /* outside DMA mask */, 256) {
        Err(SmcError::ControllerNotReady) => {}
        other => {
            let _ = other;
            return Err(SmcError::HardwareError);
        }
    }

    // --- 6. precedence: not-ready beats invalid chip-select ---
    //
    // This controller was configured with cs1: None, so Cs1 is invalid.
    // However, because state is checked first, non-ready must return
    // ControllerNotReady (not InvalidChipSelect).
    match fmc.transceive_user(
        ChipSelect::Cs1,
        &[0x05],
        &[],
        &mut [0u8; 1],
        TransferMode::Mode111,
    ) {
        Err(SmcError::ControllerNotReady) => {}
        other => {
            let _ = other;
            return Err(SmcError::HardwareError);
        }
    }

    Ok(())
}

declare_target!(Target);

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SMC Error Granularity Negative Tests";

    fn main() -> ! {
        let exit_status = match run_error_granularity_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}
