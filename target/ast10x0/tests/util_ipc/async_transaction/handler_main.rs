// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Handler side of the util/ipc AsyncTransaction QEMU test.
//!
//! Exercises `util_ipc::IpcHandler`: waits for a request, reads it, and
//! responds with the request byte incremented by one.

#![no_main]
#![no_std]

use app_handler::handle;
use pw_status::Error;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;
use util_ipc::{IpcHandle, IpcHandler};

#[entry]
fn entry() {
    let ipc = IpcHandle::new(handle::IPC);

    loop {
        if syscall::object_wait(handle::IPC, Signals::READABLE, Instant::MAX).is_err() {
            continue;
        }

        let mut buf = [0u8; 1];
        match ipc.read(0, &mut buf) {
            Ok(1) => {
                buf[0] = buf[0].wrapping_add(1);
                let _ = ipc.respond(&buf);
            }
            // Transaction was cancelled by the initiator while we were
            // waking up; go back to waiting for the next one.
            Ok(_) | Err(Error::Unavailable) => continue,
            Err(_) => continue,
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
