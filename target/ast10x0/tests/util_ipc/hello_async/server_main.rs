// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Server side of the hello-async test: one `ServiceLoop`, two channels.
//!
//! `slow` requests are deferred and completed `SLOW_DELAY_MS` later from the
//! loop's deadline. `fast` requests are answered at once. While a `slow`
//! reply is parked the loop must keep serving `fast`; that is the property
//! under test.

#![no_main]
#![no_std]

use app_server::handle;
use pw_status::Result;
use userspace::entry;
use userspace::time::{Clock, Duration, Instant, SystemClock};
use util_service::{Channel, Event, ServiceLoop};

const SLOW_DELAY_MS: u64 = 50;
const SLOW_INDEX: usize = 0;
/// Key of the one deferred reply; there is only ever one.
const SLOW_KEY: u32 = 1;

fn serve() -> Result<()> {
    let mut channels = [Channel::new(handle::SLOW), Channel::new(handle::FAST)];
    let mut service = ServiceLoop::new(handle::WG, &mut channels)?;
    let mut buf = [0u8; 16];
    // When the parked `slow` reply is due, if one is parked.
    let mut due: Option<Instant> = None;

    loop {
        match service.next(due.unwrap_or(Instant::MAX), &mut buf)? {
            Event::Request { channel, .. } if channel.index() == SLOW_INDEX => {
                match service.defer(channel, SLOW_KEY) {
                    Ok(()) => {
                        due = SystemClock::now()
                            .checked_add_duration(Duration::from_millis(SLOW_DELAY_MS));
                    }
                    Err(_) => {
                        pw_log::error!("server: defer failed");
                        let _ = service.reply(channel, b"err");
                    }
                }
            }
            Event::Request { channel, .. } => {
                let _ = service.reply(channel, b"fast");
            }
            Event::Deadline => {
                if due.take().is_some() {
                    // Fails if the client cancelled while the reply was
                    // parked. The channel is served again either way.
                    if service.complete(SLOW_KEY, b"slow").is_err() {
                        pw_log::warn!("server: complete failed (client gone)");
                    }
                }
            }
            Event::Source(_) => {}
        }
    }
}

#[entry]
fn entry() {
    if serve().is_err() {
        pw_log::error!("server: loop exited");
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
