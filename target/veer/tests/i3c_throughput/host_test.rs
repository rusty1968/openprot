// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Eager host driver for the MCTP-over-I3C throughput benchmark.
//!
//! Unlike the other host harnesses, this one deliberately does NOT sleep between
//! operations: it streams fixed-size MCTP messages to the sink as fast as the
//! socket round-trips allow, reading each one-byte ack before sending the next
//! (strict request/response, so at most one inbound frame is ever in flight and
//! the i3c ring cannot overrun). The firmware times the transfer against the
//! emulated `mtime` clock and logs `THROUGHPUT_KBPS:` — read that line from this
//! test's captured firmware log; the assertion here only confirms the run
//! completed cleanly.
//!
//! Addresses/EIDs mirror tests/mctp_i3c: target dynamic address 0x08, controller
//! 0x0a, target EID 8, host EID 9, MCTP message type 1.

use i3c_host::{
    connect_i3c_socket, crc8_smbus, read_outgoing_packet, send_private_read_on_stream,
    send_private_write_raw_on_stream, Runner,
};
use std::time::Duration;

const TARGET_ADDR: u8 = 0x08;
const CONTROLLER_ADDR: u8 = 0x0a;
const TARGET_EID: u8 = 8;
const HOST_EID: u8 = 9;
const MSG_TYPE: u8 = 1;
const MCTP_I3C_CMD: u8 = 0x0f;

/// MCTP message payload size per frame. Kept under the single-fragment MTU
/// (4 i3c + 4 mctp + 1 type + payload + 1 pec <= 250 => payload <= 240) so each
/// message is one i3c frame; raise once inbound multi-fragment reassembly lands
/// to measure larger-chunk throughput.
const PAYLOAD_LEN: usize = 200;

/// Build one MCTP-over-I3C private-write frame (TO=1 request, tag 0), mirroring
/// `MctpI3cEncap::encode`.
fn build_request(payload: &[u8]) -> Vec<u8> {
    let mut mctp_packet = vec![0x01, TARGET_EID, HOST_EID, 0xC8, MSG_TYPE];
    mctp_packet.extend_from_slice(payload);

    let byte_count = (1 + mctp_packet.len()) as u8;
    let mut frame = vec![
        TARGET_ADDR << 1,
        MCTP_I3C_CMD,
        byte_count,
        (CONTROLLER_ADDR << 1) | 1,
    ];
    frame.extend_from_slice(&mctp_packet);
    frame.push(crc8_smbus(&frame));
    frame
}

#[test]
fn i3c_throughput_host_test() {
    let runner = Runner::spawn(
        "target/veer/tests/i3c_throughput/i3c_throughput_runner.sh",
        "mctp throughput: waiting for data",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before firmware readiness"
    );

    let mut stream =
        connect_i3c_socket(Duration::from_secs(5)).expect("failed to connect to I3C socket");
    // A short read timeout turns "ack not staged yet" into a fast error so the
    // ack poll spins instead of blocking; no sleeps anywhere else (eager).
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set read timeout");

    let payload = [0xa5u8; PAYLOAD_LEN];
    let request = build_request(&payload);

    let mut rounds = 0u32;
    // Strict one-in-flight pacing: send one message, then poll private reads
    // until the firmware's (non-empty) ack frame comes back, and only then send
    // the next. A private read with nothing staged returns a ZERO-length frame
    // as Ok, so the ack must be detected by a non-empty payload — treating the
    // empty read as an ack is what let the first cut flood the socket and the
    // emulator drop the excess writes. The attempt budget is bounded so a
    // genuinely lost ack can't wedge the run; on exhaustion the outer loop
    // resends the same message (at most one extra in flight).
    while !runner.exited() {
        let _ = send_private_write_raw_on_stream(&mut stream, TARGET_ADDR, &request);
        rounds += 1;
        for _ in 0..1000 {
            if runner.exited() {
                break;
            }
            if send_private_read_on_stream(&mut stream, TARGET_ADDR).is_ok()
                && let Ok(pkt) = read_outgoing_packet(&mut stream)
                && !pkt.data.is_empty()
            {
                break;
            }
        }
    }

    let status = runner.wait();
    assert!(
        status.success(),
        "runner exited with status {status} after {rounds} message rounds"
    );
    eprintln!("drove {rounds} message rounds; see THROUGHPUT_KBPS in the firmware log above");
}
