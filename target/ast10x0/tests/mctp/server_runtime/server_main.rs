// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Server under test for the server-runtime QEMU test.
//!
//! Runs the real `mctp_server_runtime::run()` loop wired to a `Sender` that
//! stashes the last outbound frame instead of writing to I2C; `on_transport`
//! replays it into `Server::inbound`, simulating "a peer echoed the packet"
//! with no wire. See client_main.rs for the driving test cases.

#![no_std]
#![no_main]

use core::cell::RefCell;

use mctp_lib::fragment::{Fragmenter, SendOutput};
use mctp_server_runtime::Channel;
use openprot_mctp_server::{Sender, Server};
use pw_status::Result;
use userspace::{entry, syscall::Signals};

use app_server_test::handle;

const OWN_EID: u8 = 8;
/// Comfortably larger than any test payload; nothing here ever fragments.
const LOOPBACK_PKT_MAX: usize = 256;

/// Captures the single most recent outbound MCTP packet instead of writing
/// to a bus.
struct LoopbackSender<'a> {
    packet: &'a RefCell<Option<heapless::Vec<u8, LOOPBACK_PKT_MAX>>>,
}

impl Sender for LoopbackSender<'_> {
    fn send_vectored(
        &mut self,
        mut fragmenter: Fragmenter,
        payload: &[&[u8]],
    ) -> mctp::Result<mctp::Tag> {
        loop {
            let mut buf = [0u8; LOOPBACK_PKT_MAX];
            match fragmenter.fragment_vectored(payload, &mut buf) {
                SendOutput::Packet(p) => {
                    let mut pkt = heapless::Vec::new();
                    // Test payloads are well under LOOPBACK_PKT_MAX; if this
                    // ever fires it's a test bug, not a runtime error.
                    let _ = pkt.extend_from_slice(p);
                    *self.packet.borrow_mut() = Some(pkt);
                }
                SendOutput::Complete { tag, .. } => return Ok(tag),
                SendOutput::Error { err, .. } => return Err(err),
            }
        }
    }

    fn get_mtu(&self) -> usize {
        LOOPBACK_PKT_MAX - 4
    }
}

fn server_loop() -> Result<()> {
    pw_log::info!("server-runtime test: starting");

    let packet: RefCell<Option<heapless::Vec<u8, LOOPBACK_PKT_MAX>>> = RefCell::new(None);
    let sender = LoopbackSender { packet: &packet };
    let mut server = Server::<_, 16>::new(mctp::Eid(OWN_EID), 0, sender);
    let mut channels = [Channel::new(handle::MCTP_A), Channel::new(handle::MCTP_B)];

    mctp_server_runtime::run(
        handle::WG,
        &mut channels,
        handle::TRANSPORT,
        Signals::USER,
        &mut server,
        |server| {
            if let Some(pkt) = packet.borrow_mut().take() {
                let _ = server.inbound(&pkt);
            }
        },
    )
}

#[entry]
fn entry() {
    if server_loop().is_err() {
        pw_log::error!("server-runtime test: server exited with error");
    }
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
