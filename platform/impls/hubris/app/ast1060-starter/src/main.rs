//! AST1060 Starter Task
//!
//! A minimal demonstration task for AST1060 hardware running on ExHubris.
//! This task serves as a basic starting point for AST1060-based applications.
//!
//! TODO: Update to use full ExHubris APIs when workspace is set up.

#![no_std]
#![no_main]

// TODO: Enable when ExHubris workspace is available
// use userlib as _;

#[cfg(feature = "jtag-halt")]
use core::ptr::{self, addr_of};

#[export_name = "main"]
fn main() -> ! {
    #[cfg(feature = "jtag-halt")]
    jtag_halt();
    
    let mut counter = 0u32;
    
    loop {
        // Simple demonstration - increment counter
        counter = counter.wrapping_add(1);
        
        // Simple delay loop (not ideal, but works without ExHubris APIs)
        for _ in 0..1_000_000 {
            cortex_m::asm::nop();
        }
        
        // TODO: Replace with ExHubris timer API when available:
        // let start = userlib::sys_get_timer().now;
        // const INTERVAL: u64 = 1000;
        // let mut next = start + INTERVAL;
        // userlib::sys_set_timer(Some(next), hubris_notifications::TIMER);
        // userlib::sys_recv_notification(hubris_notifications::TIMER);
        
        // In a real application, this is where you'd do actual work:
        // - Communicate with other tasks via IPC
        // - Request crypto operations from crypto driver task
        // - Handle application-specific logic
    }
}

#[cfg(feature = "jtag-halt")]
fn jtag_halt() {
    static mut HALT: u32 = 1;

    // This is a debugging aid to halt the CPU in JTAG mode
    // A debugger can set HALT to 0 to continue execution
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
    // In a real application, this might log the panic
    // or signal the failure to a watchdog task
    loop {
        cortex_m::asm::wfi();
    }
}
