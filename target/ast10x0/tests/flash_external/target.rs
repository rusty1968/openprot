// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 External (host) Flash Service Target
//!
//! Runs the external-flash server as a userspace process. The server owns the
//! SPI1 controller and drives the RoT internal SPI master onto the monitored
//! BMC bus (SPIM0 path) for each operation. Clients reach it over IPC.

#![no_std]
#![no_main]

use ast10x0_board::{apply_spim_master_passthrough, set_bmc_resets};
use ast10x0_peripherals::scu::{
    ScuExtMuxSelect, ScuRegisters, SpiMonitorInstance, SpiMonitorSource,
};
use cortex_m_semihosting::debug::{exit, EXIT_FAILURE, EXIT_SUCCESS};
use target_common::{declare_target, TargetInterface};
use {console_backend as _, entry as _};

pub struct Target {}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 External Flash Service";

    fn main() -> ! {
        // Static SPI1/SPIM0 wiring, applied before any process starts so no task
        // ever needs to touch the shared pinctrl or SPIM routing at setup time
        // (avoids cross-task RMW races on those registers). The bus is left in
        // host-passthrough mode; the server process routes the RoT internal
        // master on per-operation, then tears it down.
        //
        // SAFETY: kernel main() runs once, single-threaded, with exclusive
        // hardware ownership.
        let scu = unsafe { ScuRegisters::new_global_unlocked() };
        apply_spim_master_passthrough(
            &scu,
            SpiMonitorInstance::Spim0,
            SpiMonitorSource::Spi1,
            ScuExtMuxSelect::Mux1,
        );

        // Hold the BMC in reset so the RoT can safely master its flash bus. This
        // stands in for the orchestrator, which owns the host-hold in production
        // and is what the server's `BmcResetGate` observes before allowing I/O.
        let _ = set_bmc_resets(true);

        codegen::start();
        #[expect(clippy::empty_loop)]
        loop {}
    }

    fn shutdown(code: u32) -> ! {
        let status = if code == 0 {
            EXIT_SUCCESS
        } else {
            EXIT_FAILURE
        };
        exit(status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
