// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host-side harness for the VeeR I3C echo test.
//!
//! Sends a private write, reads the firmware's echo back with a private
//! read, then sends a "DONE" write so the firmware exits 0.

use i3c_host::{
    body_with_pec, connect_i3c_socket, read_outgoing_packet, send_private_read_on_stream,
    send_private_write_on_stream, Runner,
};
use std::thread;
use std::time::Duration;

const PAYLOAD: [u8; 16] = [
    0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae,
    0xaf,
];
const DONE: &[u8] = b"DONE";

#[test]
fn i3c_echo_host_test() {
    let runner = Runner::spawn(
        "target/veer/tests/i3c_echo/i3c_echo_runner.sh",
        "waiting for private write",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before firmware readiness"
    );

    let addr = runner.target_addr();
    let mut stream =
        connect_i3c_socket(Duration::from_secs(5)).expect("failed to connect to I3C socket");
    // The firmware echoes the full body (payload + PEC) back verbatim.
    let expected = body_with_pec(addr, &PAYLOAD);

    let mut echoed = false;
    let mut attempts = 0u32;
    'outer: for _ in 0..20 {
        attempts += 1;
        if send_private_write_on_stream(&mut stream, addr, &PAYLOAD).is_err() {
            thread::sleep(Duration::from_millis(250));
            continue;
        }
        thread::sleep(Duration::from_millis(100));
        if send_private_read_on_stream(&mut stream, addr).is_err() {
            thread::sleep(Duration::from_millis(250));
            continue;
        }
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("failed to set read timeout");
        // Drain packets until the echo shows up or the read times out;
        // skip IBIs and stale responses from earlier attempts.
        loop {
            match read_outgoing_packet(&mut stream) {
                Ok(pkt) => {
                    println!(
                        "I3C HOST TRACE: packet ibi=0x{:02x} from=0x{:02x} data={:02x?}",
                        pkt.ibi, pkt.from_addr, pkt.data
                    );
                    if pkt.ibi == 0 && pkt.data == expected {
                        echoed = true;
                        break 'outer;
                    }
                }
                Err(_) => break,
            }
        }
    }
    assert!(echoed, "echo never received after {} attempts", attempts);

    // Host-driven teardown: keep sending DONE until the firmware exits.
    while !runner.exited() {
        let _ = send_private_write_on_stream(&mut stream, addr, DONE);
        thread::sleep(Duration::from_millis(100));
    }

    let status = runner.wait();
    assert!(status.success(), "runner exited with status: {}", status);
}
