// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Pigweed server-runtime for an I3C target.
//!
//! Wraps the host-buildable [`i3c_server`] (target-owning `Server` + request
//! dispatch) in a `util_service::ServiceLoop`. A single IPC channel carries the
//! transport protocol ([`i3c_api`]); the I3C hardware IRQ is added to the loop
//! as a wake source, so a frame arriving on the bus and a client request are
//! both just events. The WaitGroup belongs to `util_service`; what a request
//! means belongs to the `Server`.
//!
//! ## Hot path
//!
//! - **IRQ** — the loop returns the I3C interrupt source; `on_interrupt` decodes
//!   it. On `InboundReady` the runtime drains every available frame into the
//!   server's ring, `interrupt_ack`s, and raises `USER` on the client channel so
//!   a parked client wakes. On `ResponseRead` the staged transmit was consumed
//!   by the controller, so the runtime lets the next `Send` through.
//! - **IPC** — the loop returns a request, already read; the runtime dispatches
//!   it and replies. A `Recv` that consumes a frame also updates `USER`: it stays
//!   asserted while frames remain and clears when the ring is empty; the IRQ path
//!   re-raises it when the next frame lands.
//!
//! ## Outbound back-pressure
//!
//! A `Send` stages a response for the controller's next private read. Only one
//! can be staged at a time: a `Send` while the previous one is still unread is
//! answered `Busy` by `i3c_server::dispatch` and nothing is staged. The client
//! retries after the controller has read it. The only thing that releases the
//! next `Send` is `ResponseRead`, which the i3c target derives from the hardware
//! `TxDescComplete` interrupt. A new inbound frame does **not** release it: a
//! controller may write again before it reads. This is also how caliptra-mcu-sw's
//! I3C driver gates outbound data (`ErrorCode::BUSY`).
//!
//! A controller that never reads would leave `Busy` set for good. So the runtime
//! gives each staged response [`STAGED_RESPONSE_TIMEOUT_MS`](i3c_api::STAGED_RESPONSE_TIMEOUT_MS):
//! when that passes without a `ResponseRead` it discards the response in the
//! target ([`Server::cancel_staged`]), logs it, and accepts `Send` again. The
//! discarded message is lost.
//!
//! The **only** kernel-tagged crate in the i3c server path.

#![no_std]

use i3c_api::{decode_request, I3cOp, MAX_FRAME, STAGED_RESPONSE_TIMEOUT_MS};
use i3c_server::{dispatch, Inbound, Server};
use openprot_hal_blocking::i3c_hardware::{I3cTarget, TargetEvent};
use pw_status::Result;
use userspace::syscall::{self, Signals};
use userspace::time::{Clock, Duration, Instant, SystemClock};
use util_service::{Event, ServiceLoop};

pub use util_service::Channel;

/// How long a staged response may go unread. See
/// [`STAGED_RESPONSE_TIMEOUT_MS`].
const STAGED_TIMEOUT: Duration = Duration::from_millis(STAGED_RESPONSE_TIMEOUT_MS as u64);

/// The part of serving an I3C target that does not depend on how its clients
/// reach it: servicing its interrupt, and the clock on its staged response.
///
/// [`run`] drives one of these from its loop. A process that runs the target in
/// the same thread as its client (so no loop of this crate's own) drives one
/// itself: call [`on_interrupt`](Self::on_interrupt) when the interrupt is
/// signalled, [`after_request`](Self::after_request) around each dispatched
/// request, and [`discard_if_expired`](Self::discard_if_expired) whenever it
/// might have waited too long.
pub struct TargetService {
    irq_signals: Signals,
    /// When the staged response, if any, runs out of time to be read.
    staged_deadline: Option<Instant>,
}

impl TargetService {
    /// A service for a target whose interrupt object signals `irq_signals`.
    pub const fn new(irq_signals: Signals) -> Self {
        Self {
            irq_signals,
            staged_deadline: None,
        }
    }

    /// Bring the target onto the bus and arm the events this service depends on.
    ///
    /// The only thing that releases the next `Send` is `ResponseRead`, so the
    /// target must report it. Call this before serving anything.
    pub fn enable<T: I3cTarget, const RING: usize>(&self, srv: &mut Server<T, RING>) {
        if srv.target.enable().is_err() {
            pw_log::error!("i3c target enable failed");
        }
        if srv.target.enable_response_read_events().is_err() {
            pw_log::error!("i3c response-read events enable failed");
        }
    }

    /// When to wake to discard the staged response, or [`Instant::MAX`] if none
    /// is staged. A wait's deadline is a hint only: it is honored when nothing
    /// else is ready, so call [`discard_if_expired`](Self::discard_if_expired)
    /// as well.
    pub fn deadline(&self) -> Instant {
        self.staged_deadline.unwrap_or(Instant::MAX)
    }

    /// Service the target's interrupt: decode it, queue any inbound frames, and
    /// let the next `Send` through if the controller read the staged response.
    ///
    /// This does not acknowledge the interrupt (see
    /// [`acknowledge`](Self::acknowledge)): the caller does, once it has dealt
    /// with queued frames ([`Server::has_frame`]). The interrupt is level
    /// triggered, so until it is acknowledged it stays signalled, and a loop
    /// waiting on it is woken again. A caller that cannot deal with the frames
    /// where it is (it is in the middle of something else) can leave the
    /// interrupt unacknowledged so that its main loop sees it.
    pub fn on_interrupt<T: I3cTarget, const RING: usize>(&mut self, srv: &mut Server<T, RING>) {
        match srv.target.on_interrupt() {
            Ok(TargetEvent::InboundReady) => {
                // Drain every frame the interrupt covers into the ring; a single
                // RX-descriptor interrupt can stand for more than one hardware
                // frame. Stop on Empty.
                let mut dropped = false;
                loop {
                    match srv.latch_inbound() {
                        Ok(Inbound::Latched) => continue,
                        Ok(Inbound::DroppedFull) => {
                            dropped = true;
                            continue;
                        }
                        Ok(Inbound::Empty) => break,
                        Err(_) => {
                            pw_log::error!("i3c read_frame failed");
                            break;
                        }
                    }
                }
                if dropped {
                    pw_log::error!("i3c inbound ring full; frame(s) dropped");
                }
            }
            Ok(TargetEvent::ResponseRead) => {
                // The controller read the staged response: the next `Send` can be
                // staged.
                srv.notify_response_read();
                self.staged_deadline = None;
            }
            Ok(_) => {}
            Err(_) => pw_log::error!("i3c on_interrupt failed"),
        }
    }

    /// Acknowledge the target's interrupt after [`on_interrupt`](Self::on_interrupt):
    /// it is level triggered, so until it is acknowledged it is signalled again
    /// at once.
    pub fn acknowledge<T, const RING: usize>(&self, srv: &Server<T, RING>) {
        if syscall::interrupt_ack(srv.irq, self.irq_signals).is_err() {
            pw_log::error!("interrupt_ack failed");
        }
    }

    /// Call after dispatching a request, with whether a response was staged
    /// *before* it ([`Server::tx_pending`]): if the request staged one, its clock
    /// starts.
    pub fn after_request<T, const RING: usize>(
        &mut self,
        srv: &Server<T, RING>,
        was_pending: bool,
    ) {
        if !was_pending && srv.tx_pending() {
            self.staged_deadline = Some(
                SystemClock::now()
                    .checked_add_duration(STAGED_TIMEOUT)
                    .unwrap_or(Instant::MAX),
            );
        }
    }

    /// Discard the staged response if its time is up, so the next `Send` is not
    /// refused for good.
    pub fn discard_if_expired<T: I3cTarget, const RING: usize>(
        &mut self,
        srv: &mut Server<T, RING>,
    ) {
        let Some(due) = self.staged_deadline else {
            return;
        };
        if SystemClock::now() < due {
            return;
        }
        self.staged_deadline = None;
        match srv.cancel_staged() {
            Ok(true) => pw_log::error!("i3c staged response not read in time; discarded"),
            Ok(false) => {}
            Err(_) => pw_log::error!("i3c cancel_response failed"),
        }
    }
}

/// Run the I3C server loop until a fatal error.
///
/// Serves the one IPC channel `srv.channel` and wakes on `irq_signals` of
/// `srv.irq`.
///
/// # Errors
///
/// Only setup and WaitGroup failures return; anything that goes wrong with one
/// request or one interrupt is logged and the loop carries on.
pub fn run<T: I3cTarget, const RING: usize>(
    wg: u32,
    irq_signals: Signals,
    srv: &mut Server<T, RING>,
) -> Result<()> {
    let mut target = TargetService::new(irq_signals);
    target.enable(srv);

    let mut channels = [Channel::new(srv.channel)];
    let mut service = ServiceLoop::new(wg, &mut channels)?;
    service.add_source(srv.irq, irq_signals)?;

    let mut request_buf = [0u8; MAX_FRAME];
    let mut response_buf = [0u8; MAX_FRAME];

    loop {
        // Check the clock here and not only through `next`'s deadline: `next`
        // returns `Deadline` only when nothing else is ready, and a client
        // retrying `Busy` faster than the scheduler tick always has a request
        // ready, so the deadline would never be seen and the staged response
        // never discarded.
        target.discard_if_expired(srv);

        match service.next(target.deadline(), &mut request_buf)? {
            // A staged response may have gone unread for too long.
            Event::Deadline => target.discard_if_expired(srv),

            // ---- I3C hardware IRQ: decode, latch an inbound frame, wake client ----
            Event::Source(_) => {
                target.on_interrupt(srv);
                // The loop does not acknowledge a source, and the interrupt is
                // level triggered: ack it or `next` returns it again at once.
                target.acknowledge(srv);
                if srv.has_frame() {
                    // ORs USER onto the client channel without disturbing READABLE.
                    if syscall::object_set_peer_user_signal(srv.channel, true).is_err() {
                        pw_log::error!("object_set_peer_user_signal failed");
                    }
                }
            }

            // ---- client IPC request ----
            Event::Request { channel, len } => {
                let request = request_buf.get(..len).unwrap_or(&[]);
                let is_recv = decode_request(request).map(|(op, _)| op) == Some(I3cOp::Recv);
                let was_pending = srv.tx_pending();
                let resp_len = dispatch(srv, request, &mut response_buf);
                target.after_request(srv, was_pending);
                // A Recv pops one frame from the ring. Keep USER asserted while
                // frames remain so the client drains the rest; clear it only when
                // the ring is empty (the IRQ path re-raises it when the next frame
                // arrives). A blind clear here would park the client with frames
                // still queued.
                if is_recv
                    && syscall::object_set_peer_user_signal(srv.channel, srv.has_frame()).is_err()
                {
                    pw_log::error!("object_set_peer_user_signal update failed");
                }
                if service
                    .reply(channel, response_buf.get(..resp_len).unwrap_or(&[]))
                    .is_err()
                {
                    pw_log::error!("channel_respond failed");
                }
            }
        }
    }
}
