// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! The i3c server and the MCTP server in ONE process: the no-isolation
//! experiment.
//!
//! Behaves like running `i3c_server` and `mctp_server` as two processes, but the
//! MCTP router reaches the I3C target through an in-process
//! [`Transport`](i3c_api::Transport) that calls `i3c_server::dispatch` directly,
//! instead of a pw_kernel channel. The `mctp <-> i3c` IPC boundary (a
//! `channel_transact`: a cross-process copy, a syscall and context switches,
//! crossed once per i3c fragment) is gone. Compare the throughput this yields
//! against the two-process image to price that boundary. Only that boundary goes:
//! the application still reaches this process over the `mctp` channel, so it
//! stays a separate, isolated process.
//!
//! The tradeoff is what the boundary was for: fault and privilege isolation
//! between the I3C driver and the MCTP stack, which a Root of Trust wants. This
//! prices it; it is not a recommendation to merge the two.
//!
//! # One thread does both jobs
//!
//! In the two-process image the i3c server's own loop services the controller's
//! interrupt, so a `Send` that is `Busy` (the controller has not read the previous
//! fragment) is released by that other process. Here nothing else would service
//! it: a sender that only slept while `Busy` would never see `ResponseRead`, and
//! every fragment after the first would be discarded by the staged-response
//! timeout. So the sender's wait hook services the target itself, polling its
//! status after each sleep, and leaves the interrupt unacknowledged if that
//! latched inbound frames, so the main loop sees it and forwards them.

#![no_std]

use core::cell::RefCell;

use caliptra_i3c_target::CaliptraI3cTarget;
use i3c_api::{Transport, TransportError};
use i3c_client::I3cClient;
use i3c_server::{dispatch, Server as I3cServer};
use i3c_server_runtime::TargetService;
use mctp_server_runtime::Channel;
use openprot_mctp_server::Server as MctpServer;
use openprot_mctp_transport_i3c::{I3cSender, MctpI3cReceiver, MAX_BUSY_RETRIES};
use pw_status::Result;
use userspace::syscall::Signals;
use userspace::time::{self, Clock, Duration, Instant, SystemClock};

/// This endpoint's MCTP EID.
const OWN_EID: u8 = 8;
/// The controller's 7-bit I3C address. DSP0233 carries no destination, so it is
/// not on the wire; the sender keeps it for diagnostics.
const REMOTE_I3C_ADDR: u8 = 0x0a;
/// How long the sender waits between attempts while the previous fragment is
/// unread.
const BUSY_WAIT_MS: u64 = 5;

// The retries must outlast the staged-response timeout, or a controller that
// stops reading would make every send fail, not just lose one message.
const _: () =
    assert!(BUSY_WAIT_MS * MAX_BUSY_RETRIES as u64 > i3c_api::STAGED_RESPONSE_TIMEOUT_MS as u64);

/// The i3c server and its service, shared by the transports and the wait hook.
/// One thread uses them, one borrow at a time.
struct Shared {
    srv: RefCell<I3cServer<CaliptraI3cTarget>>,
    service: RefCell<TargetService>,
}

/// In-process i3c transport: `dispatch` straight against the shared server, no
/// channel. This is the seam that replaces the `mctp <-> i3c` IPC boundary.
struct ColoTransport<'a> {
    shared: &'a Shared,
}

impl Transport for ColoTransport<'_> {
    fn transact(
        &mut self,
        req: &[u8],
        resp: &mut [u8],
    ) -> core::result::Result<usize, TransportError> {
        let mut srv = self.shared.srv.borrow_mut();
        let was_pending = srv.tx_pending();
        let len = dispatch(&mut srv, req, resp);
        self.shared
            .service
            .borrow_mut()
            .after_request(&srv, was_pending);
        Ok(len)
    }
}

/// Which handles the process was given by its system config.
#[derive(Clone, Copy)]
pub struct Handles {
    /// The WaitGroup the MCTP runtime waits on.
    pub wg: u32,
    /// The I3C interrupt object, and the signals it raises.
    pub i3c_irq: u32,
    pub i3c_signals: Signals,
    /// The channel the application reaches the MCTP server on.
    pub mctp: u32,
}

/// Run the colocated server. Only setup and WaitGroup failures return.
pub fn run(handles: Handles) -> Result<()> {
    pw_log::info!("colocated i3c+mctp server starting");

    // SAFETY: this process exclusively owns the I3C peripheral, mapped as a
    // device region by the system config; Caliptra ROM already initialized the
    // core.
    let target = unsafe { CaliptraI3cTarget::new() };
    // No i3c IPC channel in this process: the channel handle is unused, the
    // interrupt handle is the real one.
    let shared = Shared {
        srv: RefCell::new(I3cServer::new(0, handles.i3c_irq, target)),
        service: RefCell::new(TargetService::new(handles.i3c_signals)),
    };
    shared.service.borrow().enable(&mut shared.srv.borrow_mut());

    // Nothing else services the interrupt while a send waits for the controller
    // to read the previous fragment, so the wait does it.
    let wait = || {
        let deadline = SystemClock::now()
            .checked_add_duration(Duration::from_millis(BUSY_WAIT_MS))
            .unwrap_or(Instant::MAX);
        let _ = time::sleep_until(deadline);

        let mut srv = shared.srv.borrow_mut();
        let mut service = shared.service.borrow_mut();
        service.on_interrupt(&mut srv);
        service.discard_if_expired(&mut srv);
        // Frames latched here are for the main loop to forward, which it does
        // when the interrupt is signalled: leave it unacknowledged.
        if !srv.has_frame() {
            service.acknowledge(&srv);
        }
    };

    let sender = I3cSender::new(
        I3cClient::new(ColoTransport { shared: &shared }),
        REMOTE_I3C_ADDR,
        wait,
    );
    let mut rx_client = I3cClient::new(ColoTransport { shared: &shared });
    let mut receiver = MctpI3cReceiver::new();

    let mut server = MctpServer::<_, 16>::new(mctp::Eid(OWN_EID), 0, sender);
    let mut rx_buf = [0u8; i3c_api::MAX_PAYLOAD];
    let mut channels = [Channel::new(handles.mctp)];

    // The MCTP side of the loop is the shared runtime. Its transport wake source
    // here is the i3c interrupt itself, so the callback does the i3c server's
    // interrupt work in-process before feeding the router.
    mctp_server_runtime::run(
        handles.wg,
        &mut channels,
        handles.i3c_irq,
        handles.i3c_signals,
        &mut server,
        |server| {
            {
                let mut srv = shared.srv.borrow_mut();
                let mut service = shared.service.borrow_mut();
                service.on_interrupt(&mut srv);
                // Acknowledging is what quiets this wake source: the frames are
                // forwarded below.
                service.acknowledge(&srv);
                service.discard_if_expired(&mut srv);
            }

            // Forward every latched frame to the MCTP router.
            while let Ok(Some(n)) = rx_client.recv(&mut rx_buf) {
                let decoded = rx_buf.get(..n).map(|frame| {
                    receiver.decode(frame, || rx_client.dynamic_address().ok().flatten())
                });
                match decoded {
                    Some(Ok(packet)) => {
                        let _ = server.inbound(packet);
                    }
                    _ => {
                        // What the frame looked like, and the controller's own
                        // descriptor for it (its error flags are the top bits).
                        let frame = rx_buf.get(..n).unwrap_or(&[]);
                        let at = |i: usize| frame.get(i).copied().unwrap_or(0) as u32;
                        pw_log::error!(
                            "i3c frame decode failed: len=0x{:04x} first=0x{:02x}{:02x}{:02x}{:02x} last=0x{:02x} desc=0x{:08x}",
                            n as u32,
                            at(0),
                            at(1),
                            at(2),
                            at(3),
                            at(n.saturating_sub(1)),
                            shared.srv.borrow().target.last_rx_descriptor()
                        );
                    }
                }
            }
        },
    )
}
