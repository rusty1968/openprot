// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SpiNorFlash device facade — per-CS capacity validation (DEV-PAR-001).
//!
//! Verifies that `SpiNorFlash::from_fmc_cs` validates the supplied
//! `FlashConfig.capacity_mb` against the *selected CS*'s configured capacity,
//! not against the controller-total capacity.
//!
//! QEMU caveat: the `ast1030-evb` model attaches a single `w25q80bl` chip on
//! CS0 of the FMC controller. CS1 has no backing device. This test never
//! programs/erases/reads — it only exercises the constructor's capacity
//! check, which is HAL-side and does not touch the chip.
//!
//! Three assertions:
//! 1. `from_fmc_cs(.., CS0_CFG, Cs0)` succeeds when CS0 cfg matches CS0
//!    configured capacity (different from CS0+CS1 total).
//! 2. `from_fmc_cs(.., CS0_CFG, Cs1)` returns `InvalidCapacity` because
//!    `CS0_CFG.capacity_mb` ≠ `CS1_CFG.capacity_mb`.
//! 3. `from_fmc_cs(.., CS1_CFG, Cs1)` succeeds when CS1 cfg matches CS1
//!    configured capacity.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, FmcUninit, SmcConfig, SmcController, SmcError, SmcTopology, SpiNorFlash,
};
use cortex_m_semihosting::debug::{EXIT_FAILURE, EXIT_SUCCESS, exit};
use target_common::{TargetInterface, declare_target};
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

fn run_per_cs_capacity_test() -> Result<(), SmcError> {
    let config = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(CS0_CFG),
        cs1: Some(CS1_CFG),
        dma_enabled: false,
        enable_interrupts: false,
        topology: SmcTopology::BootSpi { master_idx: 0 },
    };

    let uninit = unsafe { FmcUninit::new(config)? };
    let mut fmc = uninit.init()?;

    if !fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // Assertion 1: CS0 cfg + CS0 selector — must succeed even though
    // CS0_CFG.capacity_mb (2) differs from controller total (CS0+CS1 = 3 MB).
    {
        let result = SpiNorFlash::from_fmc_cs(&mut fmc, CS0_CFG, ChipSelect::Cs0);
        if result.is_err() {
            return Err(SmcError::HardwareError);
        }
    }

    // Assertion 2: CS0 cfg + CS1 selector — must reject with InvalidCapacity
    // because CS0_CFG.capacity_mb (2) ≠ CS1_CFG.capacity_mb (1).
    {
        match SpiNorFlash::from_fmc_cs(&mut fmc, CS0_CFG, ChipSelect::Cs1) {
            Err(SmcError::InvalidCapacity) => {}
            Ok(_) => return Err(SmcError::HardwareError),
            Err(_) => return Err(SmcError::HardwareError),
        }
    }

    // Assertion 3: CS1 cfg + CS1 selector — must succeed; CS1_CFG matches CS1.
    {
        let result = SpiNorFlash::from_fmc_cs(&mut fmc, CS1_CFG, ChipSelect::Cs1);
        if result.is_err() {
            return Err(SmcError::HardwareError);
        }
    }

    Ok(())
}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SpiNorFlash QEMU Per-CS Capacity Test";

    fn main() -> ! {
        let exit_status = match run_per_cs_capacity_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_e) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
