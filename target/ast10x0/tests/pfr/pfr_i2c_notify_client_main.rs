// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Retired. The PFR mailbox is now driven entirely by `i2c_server` (server_main),
//! which owns `SwmbxCtrl` and serves master reads synchronously inside the slave
//! IRQ wake — no IPC client is needed in the read path. This process is kept as
//! an idle placeholder so the system image layout is unchanged.

#![no_main]
#![no_std]

use app_pfr_i2c_notify_client::handle;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

#[entry]
fn entry() {
    loop {
        // Park forever; nothing to do.
        let _ = syscall::object_wait(handle::I2C, Signals::USER, Instant::MAX);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
