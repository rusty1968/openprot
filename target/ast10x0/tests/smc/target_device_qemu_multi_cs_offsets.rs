// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 SpiNorFlash device facade — CS-local offset semantics
//! (DEV-PAR-002 + DEV-PAR-003).
//!
//! Verifies that on a multi-CS controller (CS0 = 2 MB, CS1 = 1 MB):
//!
//! 1. **Bounds via `read`** — `cs1_facade.read(offset = CS1_CAPACITY, ..)`
//!    returns `InvalidCapacity`. Under the broken code the controller-total
//!    bound (3 MB) accepts this; the per-CS bound (1 MB) must reject.
//! 2. **Address translation via `read`** — `cs1_facade.read(0, ..)` does
//!    *not* return the marker bytes that a CS0 facade has just programmed at
//!    its own offset 0. After the fix, `cs1_facade.read(0, ..)` translates
//!    to controller-window address `CS0_CAPACITY`, which the segment
//!    routing sends to the (unmodeled) CS1 chip on QEMU. The chosen RED
//!    tactic is content-separation with a non-trivial marker pattern: under
//!    `ast1030-evb` CS1 has no attached device, so reads from that segment
//!    return deterministic non-marker bytes (typically 0xFF or 0x00). The
//!    marker `0xA5 0x5A …` is engineered to differ from either value, so the
//!    assertion is robust to either flavor of "unmodeled" QEMU response
//!    without depending on cross-CS isolation we cannot prove on this model.
//! 3. **Bounds via `program_page`** — `cs1_facade.program_page(offset =
//!    CS1_CAPACITY, ..)` returns `InvalidCapacity`. Coupled to (1): both
//!    paths share `validate_range` → `capacity_bytes`.
//! 4. **Bounds via `erase_sector`** — `cs1_facade.erase_sector(offset =
//!    CS1_CAPACITY)` returns `InvalidCapacity`.
//!
//! The CS0 facade's program path is exercised only as a setup step for the
//! marker plant; its functional correctness is covered by
//! `target_device_qemu_program_erase`.

#![no_std]
#![no_main]

use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, FlashDevice, FmcUninit, SmcConfig, SmcController, SmcError,
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

const CS0_CAPACITY_BYTES: u32 = (CS0_CFG.capacity_mb) * 1024 * 1024;
const CS1_CAPACITY_BYTES: u32 = (CS1_CFG.capacity_mb) * 1024 * 1024;

// 16-byte non-trivial marker. Distinct from both 0xFF (erased) and 0x00
// (typical "no device" QEMU readback) so the content-separation assertion
// is robust without depending on cross-CS isolation that QEMU does not model.
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

    // --- Phase 2: CS1 facade — bounds + address translation. ---
    {
        let mut cs1 = SpiNorFlash::from_fmc_cs(&mut fmc, CS1_CFG, ChipSelect::Cs1)?;

        // Assertion 1: read past per-CS capacity must reject.
        let mut tmp = [0u8; 16];
        match cs1.read(CS1_CAPACITY_BYTES, &mut tmp) {
            Err(SmcError::InvalidCapacity) => {}
            _ => return Err(SmcError::HardwareError),
        }

        // Assertion 3: program past per-CS capacity must reject.
        match cs1.program_page(CS1_CAPACITY_BYTES, &page) {
            Err(SmcError::InvalidCapacity) => {}
            _ => return Err(SmcError::HardwareError),
        }

        // Assertion 4: erase past per-CS capacity must reject.
        match cs1.erase_sector(CS1_CAPACITY_BYTES) {
            Err(SmcError::InvalidCapacity) => {}
            _ => return Err(SmcError::HardwareError),
        }

        // Assertion 2: CS1.read(0) must NOT alias CS0's offset 0.
        // Broken code reads controller offset 0 → returns MARKER. Fixed code
        // reads controller offset CS0_CAPACITY → segment-routed to CS1
        // (unmodeled on QEMU) → non-marker bytes.
        let mut cs1_probe = [0u8; MARKER.len()];
        cs1.read(0, &mut cs1_probe)?;
        if cs1_probe == MARKER {
            return Err(SmcError::HardwareError);
        }
    }

    // Suppress dead-code warning for CS0 capacity constant; future tightening
    // (CS0-side bounds check) can adopt it without restructuring.
    let _ = CS0_CAPACITY_BYTES;

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
