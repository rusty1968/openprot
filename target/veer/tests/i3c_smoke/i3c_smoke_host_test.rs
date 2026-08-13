// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host-side harness for the VeeR I3C smoke test.
//!
//! Launches the emulator runner, waits for the firmware to report it is
//! waiting for a private write, then injects private-write frames over the
//! I3C TCP socket until the firmware receives one and exits.

use i3c_host::{connect_i3c_socket, send_private_write_on_stream, Runner};
use std::thread;
use std::time::Duration;

const PAYLOAD: [u8; 4] = [0x01, 0x02, 0x03, 0x04];

#[test]
fn i3c_smoke_host_test() {
    let runner = Runner::spawn(
        "target/veer/tests/i3c_smoke/i3c_smoke_runner.sh",
        // Only the firmware's own log line counts as readiness: the
        // emulator's I3C socket opens minutes earlier, and frames sent that
        // early race firmware boot.
        "waiting for private write",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before firmware readiness"
    );

    let addr = runner.target_addr();
    let mut stream =
        connect_i3c_socket(Duration::from_secs(5)).expect("failed to connect to I3C socket");

    // Keep sending until the runner exits: the firmware terminates the
    // emulator as soon as one frame arrives.
    let mut sends = 0u32;
    while !runner.exited() {
        if send_private_write_on_stream(&mut stream, addr, &PAYLOAD).is_ok() {
            sends += 1;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let status = runner.wait();
    assert!(sends > 0, "no I3C private-write frame was ever sent");
    assert!(status.success(), "runner exited with status: {}", status);
}
