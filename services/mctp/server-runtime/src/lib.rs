// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! MCTP server IPC dispatch loop.
//!
//! Wraps the platform-independent `openprot_mctp_server::{Server, dispatch}`
//! in the Pigweed WaitGroup loop. Topology-agnostic, like
//! `i2c_server_runtime`: the caller supplies a list of client [`Channel`]s
//! (identified purely by their runtime IPC handle, never a named
//! `handle::*` constant) plus a single additional transport wait source, and
//! [`run`] multiplexes all of them. This lets one `mctp_server` process
//! answer any number of client channels (e.g. an SPDM requester, an SPDM
//! responder, and a PLDM task all at once) without duplicating the loop.
//!
//! The only kernel-tagged MCTP server crate; it wraps the host-buildable
//! `openprot_mctp_server::dispatch` in the Pigweed loop.

#![no_std]

use openprot_mctp_api::wire::{self, MAX_PAYLOAD_SIZE, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE};
use openprot_mctp_api::{Handle, ResponseCode};
use openprot_mctp_server::dispatch::{self, DispatchOutcome};
use openprot_mctp_server::{Sender, Server};
use pw_status::Result;
use userspace::syscall::{self, Signals};
use userspace::time::{Clock, Duration, Instant, SystemClock};

/// A deferred `Recv` awaiting a message or timeout on some [`Channel`].
struct Pending {
    /// The MCTP-layer handle (from `req`/`listener`) the recv was issued on.
    mctp_handle: Handle,
    /// Absolute deadline, or `Instant::MAX` for "no timeout".
    deadline: Instant,
}

/// One IPC client channel this server answers on.
///
/// Identified purely by its runtime handle value — never a named
/// `handle::*` constant — so the same [`run`] loop works whether the
/// process serves one channel or many.
pub struct Channel {
    handle: u32,
    pending: Option<Pending>,
}

impl Channel {
    /// Wrap a `channel_handler` IPC handle for use with [`run`].
    pub const fn new(handle: u32) -> Self {
        Self {
            handle,
            pending: None,
        }
    }
}

/// Run the MCTP IPC dispatch loop forever.
///
/// Registers every channel in `channels` (`READABLE`) plus
/// `transport_handle`/`transport_signal` (e.g. an inbound-frame
/// notification) with `wg`, then loops. `channels` must be non-empty with
/// distinct handles.
///
/// On a `transport_handle` wake, `on_transport` is called to feed the
/// server (e.g. `server.inbound(pkt)`); any channel with a deferred recv is
/// then retried. On a client channel wake, the request is decoded and
/// dispatched; a `Recv` that can't complete immediately defers that
/// channel (removed from `wg`) until a later transport wake or its own
/// timeout re-arms it.
///
/// A per-channel IPC error (e.g. a misbehaving client) is logged and that
/// channel is skipped; it never aborts service for the other channels. Only
/// WaitGroup setup and `object_wait` failures are fatal.
pub fn run<S: Sender, const N: usize>(
    wg: u32,
    channels: &mut [Channel],
    transport_handle: u32,
    transport_signal: Signals,
    server: &mut Server<S, N>,
    mut on_transport: impl FnMut(&mut Server<S, N>),
) -> Result<()> {
    if channels.is_empty() {
        return Err(pw_status::Error::InvalidArgument);
    }

    for ch in channels.iter() {
        syscall::wait_group_add(wg, ch.handle, Signals::READABLE, ch.handle as usize)?;
    }
    syscall::wait_group_add(
        wg,
        transport_handle,
        transport_signal,
        transport_handle as usize,
    )?;

    let epoch = SystemClock::now();
    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut response_buf = [0u8; MAX_RESPONSE_SIZE];
    let mut recv_buf = [0u8; MAX_PAYLOAD_SIZE];
    let wait_signals = Signals::READABLE | transport_signal;

    loop {
        let wait_deadline = earliest_deadline(channels);
        let ev = match syscall::object_wait(wg, wait_signals, wait_deadline) {
            Ok(ev) => ev,
            Err(pw_status::Error::DeadlineExceeded) => {
                retry_pending(wg, channels, server, epoch, &mut recv_buf, &mut response_buf);
                continue;
            }
            Err(err) => return Err(err),
        };

        let woken = ev.user_data as u32;
        if woken == transport_handle {
            on_transport(server);
            retry_pending(wg, channels, server, epoch, &mut recv_buf, &mut response_buf);
            continue;
        }

        let Some(ch) = channels.iter_mut().find(|c| c.handle == woken) else {
            continue;
        };

        // channel_read is non-blocking here because the WaitGroup only fires
        // after READABLE is set.
        let len = match syscall::channel_read(ch.handle, 0, &mut request_buf) {
            Ok(len) => len,
            Err(_) => {
                pw_log::error!("channel_read failed");
                continue;
            }
        };
        if ch.pending.is_some() {
            // Unreachable in steady state: a channel is removed from the
            // WaitGroup while a recv is pending on it, and each channel
            // serves one client at a time.
            let resp = wire::MctpResponseHeader::error(ResponseCode::InternalError);
            response_buf[..wire::MctpResponseHeader::SIZE].copy_from_slice(&resp.to_bytes());
            if syscall::channel_respond(
                ch.handle,
                &response_buf[..wire::MctpResponseHeader::SIZE],
            )
            .is_err()
            {
                pw_log::error!("channel_respond failed");
            }
            continue;
        }

        let now_millis = millis_since(epoch);
        match dispatch::dispatch_mctp_op(
            &request_buf[..len],
            &mut response_buf,
            server,
            &mut recv_buf,
            now_millis,
        ) {
            DispatchOutcome::Reply(n) => {
                if syscall::channel_respond(ch.handle, &response_buf[..n]).is_err() {
                    pw_log::error!("channel_respond failed");
                }
            }
            DispatchOutcome::Pending { handle: mctp_handle } => {
                let timeout_millis = wire::get_recv_timeout(&request_buf[..len]);
                let deadline = if timeout_millis == 0 {
                    Instant::MAX
                } else {
                    SystemClock::now()
                        .checked_add_duration(Duration::from_millis(timeout_millis as u64))
                        .unwrap_or(Instant::MAX)
                };
                ch.pending = Some(Pending {
                    mctp_handle,
                    deadline,
                });
                if syscall::wait_group_remove(wg, ch.handle).is_err() {
                    pw_log::error!("wait_group_remove failed");
                }
            }
        }
    }
}

/// Bridge the kernel's tick-based clock to the `Server`'s portable `now_millis` domain.
fn millis_since(epoch: Instant) -> u64 {
    (SystemClock::now() - epoch).as_millis() as u64
}

/// Earliest deadline across all channels with a deferred recv, or `Instant::MAX`.
fn earliest_deadline(channels: &[Channel]) -> Instant {
    channels
        .iter()
        .filter_map(|c| c.pending.as_ref().map(|p| p.deadline))
        .min()
        .unwrap_or(Instant::MAX)
}

/// Drive every deferred recv to completion, responding on whichever channel
/// it was issued on and re-arming that channel's WaitGroup entry.
///
/// A failure on one channel is logged and that channel is left pending
/// rather than aborting delivery to the other channels resolved in the same
/// call.
fn retry_pending<S: Sender, const N: usize>(
    wg: u32,
    channels: &mut [Channel],
    server: &mut Server<S, N>,
    epoch: Instant,
    recv_buf: &mut [u8],
    response_buf: &mut [u8],
) {
    if channels.iter().all(|c| c.pending.is_none()) {
        return;
    }

    let now_millis = millis_since(epoch);
    dispatch::drive_pending(server, now_millis, recv_buf, response_buf, |handle, bytes| {
        let Some(ch) = channels
            .iter_mut()
            .find(|c| c.pending.as_ref().map(|p| p.mctp_handle) == Some(handle))
        else {
            return;
        };
        ch.pending = None;
        if syscall::channel_respond(ch.handle, bytes).is_err() {
            pw_log::error!("channel_respond failed");
            return;
        }
        if syscall::wait_group_add(wg, ch.handle, Signals::READABLE, ch.handle as usize).is_err() {
            pw_log::error!("wait_group_add failed");
        }
    });
}
