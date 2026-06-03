// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use ast10x0_peripherals::scu::{ClockRegisterHalf, ScuRegisterHalf};
use ast10x0_peripherals::scu::{PinctrlPin, ScuRegisters};
use ast10x0_peripherals::smc::{CalibrationScratch, SmcConfig, SmcError, UninitSmc, SPI_CALIB_LEN};

/// Board descriptor metadata for AST10x0 board initialization.
#[derive(Clone, Debug)]
pub struct Ast10x0BoardDescriptor {
    /// Pin control groups to apply during board init.
    /// Applied in order via `ScuRegisters::apply_pinctrl_group()`.
    pub pinctrl_groups: &'static [&'static [PinctrlPin]],
    /// SMC controllers to calibrate during pre-kernel bring-up.
    ///
    /// Keep this empty when a board does not use SMC bring-up.
    pub smc_configs: &'static [SmcConfig],
}

/// Runtime board object that executes hardware initialization steps.
pub struct Ast10x0Board {
    descriptor: Ast10x0BoardDescriptor,
}

impl Ast10x0Board {
    /// Create a board runtime object from board metadata.
    #[must_use]
    pub const fn new(descriptor: Ast10x0BoardDescriptor) -> Self {
        Self { descriptor }
    }

    /// Initialize board: apply pinctrl groups and initialize I2C subsystem.
    ///
    /// This performs the complete platform-level initialization:
    /// 1. Apply pinctrl groups
    /// 2. Enable I2C clock via SCU
    /// 3. Assert I2C/SMBus controller reset
    /// 4. Delay for reset to settle
    /// 5. Deassert reset
    /// 6. Delay for recovery
    /// 7. Configure I2C global registers (clock dividers, etc.)
    ///
    /// # Safety
    /// - Must be called only once during board initialization.
    /// - Not thread-safe; caller must ensure no concurrent SCU or I2C accesses.
    pub unsafe fn init(&self) {
        // Unlock SCU once before the sequence of writes (aspeed-rust pattern)
        let scu = unsafe { ScuRegisters::new_global_unlocked() };

        // Apply pinctrl groups
        for group in self.descriptor.pinctrl_groups {
            scu.apply_pinctrl_group(group);
        }

        // Enable I2C clock (Group 0, bit 2)
        scu.ungate_clock_mask(ClockRegisterHalf::Lower, 1 << 2);

        // Assert I2C reset (Upper half, bit 2)
        scu.assert_reset_mask(ScuRegisterHalf::Upper, 1 << 2);
        delay_us(1000);

        // Deassert I2C reset
        scu.deassert_reset_mask(ScuRegisterHalf::Upper, 1 << 2);
        delay_us(1000);

        // Configure I2C global registers (clock dividers, etc.)
        unsafe { ast10x0_peripherals::i2c::init_i2c_global() };
    }

    /// Calibrate SMC controllers during pre-kernel bring-up.
    ///
    /// Controllers are initialized and calibrated sequentially using a shared
    /// static scratch buffer, avoiding large stack allocations.
    ///
    /// # Safety
    /// - Must be called only in single-threaded bring-up before concurrent SMC access.
    /// - Each listed controller must be uniquely owned by this initialization flow.
    pub unsafe fn calibrate_smc_controllers(&self) -> Result<(), SmcError> {
        static mut SMC_CALIBRATION_SCRATCH: CalibrationScratch = [0u8; SPI_CALIB_LEN];

        for config in self.descriptor.smc_configs {
            // SAFETY: Board init is single-threaded and called once. This code
            // borrows the static scratch buffer for one calibration at a time.
            #[expect(static_mut_refs)]
            let scratch = unsafe { &mut SMC_CALIBRATION_SCRATCH };

            // SAFETY: Board init runs before concurrent users exist and owns
            // early hardware bring-up for listed controllers.
            let uninit = unsafe { UninitSmc::new(*config)? };
            let ready = uninit.init()?;
            let _calibrated = ready.calibrate(scratch)?;
        }

        Ok(())
    }
}

/// Simple busy-wait delay in microseconds.
///
/// This is a placeholder; production code should use a proper timer or delay provider.
/// Spins for approximately `micros` microseconds.
#[inline]
fn delay_us(micros: u32) {
    // Very rough approximation: ~16 cycles per microsecond on Cortex-M4 @ ~50MHz
    // This is calibration-free but inaccurate; improve for production.
    for _ in 0..(micros * 16) {
        core::hint::spin_loop();
    }
}
