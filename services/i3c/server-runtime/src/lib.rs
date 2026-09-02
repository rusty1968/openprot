// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Pigweed server-runtime for an I3C target.
//!
//! Wraps the host-buildable [`i3c_server`] (target-owning `Server` + request
//! dispatch) in the Pigweed WaitGroup reactor. A single IPC channel carries the
//! transport protocol ([`i3c_api`]); the I3C hardware IRQ is multiplexed onto
//! the same wait, so a frame arriving on the bus and a client request are both
//! just wake-ups.
//!
//! ## Hot path
//!
//! - **IRQ** — the wait returns the I3C interrupt signal; `on_interrupt` decodes
//!   it. On `InboundReady` the runtime drains one frame into the server's latch,
//!   `interrupt_ack`s, and raises `USER` on the client channel so a parked
//!   client wakes. On `ResponseRead` the staged transmit was consumed.
//! - **IPC** — the wait returns `READABLE`; the runtime reads one request and
//!   dispatches. A `Recv` that consumes the latch also clears `USER`; the IRQ
//!   path re-raises it when the next frame lands.
//!
//! The **only** kernel-tagged crate in the i3c server path.

#![no_std]

use i3c_api::{decode_request, I3cOp, MAX_FRAME};
use i3c_server::{dispatch, Server};
use openprot_hal_blocking::i3c_hardware::{I3cTarget, TargetEvent};
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

/// Run the i3c server forever.
///
/// Enables the target, registers its channel (`READABLE`) and IRQ with `wg`,
/// then reacts to bus interrupts and client requests until the process exits.
pub fn run<T: I3cTarget>(wg: u32, irq_signals: Signals, srv: &mut Server<T>) -> ! {
    if srv.target.enable().is_err() {
        pw_log::error!("i3c target enable failed");
    }
    if syscall::wait_group_add(wg, srv.channel, Signals::READABLE, srv.channel as usize).is_err() {
        pw_log::error!("wait_group_add channel failed");
    }
    if syscall::wait_group_add(wg, srv.irq, irq_signals, srv.irq as usize).is_err() {
        pw_log::error!("wait_group_add irq failed");
    }

    let wait_mask = Signals::READABLE | irq_signals;
    let mut request_buf = [0u8; MAX_FRAME];
    let mut response_buf = [0u8; MAX_FRAME];

    loop {
        let Ok(w) = syscall::object_wait(wg, wait_mask, Instant::MAX) else {
            continue;
        };

        // ---- I3C hardware IRQ: decode, latch an inbound frame, wake client ----
        if w.pending_signals.contains(irq_signals) {
            let acked = w.pending_signals & irq_signals;
            match srv.target.on_interrupt() {
                Ok(TargetEvent::InboundReady) => {
                    if srv.latch_inbound().is_err() {
                        pw_log::error!("i3c read_frame failed");
                    }
                }
                Ok(_) => {}
                Err(_) => pw_log::error!("i3c on_interrupt failed"),
            }
            if syscall::interrupt_ack(srv.irq, acked).is_err() {
                pw_log::error!("interrupt_ack failed");
            }
            if srv.has_frame() {
                // ORs USER onto the client channel without disturbing READABLE.
                if syscall::object_set_peer_user_signal(srv.channel, true).is_err() {
                    pw_log::error!("object_set_peer_user_signal failed");
                }
            }
            continue;
        }

        // ---- client IPC request ----
        if !w.pending_signals.contains(Signals::READABLE) {
            continue;
        }
        let Ok(req_len) = syscall::channel_read(srv.channel, 0, &mut request_buf) else {
            continue;
        };
        let is_recv = matches!(
            decode_request(&request_buf[..req_len]),
            Some((I3cOp::Recv, _))
        );
        let resp_len = dispatch(srv, &request_buf[..req_len], &mut response_buf);
        // A Recv consumes the latch, so drop the USER notification; the IRQ path
        // re-raises it when the next frame arrives.
        if is_recv && syscall::object_set_peer_user_signal(srv.channel, false).is_err() {
            pw_log::error!("object_set_peer_user_signal clear failed");
        }
        if syscall::channel_respond(srv.channel, &response_buf[..resp_len]).is_err() {
            pw_log::error!("channel_respond failed");
        }
    }
}
