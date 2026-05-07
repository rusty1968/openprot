// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SMC error-interrupt scaffold.
//!
//! Tests the interrupt-driven error path of the SMC controller.  The test is
//! structured in two tiers:
//!
//! **Tier 1 — QEMU-reachable (runs here)**
//!
//! 1. **Init with interrupts enabled** — construct FMC with
//!    `enable_interrupts: true`; verify `is_ready()`.
//! 2. **Spurious-fire guard** — call `handle_dma_irq()` with no DMA in flight
//!    and no status bits set (QEMU FMC008 reads 0).  Expect
//!    `Err(SmcError::ControllerNotReady)` — the controller must not transition
//!    state or corrupt internal bookkeeping on a spurious IRQ.
//! 3. **Spurious-fire leaves state Ready** — after step 2, assert
//!    `is_ready()` is still true.
//!
//! **Tier 2 — Hardware-only (marked TODO)**
//!
//! The following require real FMC hardware or a register-level QEMU model that
//! drives FMC008 status bits:
//!
//! - Write-protect error (bit 9): assert WP, issue write, await IRQ,
//!   expect `Err(SmcError::WriteProtected)`.
//! - DMA-abort error (bit 10, DMA in-flight): start DMA to bad address,
//!   await IRQ, expect `Err(SmcError::DmaAborted)`.
//! - Command-abort error (bit 10, no DMA): issue incompatible-mode command,
//!   await IRQ, expect `Err(SmcError::HardwareError)`.
//! - DMA complete (bit 11): start valid DMA, await IRQ,
//!   expect `Ok(SmcInterrupt::DmaComplete)`.
//!
//! See `planning/irq-enablement-plan.md` for the full flow description and
//! decision on PIO vs DMA interrupt scope.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{FlashConfig, FmcUninit, SmcConfig, SmcController, SmcError, SmcTopology};
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

fn run_error_irq_test() -> Result<(), SmcError> {
    // --- 1. Init with interrupts enabled ---
    //
    // `enable_interrupts: true` causes `dma_read` to call `enable_dma_irq()`
    // (sets FMC008 `dmaintenbl`) before transitioning to DmaInFlight.
    // It does NOT unmask NVIC — that is the caller's responsibility.
    let config = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(FLASH_CFG),
        cs1: None,
        dma_enabled: true,
        enable_interrupts: true,
        topology: SmcTopology::BootSpi { master_idx: 0 },
    };

    let uninit = unsafe { FmcUninit::new(config)? };
    let mut fmc = uninit.init()?;

    if !fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // --- 2. Spurious-fire guard ---
    //
    // No DMA is in-flight and FMC008 status bits are 0 (QEMU default).
    // `handle_dma_irq` must disable the interrupt, read status, find no
    // relevant bits, and return ControllerNotReady without changing state.
    match fmc.handle_dma_irq() {
        Err(SmcError::ControllerNotReady) => {}
        Err(other) => return Err(other),
        Ok(_) => return Err(SmcError::HardwareError),
    }

    // --- 3. Spurious-fire leaves controller Ready ---
    if !fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // TODO(hardware): Write-protect error (bit 9)
    //   Steps:
    //     1. Disable write-enable on CS0 (clear CONF_ENABLE_W0 in FMC000).
    //     2. Issue a write command via transceive_user (WREN + PP).
    //     3. Unmask NVIC for IRQ 39 (Interrupt::fmc).
    //     4. Await IRQ; call handle_dma_irq() from ISR context.
    //   Expected: Err(SmcError::WriteProtected), state → Error.

    // TODO(hardware): DMA-abort error (bit 10, DMA in-flight)
    //   Steps:
    //     1. Start dma_read to an address that will trigger a hardware abort.
    //     2. Unmask NVIC for IRQ 39.
    //     3. Await IRQ; call handle_dma_irq() from ISR context.
    //   Expected: Err(SmcError::DmaAborted), state → Ready.

    // TODO(hardware): Command-abort error (bit 10, no DMA)
    //   Steps:
    //     1. Issue an incompatible-mode command outside of DMA.
    //     2. Await IRQ; call handle_dma_irq() from ISR context.
    //   Expected: Err(SmcError::HardwareError), state → Error.

    // TODO(hardware): DMA complete (bit 11)
    //   Steps:
    //     1. Start a valid dma_read.
    //     2. Unmask NVIC for IRQ 39.
    //     3. Await IRQ; call handle_dma_irq() from ISR context.
    //   Expected: Ok(SmcInterrupt::DmaComplete), state → Ready.

    Ok(())
}

declare_target!(Target);

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SMC Error IRQ Scaffold";

    fn main() -> ! {
        let exit_status = match run_error_irq_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}
