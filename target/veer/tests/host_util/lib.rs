// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Helpers shared by the VeeR host-side tests that drive firmware through
//! `caliptra_mcu_hw_model`, the way caliptra-mcu-sw's own tests do: boot the
//! model with an I3C socket, step it, read the MCU log, and move MCTP packets to
//! and from the target over the socket.
//!
//! The model is single-threaded. It only advances while it is being stepped, so
//! every wait here steps it, and a pause between two writes is a stretch of
//! stepping, not a sleep.
//!
//! The same code runs on the emulator (x86_64) and on the VCK190, where the
//! model forwards the socket to the FPGA's I3C host controller. [`on_fpga`] says
//! which, for a test that must differ.
//!
//! # MCTP over I3C on the wire
//!
//! DSP0233: one MCTP packet is the data of one private transfer, with nothing in
//! front of it. `BufferedStream::send_private_write` appends the PEC, and
//! `receive_private_read` checks and strips it, so a packet here is the bare MCTP
//! packet. [`send_message`] fragments a message into packets of at most
//! [`MAX_PACKET`] bytes and [`read_message`] reassembles the target's.

use caliptra_mcu_hw_model::McuHwModel;
use caliptra_mcu_testing_common::i3c_socket::BufferedStream;
use std::time::{Duration, Instant};

/// How long a wait may take, in wall-clock seconds, before the test fails.
pub const TIMEOUT_SECS: u64 = 120;

/// The MCTP transport header is 4 bytes; the message body follows it.
pub const MCTP_HEADER_LEN: usize = 4;

/// Largest packet the host writes, header included. The target's receive limit is
/// 256 bytes including the PEC.
pub const MAX_PACKET: usize = 251;

/// Time to let the target drain one write before the next fragment of a message.
/// The TTI receive FIFO holds 256 bytes, so back-to-back full packets would
/// overrun it (caliptra-mcu-sw's own packetized write leaves the same gap).
pub const WRITE_GAP: Duration = Duration::from_millis(50);

/// Whether this test runs against the FPGA (`CALIPTRA_INTERFACE=fpga`, which
/// `caliptra_test` sets) and not the emulator.
pub fn on_fpga() -> bool {
    std::env::var("CALIPTRA_INTERFACE").as_deref() == Ok("fpga")
}

/// Append whatever the firmware has logged to `log` and print it.
pub fn drain_log(hw: &mut impl McuHwModel, log: &mut String) {
    let out = hw.output().take(usize::MAX);
    if !out.is_empty() {
        print!("{out}");
        log.push_str(&out);
    }
}

/// Step the model until any of `terms` appears in the MCU log, accumulated in
/// `log`, and return the index of the first term found. Looks at everything
/// logged so far, so a term that arrived in the same burst as an earlier one is
/// still found; when several are present, the one earliest in `terms` wins.
/// Panics on a fatal error or timeout.
pub fn wait_for_any(hw: &mut impl McuHwModel, log: &mut String, terms: &[&str]) -> usize {
    let found = |log: &str| terms.iter().position(|t| log.contains(t));
    let start = Instant::now();
    hw.step_until(|m| {
        let out = m.output().take(usize::MAX);
        if !out.is_empty() {
            print!("{out}");
            log.push_str(&out);
        }
        found(log).is_some()
            || m.mci_fw_fatal_error().is_some()
            || start.elapsed().as_secs() > TIMEOUT_SECS
    });
    assert_eq!(hw.mci_fw_fatal_error(), None, "firmware hit a fatal error");
    found(log).unwrap_or_else(|| panic!("timed out waiting for any of {terms:?} in the MCU log"))
}

/// Like [`wait_for_any`] for a single term.
pub fn wait_for_log(hw: &mut impl McuHwModel, log: &mut String, term: &str) {
    wait_for_any(hw, log, &[term]);
}

/// Step the model for about `gap` of wall-clock time.
pub fn step_for(hw: &mut impl McuHwModel, log: &mut String, gap: Duration) {
    let start = Instant::now();
    hw.step_until(|m| {
        let out = m.output().take(usize::MAX);
        if !out.is_empty() {
            print!("{out}");
            log.push_str(&out);
        }
        start.elapsed() >= gap || m.mci_fw_fatal_error().is_some()
    });
    assert_eq!(hw.mci_fw_fatal_error(), None, "firmware hit a fatal error");
}

/// Read one packet from the target: wait for its IBI, then for the read the IBI
/// triggers. Returns the packet with its PEC already checked and stripped.
pub fn read_packet(
    hw: &mut impl McuHwModel,
    stream: &mut BufferedStream,
    log: &mut String,
    target_addr: u8,
) -> Vec<u8> {
    let mut packet: Option<Vec<u8>> = None;
    let mut got_ibi = false;
    let start = Instant::now();
    hw.step_until(|m| {
        let out = m.output().take(usize::MAX);
        if !out.is_empty() {
            print!("{out}");
            log.push_str(&out);
        }
        if !got_ibi {
            got_ibi = stream.receive_ibi(target_addr);
        } else if let Some(data) = stream.receive_private_read(target_addr) {
            packet = Some(data);
        }
        packet.is_some()
            || m.mci_fw_fatal_error().is_some()
            || start.elapsed().as_secs() > TIMEOUT_SECS
    });
    assert_eq!(hw.mci_fw_fatal_error(), None, "firmware hit a fatal error");
    packet.unwrap_or_else(|| panic!("no packet read from the target (IBI seen: {got_ibi})"))
}

/// An MCTP message: who it is from and to, its tag, and its body (the message
/// type byte, then the payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub dest: u8,
    pub src: u8,
    /// The tag-owner bit: set on a request, clear on a response.
    pub tag_owner: bool,
    pub tag: u8,
    pub body: Vec<u8>,
    /// How many packets carried it.
    pub packets: usize,
}

/// The packets of a message: the body cut into packets of at most [`MAX_PACKET`]
/// bytes, each with a transport header (version 1, destination, source, flags).
/// Flags: SOM on the first packet, EOM on the last, a 2-bit packet sequence
/// number counting from 0, the tag-owner bit, and the tag.
pub fn message_packets(dest: u8, src: u8, tag_owner: bool, tag: u8, body: &[u8]) -> Vec<Vec<u8>> {
    message_packets_sized(dest, src, tag_owner, tag, body, MAX_PACKET)
}

/// Like [`message_packets`] with packets of at most `max_packet` bytes, header
/// included, instead of [`MAX_PACKET`].
pub fn message_packets_sized(
    dest: u8,
    src: u8,
    tag_owner: bool,
    tag: u8,
    body: &[u8],
    max_packet: usize,
) -> Vec<Vec<u8>> {
    assert!(!body.is_empty(), "an MCTP message has at least a type byte");
    assert!(
        max_packet > MCTP_HEADER_LEN && max_packet <= MAX_PACKET,
        "packet size {max_packet} outside {MCTP_HEADER_LEN}..={MAX_PACKET}"
    );
    let chunks: Vec<&[u8]> = body.chunks(max_packet - MCTP_HEADER_LEN).collect();
    let last = chunks.len() - 1;
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut flags = (tag & 0x07) | (((i & 0x03) as u8) << 4);
            if tag_owner {
                flags |= 0x08;
            }
            if i == 0 {
                flags |= 0x80;
            }
            if i == last {
                flags |= 0x40;
            }
            let mut packet = vec![0x01, dest, src, flags];
            packet.extend_from_slice(chunk);
            packet
        })
        .collect()
}

/// Send a message to the target as private writes, one per packet. Between two
/// packets of one message the model is stepped for [`WRITE_GAP`] so the target
/// drains its receive FIFO.
pub fn send_message(
    hw: &mut impl McuHwModel,
    stream: &mut BufferedStream,
    log: &mut String,
    target_addr: u8,
    message: &Message,
) {
    send_message_paced(hw, stream, log, target_addr, message, WRITE_GAP);
}

/// Like [`send_message`] with the pause between the packets of one message
/// chosen by the caller. A test that times the transfer picks the smallest pause
/// the target keeps up with: the pause is part of what it measures.
pub fn send_message_paced(
    hw: &mut impl McuHwModel,
    stream: &mut BufferedStream,
    log: &mut String,
    target_addr: u8,
    message: &Message,
    gap: Duration,
) {
    send_message_sized(hw, stream, log, target_addr, message, gap, MAX_PACKET);
}

/// Like [`send_message_paced`] with packets of at most `max_packet` bytes.
///
/// A smaller packet means more of them per message and a lighter load on the
/// target's 256 byte receive FIFO, which a full 251 byte packet nearly fills.
pub fn send_message_sized(
    hw: &mut impl McuHwModel,
    stream: &mut BufferedStream,
    log: &mut String,
    target_addr: u8,
    message: &Message,
    gap: Duration,
    max_packet: usize,
) {
    let packets = message_packets_sized(
        message.dest,
        message.src,
        message.tag_owner,
        message.tag,
        &message.body,
        max_packet,
    );
    let last = packets.len() - 1;
    for (i, packet) in packets.into_iter().enumerate() {
        assert!(
            stream.send_private_write(target_addr, packet),
            "I3C socket closed before the write"
        );
        if i != last {
            step_for(hw, log, gap);
        }
    }
}

/// Read one whole message from the target: packets are read until one carries
/// EOM, checked as they go (version, that every packet is from the same
/// source, to the same destination, with the same tag, within [`MAX_PACKET`],
/// SOM on the first only, and a sequence that follows from the first packet's,
/// which may start anywhere, modulo 4) and reassembled.
pub fn read_message(
    hw: &mut impl McuHwModel,
    stream: &mut BufferedStream,
    log: &mut String,
    target_addr: u8,
) -> Message {
    let mut message: Option<Message> = None;
    let mut first_seq = 0u8;
    loop {
        let packet = read_packet(hw, stream, log, target_addr);
        // [ver][dest][src][flags][body...]
        assert!(
            packet.len() > MCTP_HEADER_LEN,
            "packet too short: {packet:02x?}"
        );
        assert!(
            packet.len() <= MAX_PACKET,
            "packet of {} bytes is over the MTU",
            packet.len()
        );
        assert_eq!(packet[0] & 0x0f, 0x01, "MCTP header version");
        let flags = packet[3];
        let (tag_owner, tag) = (flags & 0x08 != 0, flags & 0x07);
        let seq = (flags >> 4) & 0x03;
        let count = message.as_ref().map_or(0, |m| m.packets);
        if let Some(m) = &message {
            assert_eq!(packet[1], m.dest, "destination EID changed mid-message");
            assert_eq!(packet[2], m.src, "source EID changed mid-message");
            assert_eq!(
                (tag_owner, tag),
                (m.tag_owner, m.tag),
                "tag changed mid-message"
            );
        } else {
            first_seq = seq;
        }
        assert_eq!(seq, (first_seq + count as u8) & 0x03, "packet sequence");
        assert_eq!(
            flags & 0x80 != 0,
            count == 0,
            "SOM on the first packet only"
        );

        let m = message.get_or_insert_with(|| Message {
            dest: packet[1],
            src: packet[2],
            tag_owner,
            tag,
            body: Vec::new(),
            packets: 0,
        });
        m.body.extend_from_slice(&packet[MCTP_HEADER_LEN..]);
        m.packets += 1;
        if flags & 0x40 != 0 {
            return message.take().expect("a packet was read");
        }
    }
}
