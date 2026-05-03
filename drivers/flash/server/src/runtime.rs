// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use flash_api::backend::{FlashBackend, IrqMask};
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

use crate::{DispatchOutcome, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE, dispatch_request};

/// Holds at most one in-flight flash request that is waiting for the
/// operation-complete interrupt before it can be retried.
pub struct PendingRequest {
    channel: Option<u32>,
    request_len: usize,
    request: [u8; MAX_REQUEST_SIZE],
}

impl PendingRequest {
    pub const fn new() -> Self {
        Self {
            channel: None,
            request_len: 0,
            request: [0; MAX_REQUEST_SIZE],
        }
    }

    pub fn park(&mut self, channel: u32, request: &[u8]) -> bool {
        if self.channel.is_some() || request.len() > self.request.len() {
            return false;
        }

        self.channel = Some(channel);
        self.request_len = request.len();
        self.request[..request.len()].copy_from_slice(request);
        true
    }

    pub fn take_into(&mut self, out: &mut [u8]) -> Option<(u32, usize)> {
        let channel = self.channel.take()?;
        let request_len = self.request_len;
        if request_len > out.len() {
            self.channel = Some(channel);
            return None;
        }

        out[..request_len].copy_from_slice(&self.request[..request_len]);
        self.request_len = 0;
        Some((channel, request_len))
    }
}

impl Default for PendingRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the flash server dispatch loop forever.
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
/// Adding another client channel is one more `wait_group_add` call in the binary.
/// Since each controller has its own driver instance with its own IRQ, there is
/// no contention between CPU cores or controllers—IRQ routing is straightforward.
///
/// If a backend returns `WouldBlock`, the runtime parks that request and
/// retries it when the operation-complete IRQ fires. The client API stays
/// synchronous: the response is deferred until the retry completes.
pub fn run<B: FlashBackend>(backend: &mut B, wg: u32, irq: u32, irq_signals: Signals) -> ! {
    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut response_buf = [0u8; MAX_RESPONSE_SIZE];
    let mut pending = PendingRequest::new();
    let wait_mask = Signals::READABLE | irq_signals;

    loop {
        let Ok(wait_return) = syscall::object_wait(wg, wait_mask, Instant::MAX) else {
            continue;
        };

        if wait_return.user_data as u32 == irq
            && wait_return.pending_signals.contains(irq_signals)
        {
            let acked = wait_return.pending_signals & irq_signals;
            if let Some((channel, req_len)) = pending.take_into(&mut request_buf) {
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
