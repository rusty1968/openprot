// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Eager host driver for the MCTP-over-I3C throughput benchmark.
//!
//! Streams fixed-size MCTP messages to the sink as fast as the socket round
//! trips allow — no sleeps — reading each one-byte ack before sending the next
//! (strict request/response pacing; at most one message in flight). The firmware
//! times the transfer against the emulated `mtime` clock and logs
//! `THROUGHPUT_KBPS:`.
//!
//! Message size is tunable at runtime via the `TPUT_MSG_BYTES` env var (default
//! 240), so a sweep across the single-/multi-fragment boundary needs no rebuild:
//!
//! ```bash
//! for n in 120 240 480 720; do
//!   bazel test //target/veer/tests/i3c_throughput:i3c_throughput_test \
//!     --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
//!     --test_env=TPUT_MSG_BYTES=$n 2>&1 | grep -E 'THROUGHPUT:'
//! done
//! ```
//!
//! Messages larger than the ~240 B single-fragment MTU are fragmented into
//! multiple MCTP packets (SOM/EOM/seq), which the inbound ring queues and the
//! MCTP stack reassembles — exactly the "fewer, bigger messages => fewer IPC
//! crossings per byte" lever. Keep the message within the ring depth
//! (4 fragments => ~960 B) so no fragment is dropped.
//!
//! Addresses/EIDs mirror tests/mctp_i3c: target 0x08, controller 0x0a, target
//! EID 8, host EID 9, MCTP message type 1.

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

/// Max MCTP packet payload per i3c fragment. The single-fragment MTU is
/// 250 - 4 (i3c hdr) - 4 (mctp hdr) - 1 (pec) = 241; 240 leaves a byte of slack.
const FRAG_PAYLOAD: usize = 240;

/// Default data bytes per MCTP message when `TPUT_MSG_BYTES` is unset.
const DEFAULT_MSG_BYTES: usize = 240;

fn msg_bytes() -> usize {
    std::env::var("TPUT_MSG_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MSG_BYTES)
        .clamp(1, 959)
}

/// Frame one i3c private write: `[dest<<1, cmd, byte_count, src<<1|1] + mctp + pec`.
fn frame_mctp_packet(mctp_packet: &[u8]) -> Vec<u8> {
    let byte_count = (1 + mctp_packet.len()) as u8;
    let mut frame = vec![
        TARGET_ADDR << 1,
        MCTP_I3C_CMD,
        byte_count,
        (CONTROLLER_ADDR << 1) | 1,
    ];
    frame.extend_from_slice(mctp_packet);
    frame.push(crc8_smbus(&frame));
    frame
}

/// Build the i3c frames for one MCTP message carrying `data`, fragmenting into
/// `FRAG_PAYLOAD`-sized MCTP packets with SOM/EOM/sequence flags (TO=1, tag 0).
fn build_message_frames(data: &[u8]) -> Vec<Vec<u8>> {
    // MCTP message = [msg_type] + data; fragment the whole body.
    let mut body = Vec::with_capacity(1 + data.len());
    body.push(MSG_TYPE);
    body.extend_from_slice(data);

    let chunks: Vec<&[u8]> = body.chunks(FRAG_PAYLOAD).collect();
    let n = chunks.len();
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let som = if i == 0 { 0x80 } else { 0 };
            let eom = if i == n - 1 { 0x40 } else { 0 };
            let seq = ((i as u8) & 0x03) << 4;
            let flags = som | eom | seq | 0x08; // TO=1, tag 0
            let mut pkt = vec![0x01, TARGET_EID, HOST_EID, flags];
            pkt.extend_from_slice(chunk);
            frame_mctp_packet(&pkt)
        })
        .collect()
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
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set read timeout");

    let n = msg_bytes();
    let data = vec![0xa5u8; n];
    let frames = build_message_frames(&data);
    eprintln!(
        "driving {}-byte messages ({} i3c fragment(s) each)",
        n,
        frames.len()
    );

    let mut rounds = 0u32;
    // Strict one-message-in-flight pacing: send all fragments of a message, then
    // poll private reads until the firmware's (non-empty) ack comes back, and
    // only then send the next. A read with nothing staged returns a ZERO-length
    // frame as Ok, so the ack must be detected by a non-empty payload. The budget
    // is bounded so a lost ack can't wedge the run; on exhaustion the outer loop
    // resends the whole message.
    while !runner.exited() {
        for frame in &frames {
            let _ = send_private_write_raw_on_stream(&mut stream, TARGET_ADDR, frame);
        }
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
    eprintln!("drove {rounds} message rounds; see THROUGHPUT in the firmware log above");
}
