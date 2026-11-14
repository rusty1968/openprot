// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Simulated managed devices: a reset controller and a boot-event source.
//!
//! `reset` channel, answered at once with `[status, id]`:
//!   `[OP_RELEASE, id, units]`  release `id`; it boots after `units` units,
//!                              or never if `units` is 0
//!   `[OP_ASSERT, id]`          hold `id` in reset; a pending boot is lost
//!   `[OP_RESTORE, id]`         restore `id` from its recovery source
//!
//! `events` channel, a long-poll: `[OP_WAIT]` is answered `[0, id]` with the
//! next device that booted. With none ready the reply is deferred, so one
//! request sits parked until a boot happens. That is how an event reaches a
//! client that never blocks.

#![no_main]
#![no_std]

use app_devices::handle;
use pw_status::Result;
use userspace::entry;
use userspace::time::{Clock, Duration, Instant, SystemClock};
use util_service::{Channel, Event, ServiceLoop};

const OP_RELEASE: u8 = 2;
const OP_ASSERT: u8 = 3;
const OP_RESTORE: u8 = 4;
const UNIT_MS: u64 = 10;
const KEY_EVENTS: u32 = 1;
const RESET_INDEX: usize = 0;
const DEVICES: usize = 3;

/// Drop every queued boot for `id`.
fn forget(ready: &mut heapless::Deque<u8, 8>, id: u8) {
    for _ in 0..ready.len() {
        if let Some(x) = ready.pop_front()
            && x != id
        {
            let _ = ready.push_back(x);
        }
    }
}

fn serve() -> Result<()> {
    let mut channels = [Channel::new(handle::RESET), Channel::new(handle::EVENTS)];
    let mut service = ServiceLoop::new(handle::WG, &mut channels)?;
    let mut buf = [0u8; 8];
    // When each device finishes booting, if it is booting.
    let mut due: [Option<Instant>; DEVICES] = [None; DEVICES];
    // Devices that booted and have not been reported yet.
    let mut ready: heapless::Deque<u8, 8> = heapless::Deque::new();
    let mut parked = false;

    loop {
        let deadline = due.iter().flatten().copied().min().unwrap_or(Instant::MAX);
        if let Event::Request { channel, len } = service.next(deadline, &mut buf)? {
            let req = buf.get(..len).unwrap_or(&[]);
            if channel.index() == RESET_INDEX {
                let reply = match *req {
                    [OP_RELEASE, id, units] => match due.get_mut(usize::from(id)) {
                        Some(slot) => {
                            *slot = if units == 0 {
                                None
                            } else {
                                let after =
                                    Duration::from_millis(u64::from(units) * UNIT_MS + 1);
                                SystemClock::now().checked_add_duration(after)
                            };
                            [0, id]
                        }
                        None => [1, id],
                    },
                    [OP_ASSERT, id] => match due.get_mut(usize::from(id)) {
                        Some(slot) => {
                            *slot = None;
                            forget(&mut ready, id);
                            [0, id]
                        }
                        None => [1, id],
                    },
                    [OP_RESTORE, id] => [0, id],
                    _ => [1, 0],
                };
                let _ = service.reply(channel, &reply);
            } else if let Some(id) = ready.pop_front() {
                let _ = service.reply(channel, &[0, id]);
            } else if service.defer(channel, KEY_EVENTS).is_ok() {
                parked = true;
            } else {
                pw_log::error!("devices: defer failed");
                let _ = service.reply(channel, &[1, 0]);
            }
        }

        // Whatever the wake was, move any device whose boot time has passed
        // onto the ready queue.
        let now = SystemClock::now();
        for (id, slot) in due.iter_mut().enumerate() {
            if let Some(at) = *slot
                && now >= at
            {
                *slot = None;
                let _ = ready.push_back(id as u8);
            }
        }
        if parked && let Some(id) = ready.pop_front() {
            parked = false;
            if service.complete(KEY_EVENTS, &[0, id]).is_err() {
                pw_log::warn!("devices: boot event dropped (client gone)");
            }
        }
    }
}

#[entry]
fn entry() {
    if let Err(e) = serve() {
        pw_log::error!("devices: loop exited, status {}", e as u32);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
