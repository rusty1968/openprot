// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host-side harness for the VeeR I3C IBI test.
//!
//! Sends a private write to trigger the firmware's IBI, asserts the IBI's
//! MDB arrives on the socket, then sends a "DONE" write so the firmware
//! exits 0.
//!
//! The emulator's IBI model forwards only the MDB to the controller side
//! (payload forwarding is an upstream TODO in emulator/periph/src/i3c.rs
//! check_ibi_buffer), so only the MDB is asserted here even though the
//! firmware raises the IBI with a payload.

use i3c_host::{
    connect_i3c_socket, read_outgoing_packet, send_private_write_on_stream, Runner,
};
use std::thread;
use std::time::Duration;

const TRIGGER: [u8; 4] = [0xb0, 0xb1, 0xb2, 0xb3];
const IBI_MDB: u8 = 0xA5;
const DONE: &[u8] = b"DONE";

#[test]
fn i3c_ibi_host_test() {
    let runner = Runner::spawn(
        "target/veer/tests/i3c_ibi/i3c_ibi_runner.sh",
        "waiting for private write",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before firmware readiness"
    );

    let addr = runner.target_addr();
    let mut stream =
        connect_i3c_socket(Duration::from_secs(5)).expect("failed to connect to I3C socket");

    let mut ibi_seen = false;
    let mut attempts = 0u32;
    'outer: for _ in 0..20 {
        attempts += 1;
        if send_private_write_on_stream(&mut stream, addr, &TRIGGER).is_err() {
            thread::sleep(Duration::from_millis(250));
            continue;
        }
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("failed to set read timeout");
        // The IBI is forwarded spontaneously by the emulator's controller
        // pump; no read command is needed. Drain packets until it shows up
        // or the read times out.
        loop {
            match read_outgoing_packet(&mut stream) {
                Ok(pkt) => {
                    println!(
                        "I3C HOST TRACE: packet ibi=0x{:02x} from=0x{:02x} data={:02x?}",
                        pkt.ibi, pkt.from_addr, pkt.data
                    );
                    if pkt.ibi == IBI_MDB {
                        ibi_seen = true;
                        break 'outer;
                    }
                }
                Err(_) => break,
            }
        }
    }
    assert!(ibi_seen, "IBI never received after {} attempts", attempts);

    // Host-driven teardown: keep sending DONE until the firmware exits.
    while !runner.exited() {
        let _ = send_private_write_on_stream(&mut stream, addr, DONE);
        thread::sleep(Duration::from_millis(100));
    }

    let status = runner.wait();
    assert!(status.success(), "runner exited with status: {}", status);
}
