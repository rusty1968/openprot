// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_mctp_server_boot::handle;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

#[entry]
fn entry() {
    pw_log::info!("mctp_server boot smoke app started");

    if syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize).is_err() {
        pw_log::error!("wait_group_add failed");
        loop {}
    }

    pw_log::info!("blocking in object_wait on MCTP channel READABLE");
    match syscall::object_wait(handle::WG, Signals::READABLE, Instant::MAX) {
        Ok(_ev) => pw_log::error!("object_wait unexpectedly returned event"),
        Err(_e) => pw_log::error!("object_wait returned error"),
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
