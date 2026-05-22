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
//! The IPC handles are provided by the `app_package` Bazel rule, which
//! generates `app_mctp_server::handle::*` from the system configuration.

#![no_main]
#![no_std]

use i2c_api::SlaveEventKind;
use i2c_client::I2cClient;
use i2c_client_ipc::IpcTransport;
use openprot_mctp_api::wire::{MctpRequestHeader, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE, MAX_PAYLOAD_SIZE};
use openprot_mctp_api::ResponseCode;
use openprot_mctp_server::dispatch;
use openprot_mctp_transport_i2c::{I2cSender, MctpI2cReceiver};

use pw_status::Error;
use pw_status::Result;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

use app_mctp_server::handle;

const OWN_EID: u8 = 8;
const OWN_I2C_ADDR: u8 = 0x10;
const REMOTE_I2C_ADDR: u8 = 0x42;
const I2C_RX_MAX: usize = MAX_PAYLOAD_SIZE;

// ---------------------------------------------------------------------------
// Server loop
// ---------------------------------------------------------------------------

fn mctp_server_loop() -> Result<()> {
    pw_log::info!("MCTP server starting");
    let sender = I2cSender::new(
        I2cClient::new(IpcTransport::new(handle::I2C)),
        OWN_I2C_ADDR,
        REMOTE_I2C_ADDR,
    );
    let mut i2c_rx_client = I2cClient::new(IpcTransport::new(handle::I2C));
    let i2c_receiver = MctpI2cReceiver::new(OWN_I2C_ADDR);

    if i2c_rx_client.configure_slave(OWN_I2C_ADDR).is_err() {
        pw_log::error!("configure_slave failed");
        return Err(Error::Internal);
    }
    if i2c_rx_client.enable_slave().is_err() {
        pw_log::error!("enable_slave failed");
        return Err(Error::Internal);
    }
    if i2c_rx_client.enable_notification().is_err() {
        pw_log::error!("enable_notification failed");
        return Err(Error::Internal);
    }

    let mut server = openprot_mctp_server::Server::<_, 16>::new(
        mctp::Eid(OWN_EID),
        0,
        sender,
    );

    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut response_buf = [0u8; MAX_RESPONSE_SIZE];
    let mut recv_buf = [0u8; MAX_PAYLOAD_SIZE];
    let mut i2c_rx_buf = [0u8; I2C_RX_MAX];

    // Register event sources with the WaitGroup.
    // user_data=0 → IPC from a client  (MCTP channel READABLE)
    // user_data=1 → I2C USER signal (slave data latched by i2c server)
    syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize)?;
    syscall::wait_group_add(handle::WG, handle::I2C, Signals::USER, 1usize)?;

    loop {
        let ev = syscall::object_wait(handle::WG, Signals::READABLE | Signals::USER, Instant::MAX)?;

        if ev.user_data == 1 {
            // Inbound i2c data: fetch latched payload + metadata and feed to router.
            match i2c_rx_client.slave_receive(&mut i2c_rx_buf) {
                Ok(event) => {
                    if event.kind == SlaveEventKind::DataReceived && event.data_len > 0 {
                        if let Ok((pkt, _)) = i2c_receiver.decode(&i2c_rx_buf[..event.data_len]) {
                            let _ = server.inbound(pkt);
                        } else {
                            pw_log::error!("i2c frame decode failed");
                        }
                    }
                }
                Err(_) => {
                    pw_log::error!("slave_receive failed");
                }
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
fn entry() {
    if let Err(e) = mctp_server_loop() {
        pw_log::error!("mctp_server exiting with error");
        let _ = syscall::process_exit(e as u32);
    }
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
