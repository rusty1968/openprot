// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Colocated i3c + MCTP server — the no-isolation experiment.
//!
//! Identical in behaviour to running `i3c_server` and `mctp_server` as two
//! processes, but merged into ONE: the MCTP router reaches the I3C target
//! through an in-process [`Transport`] that calls `i3c_server::dispatch`
//! directly (shared via a `RefCell`), instead of a pw_kernel channel. The
//! `mctp <-> i3c` IPC boundary — a `channel_transact` (SyscallBuffer cross-
//! process copy + syscall + context switches) crossed once per i3c fragment —
//! is gone. Compare the throughput this yields against the two-process
//! `i3c_throughput` to price that boundary.
//!
//! Only the i3c<->mctp boundary is removed; the app still talks to this process
//! over the `mctp` channel, so the sink stays a separate, isolated process.
//!
//! Outbound back-pressure is intentionally dropped here: the sink's acks are a
//! single fragment, read by the host before the next, so there is no multi-TX
//! overrun to guard against in this benchmark.

#![no_main]
#![no_std]

use core::cell::RefCell;

use caliptra_i3c_target::CaliptraI3cTarget;
use i3c_api::{Transport, TransportError};
use i3c_client::I3cClient;
use i3c_server::{dispatch, Inbound, Server};
use mctp_i3c_colo_codegen::{handle, signals};
use mctp_server_runtime::Channel;
use openprot_hal_blocking::i3c_hardware::{I3cTarget, TargetEvent};
use openprot_mctp_transport_i3c::{transfer_pec, I3cSender, MctpI3cReceiver};
use pw_status::Result;
use userspace::process_entry;
use userspace::syscall;

const OWN_EID: u8 = 8;
const REMOTE_I3C_ADDR: u8 = 0x0a;
const PEC: bool = true;
const I3C_RX_MAX: usize = i3c_api::MAX_PAYLOAD;

/// In-process i3c transport: `dispatch` straight against the shared [`Server`],
/// no channel. This is the seam that replaces the `mctp <-> i3c` IPC boundary.
struct ColoTransport<'a> {
    srv: &'a RefCell<Server<CaliptraI3cTarget>>,
}

impl Transport for ColoTransport<'_> {
    fn transact(
        &mut self,
        req: &[u8],
        resp: &mut [u8],
    ) -> core::result::Result<usize, TransportError> {
        let mut srv = self.srv.borrow_mut();
        Ok(dispatch(&mut srv, req, resp))
    }
}

fn colo_loop() -> Result<()> {
    pw_log::info!("colocated i3c+mctp server starting (dsp0233 framing, diag build 6)");

    // SAFETY: this process exclusively owns the I3C peripheral, mapped as a
    // device region by system.json5; Caliptra ROM already initialized the core.
    let target = unsafe { CaliptraI3cTarget::new() };
    // channel handle unused in colocated mode; the IRQ handle is real.
    let srv = RefCell::new(Server::new(0, handle::I3C_IRQ, target));
    let _ = srv.borrow_mut().target.enable();

    let sender = I3cSender::new(
        I3cClient::new(ColoTransport { srv: &srv }),
        REMOTE_I3C_ADDR,
        PEC,
    );
    let mut i3c_rx_client = I3cClient::new(ColoTransport { srv: &srv });
    let mut i3c_receiver = MctpI3cReceiver::new(PEC);

    let mut server = openprot_mctp_server::Server::<_, 16>::new(mctp::Eid(OWN_EID), 0, sender);
    let mut i3c_rx_buf = [0u8; I3C_RX_MAX];
    let mut channels = [Channel::new(handle::MCTP)];
    // Diagnostics (local): how many events/packets to log.
    let mut diag_budget = 120u32;

    // The MCTP side of the loop is the shared runtime. Its transport wake
    // source here is the i3c IRQ itself, so the callback does the i3c
    // server's interrupt work in-process before feeding the router.
    mctp_server_runtime::run(
        handle::WG,
        &mut channels,
        handle::I3C_IRQ,
        signals::I3C,
        &mut server,
        |server| {
            // ---- i3c IRQ: drain inbound frames into the ring (in-process) ----
            // Read the event first (releasing the RefCell borrow) so the latch
            // loop below can borrow the Server again without a double-borrow.
            let evt = srv.borrow_mut().target.on_interrupt();
            // Only unexpected events are logged; the common ones would use up
            // the diagnostic budget before the transfer starts.
            match evt {
                Ok(TargetEvent::InboundReady) | Ok(TargetEvent::ResponseRead) => {}
                Ok(_) => pw_log::info!("i3c diag: irq event other"),
                Err(_) => pw_log::info!("i3c diag: irq event error"),
            }
            match evt {
                Ok(TargetEvent::InboundReady) => loop {
                    match srv.borrow_mut().latch_inbound() {
                        Ok(Inbound::Latched) => continue,
                        Ok(Inbound::DroppedFull) => {
                            pw_log::error!("colo i3c inbound ring full; frame dropped");
                            break;
                        }
                        Ok(Inbound::Empty) => break,
                        Err(_) => {
                            pw_log::error!("colo i3c read_frame failed");
                            break;
                        }
                    }
                },
                Ok(TargetEvent::ResponseRead) => srv.borrow_mut().notify_response_read(),
                _ => {}
            }
            // Acking is what quiets this wake source for the runtime.
            let _ = syscall::interrupt_ack(handle::I3C_IRQ, signals::I3C);

            // Drain every latched frame into the MCTP router (direct dispatch).
            while let Ok(Some(n)) = i3c_rx_client.recv(&mut i3c_rx_buf) {
                let decoded = i3c_rx_buf.get(..n).map(|frame| {
                    i3c_receiver.decode(frame, || i3c_rx_client.dynamic_address().ok().flatten())
                });
                if let Some(Ok(pkt)) = decoded {
                    let res = server.inbound(pkt);
                    if diag_budget > 0 {
                        diag_budget -= 1;
                        let byte = |i: usize| pkt.get(i).copied().unwrap_or(0) as u32;
                        pw_log::info!(
                            "i3c diag: rx pkt len={} dest={} src={} flags=0x{:02x} type=0x{:02x} inbound_ok={}",
                            pkt.len() as u32,
                            byte(1),
                            byte(2),
                            byte(3),
                            byte(4),
                            res.is_ok() as u32
                        );
                        let r = srv.borrow().target.debug_regs();
                        pw_log::info!(
                            "i3c regs: irq=0x{:08x} qs=0x{:08x} desc=0x{:08x} data=0x{:08x}",
                            r[0],
                            r[1],
                            r[2],
                            r[3]
                        );
                        pw_log::info!(
                            "i3c regs: err=0x{:08x} st=0x{:08x} framing={} ri_pec={}",
                            r[4],
                            r[5],
                            r[6],
                            r[7]
                        );
                    }
                } else {
                    pw_log::error!("i3c frame decode failed");
                    // Diagnostics (local): what arrived and what PEC we expected.
                    if let Some(frame) = i3c_rx_buf.get(..n) {
                        let own = i3c_rx_client.dynamic_address().ok().flatten();
                        let body_len = frame.len().saturating_sub(1);
                        let body = frame.get(..body_len).unwrap_or(&[]);
                        let rx_pec = frame.get(body_len).copied().unwrap_or(0);
                        let expect = own.map(|a| transfer_pec(a << 1, body)).unwrap_or(0);
                        pw_log::error!(
                            "i3c diag: len={} own_addr={} pec_rx=0x{:02x} pec_expect=0x{:02x}",
                            n as u32,
                            own.map(|a| a as u32).unwrap_or(0xffff),
                            rx_pec as u32,
                            expect as u32
                        );
                        let b = |i: usize| frame.get(i).copied().unwrap_or(0) as u32;
                        pw_log::error!(
                            "i3c diag: bytes {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                            b(0),
                            b(1),
                            b(2),
                            b(3),
                            b(4),
                            b(5),
                            b(6),
                            b(7)
                        );
                    }
                }
            }
        },
    )
}

#[process_entry("mctp_i3c_colo")]
fn entry() {
    if let Err(_e) = colo_loop() {
        pw_log::error!("colocated server exiting with error");
        let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
    }
    #[expect(clippy::empty_loop)]
    loop {}
}
