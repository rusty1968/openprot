// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 Flash Service Target

#![no_std]
#![no_main]

use ast10x0_board_descriptors::{apply_spim_wiring, presets, SpimWiring};
use ast10x0_peripherals::scu::ScuRegisters;
use ast10x0_peripherals::smc::SmcController;
use ast10x0_peripherals::spimonitor::MonitorPolicy;
use cortex_m_semihosting::debug::{EXIT_FAILURE, EXIT_SUCCESS, exit};
use target_common::{TargetInterface, declare_target};
use {console_backend as _, entry as _};

// Stored as a static so its ~548-byte body lives in `.rodata` rather than
// being copied onto the kernel bootstrap thread's stack.
static BMC_POLICY: MonitorPolicy = presets::bmc_default_policy();

pub struct Target {}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 Flash Service";

    fn main() -> ! {
        // Trusted SPIM setup before user-space apps spawn. Programs SCU
        // routing, SPIPF allow-list, and the one-way SPIPF lock for SPI1
        // (via SPIM0) and SPI2 (via SPIM2). Doing this in privileged
        // kernel context keeps SPI server MPU domains free of SCU/SPIPF
        // access. The returned `LockedSpiMonitor` witnesses are dropped;
        // the lock is one-way per silicon spec and persists.
        // SAFETY: kernel `main` runs before any user-space process is
        // started, so we have exclusive access to SCU and SPIPF.
        let scu = unsafe { ScuRegisters::new_global() };

        // SAFETY: see above.
        if unsafe {
            apply_spim_wiring(
                &scu,
                SmcController::Spi1,
                SpimWiring::default_spi1_via_spim0(),
                &BMC_POLICY,
            )
        }
        .is_err()
        {
            exit(EXIT_FAILURE);
        }

        // SAFETY: see above.
        if unsafe {
            apply_spim_wiring(
                &scu,
                SmcController::Spi2,
                SpimWiring::default_spi2_via_spim2(),
                &BMC_POLICY,
            )
        }
        .is_err()
        {
            exit(EXIT_FAILURE);
        }

        codegen::start();
        #[expect(clippy::empty_loop)]
        loop {}
    }

    fn shutdown(code: u32) -> ! {
        let status = if code == 0 { EXIT_SUCCESS } else { EXIT_FAILURE };
        exit(status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
