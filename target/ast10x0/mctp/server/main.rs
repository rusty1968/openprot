// Licensed under the Apache-2.0 license

//! MCTP Server — IPC Dispatch Loop
//!
//! Userspace service that receives MCTP requests over a Pigweed IPC channel,
//! dispatches them to the MCTP server core, and responds with results.
//!
//! # Architecture
//!
//! ```text
//! ┌─ Client ──────────────────────────┐
//! │ channel_transact(request)         │
//! └──────────────┬────────────────────┘
//!                │ IPC channel
//!                ▼
//! ┌─ This Server ─────────────────────┐
//! │ object_wait(READABLE)             │
//! │ channel_read → MctpRequestHeader  │
//! │ dispatch_mctp_op(op, server)      │
//! │ channel_respond ← MctpRespHeader  │
//! └──────────────┬────────────────────┘
//!                │ mctp-stack Router
//!                ▼
//! ┌─ I2C Transport ──────────────────┐
//! │ I2cSender → I2C Server IPC      │
//! └──────────────────────────────────┘
//! ```
//!
//! # IPC Pattern
//!
//! Follows the same loop as `services/i2c/server/src/main.rs`:
//!
//! 1. `object_wait(handle, READABLE)` — block until a client sends a request
//! 2. `channel_read(handle)` — read the raw request bytes
//! 3. Parse `MctpRequestHeader`, dispatch via `dispatch_mctp_op`
//! 4. `channel_respond(handle)` — send response header + data
//!
//! # Handle Binding
//!
//! The IPC handle is provided by the `app_package` Bazel rule, which generates
//! `app_mctp_server::handle::MCTP` from the system configuration.

#![no_main]
#![no_std]

use core::{cell::RefCell, ops::DerefMut};

use ast10x0_serial_direct::Ast10x0DirectSerial;
use embedded_io::{Read, Write};
use mctp_lib::Sender;
use openprot_mctp_api::wire::{MctpRequestHeader, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE, MAX_PAYLOAD_SIZE};
use openprot_mctp_api::ResponseCode;
use openprot_mctp_server::dispatch;

use pw_status::Result;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

use app_mctp_server::{handle, signals};

const OWN_EID: u8 = 8;

struct SerialSender<'a, T> {
    serial: &'a RefCell<T>,
    serial_handler: mctp_lib::serial::MctpSerialHandler,
}

impl<'a, T: Read + Write> SerialSender<'a, T> {
    fn new(serial: &'a RefCell<T>) -> Self {
        Self {
            serial,
            serial_handler: mctp_lib::serial::MctpSerialHandler::new(),
        }
    }
}

impl<T: Read + Write> Sender for SerialSender<'_, T> {
    fn send_vectored(
        &mut self,
        mut fragmenter: mctp_lib::fragment::Fragmenter,
        payload: &[&[u8]],
    ) -> mctp::Result<mctp::Tag> {
        loop {
            let mut pkt = [0u8; mctp_lib::serial::MTU_MAX];
            match fragmenter.fragment_vectored(payload, &mut pkt) {
                mctp_lib::fragment::SendOutput::Packet(p) => {
                    self.serial_handler
                        .send_sync(p, &mut self.serial.borrow_mut().deref_mut())?;
                    self.serial
                        .borrow_mut()
                        .flush()
                        .map_err(|_| mctp::Error::TxFailure)?;
                }
                mctp_lib::fragment::SendOutput::Complete { tag, .. } => break Ok(tag),
                mctp_lib::fragment::SendOutput::Error { err, .. } => break Err(err),
            }
        }
    }

    fn get_mtu(&self) -> usize {
        mctp_lib::serial::MTU_MAX
    }
}

// ---------------------------------------------------------------------------
// Server loop
// ---------------------------------------------------------------------------

fn mctp_server_loop() -> Result<()> {
    pw_log::info!("MCTP server starting");

    let serial = RefCell::new(Ast10x0DirectSerial::new_uart5());
    serial.borrow().enable_rx_data_available_interrupt();

    let sender = SerialSender::<Ast10x0DirectSerial>::new(&serial);
    let mut serial_reader = mctp_lib::serial::MctpSerialHandler::new();
    let mut server = openprot_mctp_server::Server::<_, 16>::new(
        mctp::Eid(OWN_EID),
        0,
        sender,
    );

    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut response_buf = [0u8; MAX_RESPONSE_SIZE];
    let mut recv_buf = [0u8; MAX_PAYLOAD_SIZE];

    // Register event sources with the WaitGroup.
    // user_data=0 → IPC from a client  (MCTP channel READABLE)
    // user_data=1 → UART interrupt notification (SERIAL IRQ)
    syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize)?;
    syscall::wait_group_add(handle::WG, handle::UART_IRQ, signals::UART, 1usize)?;

    loop {
        let ev = syscall::object_wait(handle::WG, Signals::READABLE, Instant::MAX)?;

        if ev.user_data == 1 {
            // Inbound serial data: parse framed packet and feed to router.
            if let Ok(pkt) = serial_reader.recv_sync(&mut serial.borrow_mut().deref_mut()) {
                let _ = server.inbound(pkt);
            }
        } else {
            // IPC from a client — channel_read is non-blocking here because
            // the WaitGroup only fires after READABLE is set.
            let len = syscall::channel_read(handle::MCTP, 0, &mut request_buf)?;

            if len < MctpRequestHeader::SIZE {
                // Truncated request — respond with error
                let resp = openprot_mctp_api::wire::MctpResponseHeader::error(ResponseCode::BadArgument);
                response_buf[..openprot_mctp_api::wire::MctpResponseHeader::SIZE]
                    .copy_from_slice(&resp.to_bytes());
                syscall::channel_respond(
                    handle::MCTP,
                    &response_buf[..openprot_mctp_api::wire::MctpResponseHeader::SIZE],
                )?;
                continue;
            }

            // Dispatch and respond
            let response_len = dispatch::dispatch_mctp_op(
                &request_buf[..len],
                &mut response_buf,
                &mut server,
                &mut recv_buf,
            );
            syscall::channel_respond(handle::MCTP, &response_buf[..response_len])?;
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[entry]
fn entry() -> ! {
    if let Err(e) = mctp_server_loop() {
        ast10x0_userspace_runtime::fail_stop("mctp_server", e as u32);
    }
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
