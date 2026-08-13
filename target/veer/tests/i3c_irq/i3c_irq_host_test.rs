// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host-side harness for the VeeR interrupt-driven I3C RX test.
//!
//! Identical to the smoke harness: inject private-write frames until the
//! firmware (which discovers the frame via the I3C interrupt rather than
//! polling) receives one and exits.

use i3c_host::{connect_i3c_socket, send_private_write_on_stream, Runner};
use std::thread;
use std::time::Duration;

const PAYLOAD: [u8; 4] = [0x01, 0x02, 0x03, 0x04];

#[test]
fn i3c_irq_host_test() {
    let runner = Runner::spawn(
        "target/veer/tests/i3c_irq/i3c_irq_runner.sh",
        "waiting for private write",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before firmware readiness"
    );

    let addr = runner.target_addr();
    let mut stream =
        connect_i3c_socket(Duration::from_secs(5)).expect("failed to connect to I3C socket");

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
