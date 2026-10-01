// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host harness for the VeeR channel-IPC stress isolation test.
//!
//! Self-contained: there is no I3C traffic, so this just launches the emulator
//! and waits for the firmware to finish. The client drives tens of thousands of
//! `channel_transact` round-trips against the echo server and exits 0 on
//! success; a kernel channel-path crash exits non-zero. A clean exit here means
//! the raw channel syscalls survive sustained load; a failure reproduces the
//! jump-to-null crash the PLDM-over-I3C download hits, with no I3C/PLDM involved.

use i3c_host::Runner;
use std::time::Duration;

#[test]
fn channel_stress_test() {
    let runner = Runner::spawn(
        "target/veer/tests/channel_stress/channel_stress_runner.sh",
        "channel stress client: starting",
    );
    assert!(
        runner.wait_ready(Duration::from_secs(600)),
        "runner exited or timed out before the client started"
    );

    let status = runner.wait();
    assert!(
        status.success(),
        "runner exited with status: {status} (channel-IPC stress crashed the kernel?)"
    );
}
