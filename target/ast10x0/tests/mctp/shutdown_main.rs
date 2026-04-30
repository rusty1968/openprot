// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use userspace::entry;
use userspace::syscall;

#[entry]
fn entry() -> ! {
    // Smoke-test completion condition: if userspace reached this point,
    // the system image booted and app startup succeeded.
    let _ = syscall::debug_shutdown(Ok(()));
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
