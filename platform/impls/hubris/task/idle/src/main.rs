#![no_std]
#![no_main]

use userlib as _;

#[export_name = "main"]
fn main() -> ! {
    loop {
        if cfg!(feature = "insomniac") {
            // It may be useful, on some systems, to stop the CPU from entering
            // low power mode. Usually this is done because low power mode
            // interrupts debugging.
            //
            // This is not an empty block because that would reduce to `loop
            // {}`, which the compiler tends to replace with a trap instruction.
            core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        } else {
            userlib::idle();
        }
    }
}
