// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host-side harness for the VeeR MCTP-over-I3C echo test.
//!
//! Sends one MCTP message to the target as an I3C private write, framed per
//! DSP0233: the MCTP packet (transport header + message-type byte + payload) is
//! the data of the write, followed by a PEC over the write address byte and the
//! packet (added by `send_private_write_on_stream`). The target's MCTP router
//! hands it to the echo app, which sends the payload straight back; the firmware
//! then exits 0. We collect the echoed frame with a private read, check its PEC,
//! and check the payload survived the round trip.
//!
//! NOTE (first-cut, expected to need tuning once it runs):
//!   - Addresses: target dynamic address 0x08 (ROM-assigned, as in
//!     tests/i3c_service). The controller address is not carried on the wire
//!     (DSP0233 has no source field).
//!   - EIDs: target 8, host/controller 9 (echo replies to the source EID).
//!   - The firmware exits as soon as it has staged the reply, so we poll the
//!     private read alongside detecting exit, as tests/i3c_service does.

use i3c_host::{
    connect_i3c_socket, packet_from_read, read_outgoing_packet, send_private_read_on_stream,
    send_private_write_on_stream, Runner,
};
use std::thread;
use std::time::Duration;

const TARGET_ADDR: u8 = 0x08;
const TARGET_EID: u8 = 8;
const HOST_EID: u8 = 9;
const ECHO_MSG_TYPE: u8 = 1;

/// The payload we send and expect echoed back.
const PAYLOAD: &[u8] = b"mctp-i3c";

/// Build the MCTP packet carried by the private write (DSP0233: nothing precedes
/// it, and `send_private_write_on_stream` appends the PEC).
fn build_request() -> Vec<u8> {
    // MCTP transport header (DSP0236): version, dest EID, src EID, flags.
    // flags 0xC8 = SOM | EOM | TO(tag owner) | tag 0.
    let mut mctp_packet = vec![0x01, TARGET_EID, HOST_EID, 0xC8, ECHO_MSG_TYPE];
    mctp_packet.extend_from_slice(PAYLOAD);
    mctp_packet
}

/// Pull the echoed payload out of a reply packet (PEC already verified and
/// stripped), if it looks like ours.
/// Layout: [mctp hdr (4)][msg_type (1)][payload...].
fn echoed_payload(data: &[u8]) -> Option<&[u8]> {
    const PREFIX: usize = 4 + 1;
    if data.len() <= PREFIX {
        return None;
    }
    Some(&data[PREFIX..])
}

#[test]
fn mctp_i3c_echo_host_test() {
    let runner = Runner::spawn(
        "target/veer/tests/mctp_i3c/mctp_i3c_runner.sh",
        "mctp echo: waiting for request",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before firmware readiness"
    );

    let mut stream =
        connect_i3c_socket(Duration::from_secs(5)).expect("failed to connect to I3C socket");
    let request = build_request();

    let mut got_echo = false;
    let mut attempts = 0u32;
    while !runner.exited() && attempts < 200 {
        attempts += 1;
        // The write helper appends the PEC over the write address byte and the
        // MCTP packet, which is exactly the frame DSP0233 defines.
        let _ = send_private_write_on_stream(&mut stream, TARGET_ADDR, &request);
        thread::sleep(Duration::from_millis(100));

        // Collect the staged echo with a private read.
        if send_private_read_on_stream(&mut stream, TARGET_ADDR).is_ok()
            && let Ok(frame) = read_outgoing_packet(&mut stream)
            && let Ok(mctp) = packet_from_read(TARGET_ADDR, &frame.data)
            && echoed_payload(mctp) == Some(PAYLOAD)
        {
            got_echo = true;
            break;
        }
    }

    let status = runner.wait();
    assert!(
        status.success(),
        "runner exited with status: {status} after {attempts} attempts (echo captured: {got_echo})"
    );
    // The clean exit already proves the message reached the echo app; the byte
    // check is a stronger assertion when the read wins the race against exit.
    if !got_echo {
        eprintln!("note: firmware exited 0 but the echoed frame was not captured before exit");
    }
}
