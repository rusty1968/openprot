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
use openprot_hal_blocking::i3c_hardware::{I3cTarget, TargetEvent};
use openprot_mctp_api::wire::{
    self, MctpOp, MctpRequestHeader, MAX_PAYLOAD_SIZE, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE,
};
use openprot_mctp_api::{Handle, ResponseCode};
use openprot_mctp_server::dispatch::{self as mctp_dispatch, DispatchOutcome};
use openprot_mctp_transport_i3c::{I3cSender, MctpI3cReceiver};
use pw_status::Result;
use userspace::process_entry;
use userspace::syscall::{self, Signals};
use userspace::time::{Clock, Duration, Instant, SystemClock};

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
    fn transact(&mut self, req: &[u8], resp: &mut [u8]) -> core::result::Result<usize, TransportError> {
        let mut srv = self.srv.borrow_mut();
        Ok(dispatch(&mut srv, req, resp))
    }
}

fn colo_loop() -> Result<()> {
    pw_log::info!("colocated i3c+mctp server starting");

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
    let i3c_receiver = MctpI3cReceiver::new(PEC);

    let mut server = openprot_mctp_server::Server::<_, 16>::new(mctp::Eid(OWN_EID), 0, sender);

    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut response_buf = [0u8; MAX_RESPONSE_SIZE];
    let mut recv_buf = [0u8; MAX_PAYLOAD_SIZE];
    let mut i3c_rx_buf = [0u8; I3C_RX_MAX];

    struct PendingRecv {
        handle: Handle,
        deadline: Instant,
    }
    let mut pending_recv: Option<PendingRecv> = None;

    // user_data=0 -> IPC from an MCTP client; user_data=1 -> the i3c IRQ.
    syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize)?;
    syscall::wait_group_add(handle::WG, handle::I3C_IRQ, signals::I3C, 1usize)?;

    let wait_mask = Signals::READABLE | signals::I3C;

    loop {
        let wait_deadline = pending_recv
            .as_ref()
            .map(|pending| pending.deadline)
            .unwrap_or(Instant::MAX);
        let ev = match syscall::object_wait(handle::WG, wait_mask, wait_deadline) {
            Ok(ev) => ev,
            Err(pw_status::Error::DeadlineExceeded) => {
                if pending_recv.take().is_some() {
                    let resp = wire::MctpResponseHeader::error(ResponseCode::TimedOut);
                    response_buf[..wire::MctpResponseHeader::SIZE]
                        .copy_from_slice(&resp.to_bytes());
                    let _ = syscall::channel_respond(
                        handle::MCTP,
                        &response_buf[..wire::MctpResponseHeader::SIZE],
                    );
                    let _ =
                        syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize);
                }
                continue;
            }
            Err(err) => return Err(err),
        };

        if ev.user_data == 1 {
            // ---- i3c IRQ: drain inbound frames into the ring (in-process) ----
            let acked = ev.pending_signals & signals::I3C;
            // Read the event first (releasing the RefCell borrow) so the latch
            // loop below can borrow the Server again without a double-borrow.
            let evt = srv.borrow_mut().target.on_interrupt();
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
            let _ = syscall::interrupt_ack(handle::I3C_IRQ, acked);

            // Drain every latched frame into the MCTP router (direct dispatch).
            loop {
                match i3c_rx_client.recv(&mut i3c_rx_buf) {
                    Ok(Some(n)) => {
                        if let Ok((pkt, _)) = i3c_receiver.decode(&i3c_rx_buf[..n]) {
                            let _ = server.inbound(pkt);
                        } else {
                            pw_log::error!("i3c frame decode failed");
                        }
                    }
                    _ => break,
                }
            }

            // Satisfy a deferred blocking-recv now that inbound data arrived.
            if let Some(pending) = pending_recv.as_ref() {
                if let Some(meta) = server.try_recv(pending.handle, &mut recv_buf) {
                    let payload = &recv_buf[..meta.payload_size];
                    let response_len = wire::encode_recv_response(
                        &mut response_buf,
                        meta.msg_type,
                        meta.msg_ic,
                        meta.remote_eid,
                        meta.msg_tag,
                        payload,
                    )
                    .unwrap_or_else(|_| {
                        wire::encode_error_response(&mut response_buf, ResponseCode::InternalError)
                            .unwrap_or(0)
                    });
                    syscall::channel_respond(handle::MCTP, &response_buf[..response_len])?;
                    pending_recv = None;
                    syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize)?;
                }
            }
        } else {
            // ---- MCTP app request (unchanged from the two-process server) ----
            let len = syscall::channel_read(handle::MCTP, 0, &mut request_buf)?;
            if pending_recv.is_some() {
                let resp = wire::MctpResponseHeader::error(ResponseCode::InternalError);
                response_buf[..wire::MctpResponseHeader::SIZE].copy_from_slice(&resp.to_bytes());
                syscall::channel_respond(
                    handle::MCTP,
                    &response_buf[..wire::MctpResponseHeader::SIZE],
                )?;
                continue;
            }

            if len < MctpRequestHeader::SIZE {
                let resp = wire::MctpResponseHeader::error(ResponseCode::BadArgument);
                response_buf[..wire::MctpResponseHeader::SIZE].copy_from_slice(&resp.to_bytes());
                syscall::channel_respond(
                    handle::MCTP,
                    &response_buf[..wire::MctpResponseHeader::SIZE],
                )?;
                continue;
            }

            if MctpRequestHeader::from_bytes(&request_buf[..len])
                .and_then(|h| h.operation())
                .is_some_and(|op| matches!(op, MctpOp::Recv))
            {
                let header = MctpRequestHeader::from_bytes(&request_buf[..len]).unwrap();
                let recv_handle = Handle(header.handle);
                let payload = wire::get_request_payload(&request_buf[..len]);
                if payload.len() < 4 {
                    let resp = wire::MctpResponseHeader::error(ResponseCode::BadArgument);
                    response_buf[..wire::MctpResponseHeader::SIZE]
                        .copy_from_slice(&resp.to_bytes());
                    syscall::channel_respond(
                        handle::MCTP,
                        &response_buf[..wire::MctpResponseHeader::SIZE],
                    )?;
                    continue;
                }

                let timeout_millis = u32::from_le_bytes(payload[..4].try_into().unwrap());
                match server.try_recv(recv_handle, &mut recv_buf) {
                    Some(meta) => {
                        let payload = &recv_buf[..meta.payload_size];
                        let response_len = wire::encode_recv_response(
                            &mut response_buf,
                            meta.msg_type,
                            meta.msg_ic,
                            meta.remote_eid,
                            meta.msg_tag,
                            payload,
                        )
                        .unwrap_or_else(|_| {
                            wire::encode_error_response(
                                &mut response_buf,
                                ResponseCode::InternalError,
                            )
                            .unwrap_or(0)
                        });
                        syscall::channel_respond(handle::MCTP, &response_buf[..response_len])?;
                    }
                    None => {
                        let deadline = if timeout_millis == 0 {
                            Instant::MAX
                        } else {
                            SystemClock::now()
                                .checked_add_duration(Duration::from_millis(timeout_millis as u64))
                                .unwrap_or(Instant::MAX)
                        };
                        pending_recv = Some(PendingRecv {
                            handle: recv_handle,
                            deadline,
                        });
                        let _ = syscall::wait_group_remove(handle::WG, handle::MCTP);
                    }
                }
            } else {
                let response_len = match mctp_dispatch::dispatch_mctp_op(
                    &request_buf[..len],
                    &mut response_buf,
                    &mut server,
                    &mut recv_buf,
                    0,
                ) {
                    DispatchOutcome::Reply(n) => n,
                    DispatchOutcome::Pending { .. } => unreachable!("Recv handled above"),
                };
                syscall::channel_respond(handle::MCTP, &response_buf[..response_len])?;
            }
        }
    }
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
