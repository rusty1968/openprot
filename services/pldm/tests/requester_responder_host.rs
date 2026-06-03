// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host integration test for PLDM requester/responder using in-memory MCTP transfer.
//!
//! This test mirrors the DirectClient strategy used by MCTP echo host tests:
//! two in-memory MCTP servers exchange packets via shared Vec-backed buffers.
//! The requester side uses a DirectClient variant with a pre-recv hook so it
//! can pump packets to the responder and back before polling for a response.

use core::cell::{Cell, RefCell};

use mctp::{Eid, Tag};
use mctp_lib::fragment::{Fragmenter, SendOutput};
use mctp_lib::Sender;
use openprot_mctp_api::{Handle, MctpClient, MctpError, RecvMetadata, ResponseCode};
use openprot_mctp_server::Server;
use openprot_pldm_service::{MctpPldmTransport, PldmRequester, PldmResponder};
use pldm_common::protocol::base::{PldmControlCmd, PldmSupportedType};
use pldm_interface::control_context::ProtocolCapability;

const REQUESTER_EID: u8 = 8;
const RESPONDER_EID: u8 = 42;
const TIMEOUT_MILLIS: u32 = 0;

const CTRL_CMDS: [u8; 5] = [
    PldmControlCmd::SetTid as u8,
    PldmControlCmd::GetTid as u8,
    PldmControlCmd::GetPldmCommands as u8,
    PldmControlCmd::GetPldmVersion as u8,
    PldmControlCmd::GetPldmTypes as u8,
];

static CAPS: [ProtocolCapability<'static>; 1] = [ProtocolCapability {
    pldm_type: PldmSupportedType::Base,
    protocol_version: 0xF1F1F000, // "1.1.0" BCD-encoded
    supported_commands: &CTRL_CMDS,
}];

struct BufferSender<'a> {
    packets: &'a RefCell<Vec<Vec<u8>>>,
}

impl Sender for BufferSender<'_> {
    fn send_vectored(
        &mut self,
        mut fragmenter: Fragmenter,
        payload: &[&[u8]],
    ) -> mctp::Result<Tag> {
        loop {
            let mut buf = [0u8; 255];
            match fragmenter.fragment_vectored(payload, &mut buf) {
                SendOutput::Packet(p) => self.packets.borrow_mut().push(p.to_vec()),
                SendOutput::Complete { tag, .. } => return Ok(tag),
                SendOutput::Error { err, .. } => return Err(err),
            }
        }
    }

    fn get_mtu(&self) -> usize {
        255
    }
}

fn transfer<S: Sender, const N: usize>(packets: &RefCell<Vec<Vec<u8>>>, dest: &mut Server<S, N>) {
    let pkts = packets.borrow();
    for pkt in pkts.iter() {
        dest.inbound(pkt).expect("inbound should accept packet");
    }
}

struct DirectClientWithPump<'a, S: Sender, const N: usize, F: FnMut()> {
    server: &'a RefCell<Server<S, N>>,
    pre_recv_pump: RefCell<F>,
}

impl<'a, S: Sender, const N: usize, F: FnMut()> DirectClientWithPump<'a, S, N, F> {
    fn new(server: &'a RefCell<Server<S, N>>, pre_recv_pump: F) -> Self {
        Self {
            server,
            pre_recv_pump: RefCell::new(pre_recv_pump),
        }
    }
}

impl<S: Sender, const N: usize, F: FnMut()> MctpClient for DirectClientWithPump<'_, S, N, F> {
    fn req(&self, eid: u8) -> Result<Handle, MctpError> {
        self.server.borrow_mut().req(eid)
    }

    fn listener(&self, msg_type: u8) -> Result<Handle, MctpError> {
        self.server.borrow_mut().listener(msg_type)
    }

    fn get_eid(&self) -> u8 {
        self.server.borrow().get_eid()
    }

    fn set_eid(&self, eid: u8) -> Result<(), MctpError> {
        self.server.borrow_mut().set_eid(eid)
    }

    fn recv(
        &self,
        handle: Handle,
        _timeout_millis: u32,
        buf: &mut [u8],
    ) -> Result<RecvMetadata, MctpError> {
        // recv is the scheduling point for the in-memory test: run the pump to
        // move packets across the link *after* this endpoint has registered its
        // listener/request, then poll the local server. (mctp-lib discards
        // inbound packets that arrive with no matching listener, so delivery
        // must happen here, not before recv.)
        (self.pre_recv_pump.borrow_mut())();

        self.server
            .borrow_mut()
            .try_recv(handle, buf)
            .ok_or(MctpError::from_code(ResponseCode::TimedOut))
    }

    fn send(
        &self,
        handle: Option<Handle>,
        msg_type: u8,
        eid: Option<u8>,
        tag: Option<u8>,
        integrity_check: bool,
        buf: &[u8],
    ) -> Result<u8, MctpError> {
        self.server
            .borrow_mut()
            .send(handle, msg_type, eid, tag, integrity_check, buf)
    }

    fn drop_handle(&self, handle: Handle) {
        let _ = self.server.borrow_mut().unbind(handle);
    }
}

#[test]
fn requester_to_responder_in_memory_via_directclient() {
    // Build two isolated in-memory MCTP endpoints: requester EID 8 and
    // responder EID 42. Each one captures outbound packets into a Vec.
    let req_packets = RefCell::new(Vec::new());
    let req_sender = BufferSender {
        packets: &req_packets,
    };
    let requester_server: RefCell<Server<_, 16>> =
        RefCell::new(Server::new(Eid(REQUESTER_EID), 0, req_sender));

    let rsp_packets = RefCell::new(Vec::new());
    let rsp_sender = BufferSender {
        packets: &rsp_packets,
    };
    let responder_server: RefCell<Server<_, 16>> =
        RefCell::new(Server::new(Eid(RESPONDER_EID), 0, rsp_sender));

    // The responder's MCTP client pumps the in-memory link right before each
    // try_recv, i.e. *after* run_once has registered the PLDM listener. The
    // request packets must not be delivered any earlier or mctp-lib drops them.
    let responder_client = DirectClientWithPump::new(&responder_server, || {
        transfer(&req_packets, &mut responder_server.borrow_mut());
        req_packets.borrow_mut().clear();
    });
    let responder_transport = MctpPldmTransport::new(responder_client);
    let responder = RefCell::new(PldmResponder::new(&CAPS));
    let responder_buf = RefCell::new([0u8; 1024]);

    // Count how many times the pump actually runs and how many responder
    // iterations successfully consume a request.
    let pump_calls = Cell::new(0usize);
    let responder_runs = Cell::new(0usize);

    // The requester's client drives one full responder cycle (which consumes
    // the request packets queued above) and then delivers the response packets
    // back to the requester server before the requester polls for its response.
    let requester_client = DirectClientWithPump::new(&requester_server, || {
        pump_calls.set(pump_calls.get() + 1);

        responder
            .borrow_mut()
            .run_once(
                &responder_transport,
                &mut responder_buf.borrow_mut()[..],
                TIMEOUT_MILLIS,
            )
            .expect("responder should process one PLDM request");
        responder_runs.set(responder_runs.get() + 1);

        transfer(&rsp_packets, &mut requester_server.borrow_mut());
        rsp_packets.borrow_mut().clear();
    });

    let requester_transport = MctpPldmTransport::new(requester_client);
    let mut requester = PldmRequester::new(&CAPS);
    let mut requester_buf = [0u8; 1024];

    // Queue one explicit requester command so this test drives a single
    // deterministic request/response exchange.
    requester.queue_get_tid();

    // A single run_once performs one complete request/response round trip over
    // the in-memory link.
    requester
        .run_once(
            &requester_transport,
            RESPONDER_EID,
            &mut requester_buf,
            TIMEOUT_MILLIS,
        )
        .expect("requester run_once should complete a full exchange");

    assert!(pump_calls.get() > 0, "requester should perform at least one recv");
    assert!(
        responder_runs.get() > 0,
        "at least one PLDM request/response should be pumped in-memory"
    );
}
