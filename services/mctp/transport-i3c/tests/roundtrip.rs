// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! MCTP over I3C, end to end on the host.
//!
//! `Server::send()` → `I3cSender` → the real `i3c_server::dispatch` → a mock
//! `I3cTarget` that records every staged frame, then back out through
//! `MctpI3cReceiver`. Every layer but the hardware is the production one.

use std::cell::RefCell;

use i3c_api::MAX_PAYLOAD as I3C_MAX_PAYLOAD;
use i3c_client::I3cClient;
use i3c_server::{LoopbackTransport, Server as I3cServer};
use mctp::Eid;
use openprot_hal_blocking::i3c_hardware::{DynamicAddress, I3cTarget, TargetEvent};
use openprot_mctp_server::Server;
use openprot_mctp_transport_i3c::{transfer_pec, I3cSender, MctpI3cReceiver, MCTP_I3C_IPC_MAXMTU};

const OWN_ADDR: u8 = 0x08;
const REMOTE_ADDR: u8 = 0x1d;
const OWN_EID: u8 = 8;
const REMOTE_EID: u8 = 48;
const MSG_TYPE: u8 = 0x05; // SPDM

/// A target that records every frame staged for transmission.
struct CaptureTarget<'a> {
    addr: Option<u8>,
    sends: &'a RefCell<Vec<Vec<u8>>>,
}

#[derive(Debug, PartialEq, Eq)]
struct CaptureError;

impl I3cTarget for CaptureTarget<'_> {
    type Error = CaptureError;

    fn enable(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn on_interrupt(&mut self) -> Result<TargetEvent, Self::Error> {
        Ok(TargetEvent::None)
    }

    fn read_frame(&mut self, _buf: &mut [u8]) -> Result<Option<usize>, Self::Error> {
        Ok(None)
    }

    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.sends.borrow_mut().push(data.to_vec());
        Ok(())
    }

    fn dynamic_address(&self) -> Option<DynamicAddress> {
        self.addr.and_then(|a| DynamicAddress::try_from(a).ok())
    }
}

type Sender<'a> = I3cSender<LoopbackTransport<CaptureTarget<'a>>>;

fn server(sends: &RefCell<Vec<Vec<u8>>>, addr: Option<u8>, pec: bool) -> Server<Sender<'_>, 16> {
    let target = CaptureTarget { addr, sends };
    let client = I3cClient::new(LoopbackTransport::new(I3cServer::new(1, 2, target)));
    Server::new(Eid(OWN_EID), 0, I3cSender::new(client, REMOTE_ADDR, pec))
}

/// Decode a frame the way the I3C *controller* sees it: the target's response to
/// a private read. DSP0233 frames it as the MCTP packet followed by a PEC over
/// the read address byte `(addr << 1) | 1` and the packet. Nothing precedes the
/// packet.
fn controller_read_decode(addr: u8, frame: &[u8], pec: bool) -> &[u8] {
    if !pec {
        return frame;
    }
    let (received, packet) = frame.split_last().expect("frame carries a PEC");
    assert_eq!(
        *received,
        transfer_pec((addr << 1) | 1, packet),
        "PEC of a controller read"
    );
    packet
}

#[test]
fn sender_receiver_roundtrip() {
    let sends = RefCell::new(Vec::new());
    let mut srv = server(&sends, Some(OWN_ADDR), true);

    let payload = b"hello mctp";
    let req = srv.req(REMOTE_EID).expect("req");
    srv.send(Some(req), MSG_TYPE, None, None, false, payload)
        .expect("send");

    let frames = sends.borrow();
    assert_eq!(frames.len(), 1, "short payload should be one frame");

    // The frame is exactly the MCTP packet plus a PEC.
    let pkt = controller_read_decode(OWN_ADDR, &frames[0], true);
    assert_eq!(frames[0].len(), pkt.len() + 1);

    // MCTP transport header (4) then the message-type byte, then the payload.
    assert!(pkt.len() >= 5, "packet too short: {}", pkt.len());
    assert_eq!(pkt[4] & 0x7f, MSG_TYPE, "message type");
    assert_eq!(&pkt[5..], payload, "payload");
}

#[test]
fn roundtrip_without_pec() {
    let sends = RefCell::new(Vec::new());
    let mut srv = server(&sends, Some(OWN_ADDR), false);

    let payload = b"no pec";
    let req = srv.req(REMOTE_EID).expect("req");
    srv.send(Some(req), MSG_TYPE, None, None, false, payload)
        .expect("send");

    let frames = sends.borrow();
    let pkt = controller_read_decode(OWN_ADDR, &frames[0], false);
    assert_eq!(&pkt[5..], payload);

    // Without a PEC the frame is the bare packet.
    assert_eq!(frames[0].len(), pkt.len());
}

#[test]
fn single_fragment_produces_one_send() {
    let sends = RefCell::new(Vec::new());
    let mut srv = server(&sends, Some(OWN_ADDR), true);

    let req = srv.req(REMOTE_EID).expect("req");
    srv.send(Some(req), 1, None, None, false, b"short")
        .expect("send");

    assert_eq!(sends.borrow().len(), 1);
}

/// The case the MTU cap exists for: a payload past one fragment must split,
/// and *every* frame must still fit one i3c IPC frame.
#[test]
fn large_payload_fragments_and_every_frame_fits() {
    let sends = RefCell::new(Vec::new());
    let mut srv = server(&sends, Some(OWN_ADDR), true);

    let payload = vec![0x5au8; MCTP_I3C_IPC_MAXMTU * 3];
    let req = srv.req(REMOTE_EID).expect("req");
    srv.send(Some(req), MSG_TYPE, None, None, false, &payload)
        .expect("send");

    let frames = sends.borrow();
    assert!(
        frames.len() > 1,
        "payload of {} should have fragmented, got {} frame(s)",
        payload.len(),
        frames.len()
    );

    for (i, frame) in frames.iter().enumerate() {
        assert!(
            frame.len() <= I3C_MAX_PAYLOAD,
            "frame {} is {} bytes, over the {}-byte i3c limit",
            i,
            frame.len(),
            I3C_MAX_PAYLOAD
        );
        // Each frame must carry a valid PEC on its own.
        controller_read_decode(OWN_ADDR, frame, true);
    }
}

/// A payload sized exactly to the MTU is the boundary the cap is derived from.
#[test]
fn mtu_sized_fragment_fills_the_frame_exactly() {
    let sends = RefCell::new(Vec::new());
    let mut srv = server(&sends, Some(OWN_ADDR), true);

    // One byte of the first fragment is the message-type byte, so an MTU-sized
    // first packet carries MTU-1 bytes of payload.
    let payload = vec![0xa5u8; MCTP_I3C_IPC_MAXMTU - 1];
    let req = srv.req(REMOTE_EID).expect("req");
    srv.send(Some(req), MSG_TYPE, None, None, false, &payload)
        .expect("send");

    let frames = sends.borrow();
    assert_eq!(frames.len(), 1, "should still be a single fragment");
    assert_eq!(
        frames[0].len(),
        I3C_MAX_PAYLOAD,
        "an MTU-sized fragment should exactly fill the i3c frame"
    );
}

/// A send before the controller has assigned a dynamic address must fail
/// without putting anything on the bus.
///
/// This asserts the observable contract through `Server::send`. The precise
/// `mctp::Error::Unreachable` that `I3cSender` returns (documented on its
/// `own_addr`) is not checked here: the MCTP server maps it, along with several
/// other errors, through a wildcard to `ResponseCode::InternalError`, so the
/// specific variant cannot be recovered at this layer.
#[test]
fn send_before_entdaa_fails_without_touching_the_bus() {
    let sends = RefCell::new(Vec::new());
    let mut srv = server(&sends, None, true);

    let req = srv.req(REMOTE_EID).expect("req");
    assert!(
        srv.send(Some(req), MSG_TYPE, None, None, false, b"x")
            .is_err(),
        "send without a dynamic address must fail"
    );
    assert!(
        sends.borrow().is_empty(),
        "nothing should reach the bus before an address is assigned"
    );
}

/// A frame the target stages for a controller *read* must not be accepted as an
/// inbound controller *write*: the two PECs cover different address bytes
/// (`(addr << 1) | 1` versus `addr << 1`).
#[test]
fn receiver_rejects_an_outbound_frame() {
    let sends = RefCell::new(Vec::new());
    let mut srv = server(&sends, Some(OWN_ADDR), true);

    let req = srv.req(REMOTE_EID).expect("req");
    srv.send(Some(req), MSG_TYPE, None, None, false, b"loopback")
        .expect("send");

    let frame = sends.borrow()[0].clone();
    let mut rx = MctpI3cReceiver::new(true);
    assert!(rx.decode(&frame, || Some(OWN_ADDR)).is_err());
}

#[test]
fn receiver_rejects_a_corrupt_pec() {
    let sends = RefCell::new(Vec::new());
    let mut srv = server(&sends, Some(OWN_ADDR), true);

    let req = srv.req(REMOTE_EID).expect("req");
    srv.send(Some(req), MSG_TYPE, None, None, false, b"tamper")
        .expect("send");

    // Re-frame the packet the way a controller writes it, then corrupt the PEC.
    let frame = sends.borrow()[0].clone();
    let packet = controller_read_decode(OWN_ADDR, &frame, true).to_vec();
    let mut write = packet.clone();
    write.push(transfer_pec(OWN_ADDR << 1, &packet));

    let mut rx = MctpI3cReceiver::new(true);
    assert_eq!(
        rx.decode(&write, || Some(OWN_ADDR)).expect("good write"),
        &packet[..]
    );

    let last = write.len() - 1;
    write[last] ^= 0xff;
    assert!(rx.decode(&write, || Some(OWN_ADDR)).is_err());
}
