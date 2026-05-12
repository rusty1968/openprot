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

use ast10x0_peripherals::scu::{PinctrlPin, ScuRegisters};
use ast10x0_peripherals::scu::{ClockRegisterHalf, ScuRegisterHalf};

/// Board-level descriptor for AST10x0 SMC flash topology.
///
/// Describes board pin-control setup for early platform initialization.
#[derive(Clone, Debug)]
pub struct Ast10x0BoardDescriptor {
    /// Pin control groups to apply during board init.
    /// Applied in order via `ScuRegisters::apply_pinctrl_group()`.
    pub pinctrl_groups: &'static [&'static [PinctrlPin]],
}

impl Ast10x0BoardDescriptor {
    /// Initialize board: apply pinctrl groups.
    ///
    /// # Safety
    /// Caller must hold exclusive access to the SCU register block.
    pub unsafe fn init_board(&self, scu: &ScuRegisters) {
        for group in self.pinctrl_groups {
            scu.apply_pinctrl_group(group);
        }
    }
}

/// Initialize I2C subsystem at the board level.
///
/// This performs the platform-level I2C initialization:
/// 1. Enable I2C clock via SCU
/// 2. Assert I2C/SMBus controller reset
/// 3. Delay for reset to settle
/// 4. Deassert reset
/// 5. Delay for recovery
/// 6. Configure I2C global registers (clock dividers, etc.)
///
/// # Safety
/// - Must be called only once during board initialization.
/// - Not thread-safe; caller must ensure no concurrent SCU or I2C accesses.
pub unsafe fn init_i2c() {
    // Unlock SCU once before the sequence of writes (aspeed-rust pattern)
    let scu = unsafe { ScuRegisters::new_global_unlocked() };

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