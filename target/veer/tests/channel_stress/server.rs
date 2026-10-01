// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Channel-stress echo server.
//!
//! Parks on the channel's `READABLE` signal, reads each request, and responds
//! with the same bytes. No I3C, no protocol — just the raw kernel channel
//! syscalls (`channel_read` / `channel_respond`), so a crash here is the
//! kernel's channel path, nothing else.

#![no_main]
#![no_std]

use chanstress_server_codegen::handle;
use userspace::process_entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

/// Response buffer; larger than any request the client sends.
const MAX_MSG: usize = 256;

#[process_entry("server")]
fn entry() {
    pw_log::info!("channel stress server: starting");

    let mut buf = [0u8; MAX_MSG];
    if syscall::wait_group_add(handle::WG, handle::CHANNEL, Signals::READABLE, 0).is_err() {
        pw_log::error!("channel stress server: wait_group_add failed");
    }

    loop {
        if syscall::object_wait(handle::WG, Signals::READABLE, Instant::MAX).is_err() {
            pw_log::error!("channel stress server: object_wait failed");
            continue;
        }
        match syscall::channel_read(handle::CHANNEL, 0, &mut buf) {
            Ok(n) => {
                if syscall::channel_respond(handle::CHANNEL, &buf[..n]).is_err() {
                    pw_log::error!("channel stress server: channel_respond failed");
                }
            }
            Err(_) => pw_log::error!("channel stress server: channel_read failed"),
        }
    }
}
