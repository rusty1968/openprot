// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SpiNorFlash device facade — CS-local offset semantics
//! (DEV-PAR-002 + DEV-PAR-003).
//!
//! Verifies that on a multi-CS controller (CS0 = 2 MB, CS1 = 1 MB) the
//! `SpiNorFlash` facade enforces *per-CS* capacity bounds rather than the
//! controller-total bound. Specifically, on a CS1-bound facade:
//!
//! 1. `cs1_facade.read(offset = CS1_CAPACITY, ..)` returns
//!    `InvalidCapacity`. Broken code accepts (1 MB + n < 3 MB total);
//!    fixed code rejects (1 MB + n > 1 MB per-CS).
//! 2. `cs1_facade.program_page(offset = CS1_CAPACITY, ..)` returns
//!    `InvalidCapacity`.
//! 3. `cs1_facade.erase_sector(offset = CS1_CAPACITY)` returns
//!    `InvalidCapacity`.
//!
//! **Addressing-math caveat (deviation from plan §3.5 RED step 2).** The
//! plan also asks for a content-separation assertion (CS1.read(0) ≠
//! CS0.read(0), or the addressing-math downgrade). Under QEMU's
//! `ast1030-evb`, both CS1 segment reads and CS0-out-of-chip reads alias
//! back to the single attached `w25q80bl`'s byte 0 (CS line aliasing +
//! chip wrap), so neither tactic distinguishes broken from fixed code on
//! this model — both forms either always pass or always fail regardless of
//! whether `device_to_controller_offset` is invoked. The deterministic
//! testable surface of DEV-PAR-002/003 on QEMU is the per-CS bounds check
//! above; the actual offset translation is exercised by every passing
//! `cs1.read/program/erase` call in the suite (which would otherwise hit
//! the wrong segment on real silicon). The address-translation invariant
//! is restated in code via `device_to_controller_offset` and is asserted
//! at the type level, not at runtime under this model.
//!
//! The CS0 facade's erase + program in Phase 1 is a smoke test that the
//! fix did not regress the CS0 happy path.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, SpiNorFlashDevice, FmcUninit, SmcConfig, SmcController, SmcError,
    SpiNorFlash,
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

const CS1_CAPACITY_BYTES: u32 = (CS1_CFG.capacity_mb) * 1024 * 1024;

// Non-trivial marker for the CS0 smoke-test plant in Phase 1.
const MARKER: [u8; 16] = [
    0xA5, 0x5A, 0xA5, 0x5A, 0xA5, 0x5A, 0xA5, 0x5A,
    0xA5, 0x5A, 0xA5, 0x5A, 0xA5, 0x5A, 0xA5, 0x5A,
];

fn run_offsets_test() -> Result<(), SmcError> {
    let config = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(CS0_CFG),
        cs1: Some(CS1_CFG),
        dma_enabled: false,
        enable_interrupts: false,
    };

    let uninit = unsafe { FmcUninit::new(config)? };
    let mut fmc = uninit.init()?;

    if !fmc.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // --- Phase 1: Plant a marker on CS0 at device-local offset 0. ---
    let mut page = [0xFFu8; 256];
    page[..MARKER.len()].copy_from_slice(&MARKER);

    {
        let mut cs0 = SpiNorFlash::from_fmc_cs(&mut fmc, CS0_CFG, ChipSelect::Cs0)?;
        cs0.erase_sector(0)?;
        let written = cs0.program_page(0, &page)?;
        if written != page.len() {
            return Err(SmcError::HardwareError);
        }
        // Sanity: CS0 facade reads its own marker back.
        let mut probe = [0u8; MARKER.len()];
        cs0.read(0, &mut probe)?;
        if probe != MARKER {
            return Err(SmcError::HardwareError);
        }
    }

    // --- Phase 2: CS1 facade — per-CS bounds. ---
    {
        let mut cs1 = SpiNorFlash::from_fmc_cs(&mut fmc, CS1_CFG, ChipSelect::Cs1)?;

        // Assertion 1: read past per-CS capacity must reject.
        let mut tmp = [0u8; 16];
        match cs1.read(CS1_CAPACITY_BYTES, &mut tmp) {
            Err(SmcError::InvalidCapacity) => {}
            _ => return Err(SmcError::HardwareError),
        }

        // Assertion 2: program past per-CS capacity must reject.
        match cs1.program_page(CS1_CAPACITY_BYTES, &page) {
            Err(SmcError::InvalidCapacity) => {}
            _ => return Err(SmcError::HardwareError),
        }

        // Assertion 3: erase past per-CS capacity must reject.
        match cs1.erase_sector(CS1_CAPACITY_BYTES) {
            Err(SmcError::InvalidCapacity) => {}
            _ => return Err(SmcError::HardwareError),
        }
    }

    Ok(())
}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 SpiNorFlash QEMU CS-Local Offsets Test";

    fn main() -> ! {
        let exit_status = match run_offsets_test() {
            Ok(()) => EXIT_SUCCESS,
            Err(_e) => EXIT_FAILURE,
        };
        exit(exit_status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
