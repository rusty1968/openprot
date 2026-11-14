// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Simulated verifier service.
//!
//! A verify request names the component and how long the check takes, so the
//! orchestrator's test plan decides the latency and the service keeps no
//! scenario state.
//!
//! Request:  `[OP_VERIFY, id, units]`  (one unit is `UNIT_MS`)
//! Response: `[status, id]`            (status 0 is "authenticated")
//!
//! The reply is deferred. While it is parked the loop keeps running, but the
//! channel is out of the WaitGroup, so a client that cancels still cannot
//! have a new request read until the deferred reply is completed. The
//! orchestrator's test plan accounts for that.
//!
//! A second, never-connected channel (`idle`) keeps the WaitGroup from going
//! empty while `ipc` is deferred. The kernel rejects a wait on a WaitGroup
//! with no members (`InvalidArgument`), so a `ServiceLoop` whose only
//! channel is parked cannot wait for its own deadline.

#![no_main]
#![no_std]

use app_verifier::handle;
use pw_status::Result;
use userspace::entry;
use userspace::time::{Clock, Duration, Instant, SystemClock};
use util_service::{Channel, Event, ServiceLoop};

const OP_VERIFY: u8 = 1;
const UNIT_MS: u64 = 10;
const KEY: u32 = 1;

fn serve() -> Result<()> {
    let mut channels = [Channel::new(handle::IPC), Channel::new(handle::IDLE)];
    let mut service = ServiceLoop::new(handle::WG, &mut channels)?;
    let mut buf = [0u8; 8];
    // When the parked reply is due, and for which component.
    let mut parked: Option<(Instant, u8)> = None;

    loop {
        let deadline = parked.map_or(Instant::MAX, |(due, _)| due);
        match service.next(deadline, &mut buf)? {
            Event::Request { channel, len } => {
                let req = buf.get(..len).unwrap_or(&[]);
                if let [OP_VERIFY, id, units] = *req {
                    match service.defer(channel, KEY) {
                        Ok(()) => {
                            // One millisecond late on purpose: the wake must
                            // not land a tick before the deadline.
                            let after = Duration::from_millis(u64::from(units) * UNIT_MS + 1);
                            let due = SystemClock::now()
                                .checked_add_duration(after)
                                .unwrap_or(Instant::MAX);
                            parked = Some((due, id));
                        }
                        Err(_) => {
                            pw_log::error!("verifier: defer failed");
                            let _ = service.reply(channel, &[1, id]);
                        }
                    }
                } else {
                    let _ = service.reply(channel, &[1, 0]);
                }
            }
            Event::Deadline => {
                if let Some((due, id)) = parked
                    && SystemClock::now() >= due
                {
                    parked = None;
                    // Fails if the client gave up (cancelled) while the reply
                    // was parked. The channel is served again either way.
                    if service.complete(KEY, &[0, id]).is_err() {
                        pw_log::warn!("verifier: late reply dropped (client cancelled)");
                    }
                }
            }
            Event::Source(_) => {}
        }
    }
}

#[entry]
fn entry() {
    if let Err(e) = serve() {
        pw_log::error!("verifier: loop exited, status {}", e as u32);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
