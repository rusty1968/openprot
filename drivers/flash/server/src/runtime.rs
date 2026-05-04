// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use flash_api::backend::{FlashBackend, IrqMask};
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

use crate::{DispatchOutcome, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE, dispatch_request};

/// Holds at most one in-flight flash request that is waiting for the
/// operation-complete interrupt before it can be retried.
///
/// The parked routing `key` lets the IRQ retry path resume the request on
/// the same chip-select (or other backend route) it was originally parked
/// against. For single-CS backends `K = ()`.
pub struct PendingRequest<K: Copy> {
    channel: Option<u32>,
    key: Option<K>,
    request_len: usize,
    request: [u8; MAX_REQUEST_SIZE],
}

impl<K: Copy> PendingRequest<K> {
    pub fn new() -> Self {
        Self {
            channel: None,
            key: None,
            request_len: 0,
            request: [0; MAX_REQUEST_SIZE],
        }
    }

    pub fn park(&mut self, channel: u32, key: K, request: &[u8]) -> bool {
        if self.channel.is_some() || request.len() > self.request.len() {
            return false;
        }

        self.channel = Some(channel);
        self.key = Some(key);
        self.request_len = request.len();
        self.request[..request.len()].copy_from_slice(request);
        true
    }

    pub fn take_into(&mut self, out: &mut [u8]) -> Option<(u32, K, usize)> {
        let channel = self.channel.take()?;
        let key = match self.key.take() {
            Some(k) => k,
            None => {
                self.channel = Some(channel);
                return None;
            }
        };
        let request_len = self.request_len;
        if request_len > out.len() {
            self.channel = Some(channel);
            self.key = Some(key);
            return None;
        }

        out[..request_len].copy_from_slice(&self.request[..request_len]);
        self.request_len = 0;
        Some((channel, key, request_len))
    }
}

impl<K: Copy> Default for PendingRequest<K> {
    fn default() -> Self {
        Self::new()
    }
}

/// Binds an IPC channel handle to the backend's per-call routing key.
///
/// The dual-CS server hosts one binding per chip-select; single-CS servers
/// keep using `run` (which is a thin wrapper that synthesizes a single
/// `()`-keyed binding internally).
#[derive(Clone, Copy)]
pub struct ChannelBinding<K: Copy> {
    pub handle: u32,
    pub key: K,
}

/// Run the flash server dispatch loop forever (single-CS convenience).
///
/// Equivalent to calling `run_routed` with one `()`-keyed binding for
/// `channel`. Existing single-CS server binaries can keep using this
/// without source changes once their backend's `RouteKey` defaults to
/// `()`.
///
/// Each server instance is dedicated to a single flash controller (e.g., FMC,
/// SPI1, or SPI2 on AST1030/AST1060). The caller is responsible for populating
/// the wait group (`wg`) ahead of time with:
///
/// - For each IPC channel client, register it as:
///   `wait_group_add(wg, ch, Signals::READABLE, ch as usize)`.
/// - Register the controller's IRQ as:
///   `wait_group_add(wg, irq, irq_signals, irq as usize)`.
///
/// The loop routes wake-ups using `wait_return.user_data` as the channel handle.
pub fn run<B: FlashBackend<RouteKey = ()>>(
    backend: &mut B,
    wg: u32,
    irq: u32,
    irq_signals: Signals,
) -> ! {
    // Single-CS path: no per-channel routing. Any READABLE wake maps to the
    // unit key.
    run_routed_inner(backend, wg, &[], irq, irq_signals, Some(()))
}

/// Run the flash server dispatch loop forever, routing each registered
/// channel to the backend's per-CS (or per-route) key.
///
/// `channels` lists every IPC channel handle the wait_group will deliver
/// `READABLE` signals on. The caller must register each channel with the
/// wait_group ahead of time; `channels` only tells the runtime which
/// `RouteKey` to pass to the backend for each handle.
///
/// A wake on a channel handle that is not in `channels` is dropped (logged
/// and ignored) — the kernel should never deliver one, but the runtime
/// fails closed if it does.
pub fn run_routed<B: FlashBackend>(
    backend: &mut B,
    wg: u32,
    channels: &[ChannelBinding<B::RouteKey>],
    irq: u32,
    irq_signals: Signals,
) -> ! {
    run_routed_inner(backend, wg, channels, irq, irq_signals, None)
}

fn run_routed_inner<B: FlashBackend>(
    backend: &mut B,
    wg: u32,
    channels: &[ChannelBinding<B::RouteKey>],
    irq: u32,
    irq_signals: Signals,
    fallback_key: Option<B::RouteKey>,
) -> ! {
    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut response_buf = [0u8; MAX_RESPONSE_SIZE];
    let mut pending = PendingRequest::<B::RouteKey>::new();
    let wait_mask = Signals::READABLE | irq_signals;

    loop {
        let Ok(wait_return) = syscall::object_wait(wg, wait_mask, Instant::MAX) else {
            continue;
        };

        if wait_return.user_data as u32 == irq
            && wait_return.pending_signals.contains(irq_signals)
        {
            let acked = wait_return.pending_signals & irq_signals;
            if let Some((channel, key, req_len)) = pending.take_into(&mut request_buf) {
                pw_log::info!(
                    "flash_server: irq wake, retry ch={} req_len={} op=0x{:02x}",
                    channel as u32,
                    req_len as u32,
                    request_buf[0] as u32
                );
                let _ = backend.disable_interrupts(IrqMask::OPERATION_COMPLETE);
                let _ = syscall::interrupt_ack(irq, acked);

                match dispatch_request(
                    backend,
                    key,
                    &mut pending,
                    channel,
                    &request_buf[..req_len],
                    &mut response_buf,
                ) {
                    DispatchOutcome::Respond(resp_len) => {
                        pw_log::info!(
                            "flash_server: irq tx ch={} resp_len={}",
                            channel as u32,
                            resp_len as u32
                        );
                        let _ = syscall::channel_respond(channel, &response_buf[..resp_len]);
                    }
                    DispatchOutcome::Queued => {
                        pw_log::info!("flash_server: irq retry re-queued ch={}", channel as u32);
                        // Still not ready; request was re-parked by dispatch.
                    }
                }
            } else {
                pw_log::info!("flash_server: irq wake with no pending request");
                let _ = syscall::interrupt_ack(irq, acked);
            }
            continue;
        }

        if !wait_return.pending_signals.contains(Signals::READABLE) {
            continue;
        }

        let channel = wait_return.user_data as u32;

        // Resolve the routing key. For multi-CS callers (`run_routed`) the
        // key comes from the binding list; for single-CS callers (`run`) the
        // fallback is `Some(())` and bindings are empty.
        let key = match channels.iter().find(|b| b.handle == channel) {
            Some(binding) => binding.key,
            None => match fallback_key {
                Some(k) => k,
                None => {
                    pw_log::warn!(
                        "flash_server: wake on unknown channel ch={}",
                        channel as u32
                    );
                    continue;
                }
            },
        };

        let Ok(req_len) = syscall::channel_read(channel, 0, &mut request_buf) else {
            pw_log::warn!("flash_server: channel_read failed ch={}", channel as u32);
            continue;
        };

        pw_log::info!(
            "flash_server: ipc rx ch={} req_len={} op=0x{:02x}",
            channel as u32,
            req_len as u32,
            request_buf[0] as u32
        );

        match dispatch_request(
            backend,
            key,
            &mut pending,
            channel,
            &request_buf[..req_len],
            &mut response_buf,
        ) {
            DispatchOutcome::Respond(resp_len) => {
                pw_log::info!(
                    "flash_server: ipc tx ch={} resp_len={}",
                    channel as u32,
                    resp_len as u32
                );
                let _ = syscall::channel_respond(channel, &response_buf[..resp_len]);
            }
            DispatchOutcome::Queued => {
                pw_log::info!("flash_server: request queued ch={}", channel as u32);
                // Response will be sent from the IRQ completion path.
            }
        }
    }
}

