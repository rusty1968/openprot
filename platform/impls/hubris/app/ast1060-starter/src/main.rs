// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![no_std]
#![no_main]

// We have to do this if we don't otherwise use it to ensure its vector table
// gets linked in.

use cortex_m_rt::entry;
use ast1060_pac::Peripherals;

#[cfg(feature = "jtag-halt")]
use core::ptr::{self, addr_of};

#[entry]
fn main() -> ! {

    // This code just forces the ast1060 pac to be linked in.
    let peripherals = unsafe {
        Peripherals::steal()
    };
    peripherals.scu.scu000().modify(|_, w| {
        w
    });
    peripherals.scu.scu41c().modify(|_, w| {
        // Set the JTAG pinmux to 0x1f << 25
        w.enbl_armtmsfn_pin().bit(true)
            .enbl_armtckfn_pin().bit(true)
            .enbl_armtrstfn_pin().bit(true)
            .enbl_armtdifn_pin().bit(true)
            .enbl_armtdofn_pin().bit(true)
    });

    #[cfg(feature = "jtag-halt")]
    jtag_halt();

    // Note: In exhubris, we would need to adapt this to use the available
    // kernel startup mechanism or create a simple loop for demonstration
    loop {
        // Simple demonstration loop - in a real exhubris application,
        // this would be replaced with proper task initialization
        cortex_m::asm::nop();
    }

    // Original Hubris kernel startup (not available in exhubris):
    // unsafe { kern::startup::start_kernel(CYCLES_PER_MS) }
}

#[cfg(feature = "jtag-halt")]
fn jtag_halt() {
    static mut HALT : u32 = 1;

    // This is a hack to halt the CPU in JTAG mode.
    // It writes a value to a volatile memory location
    // Break by jtag and set val to zero to continue.
    loop {
        let val;
        unsafe {
            val = ptr::read_volatile(addr_of!(HALT));
        }

        if val == 0 {
            break;
        }
    }
}

// Panic handler required for no_std
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
