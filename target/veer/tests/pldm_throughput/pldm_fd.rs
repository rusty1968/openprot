// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! PLDM firmware device process for the PLDM throughput benchmark: the shared FD
//! (`//target/veer/tests/pldm_fd`) over two `IpcMctpClient`s to the
//! `mctp_server`.
//!
//! Large chunks. Each `RequestFirmwareData` response spans about five inbound MCTP
//! fragments, which the i3c server's inbound ring queues (`RX_RING`, 6) and the
//! FD's MCTP stack reassembles into a message of about 965 bytes, within the MCTP
//! stack's 1032 byte payload limit: fewer, bigger download round trips, which is
//! what the throughput depends on most. It needs `pldm-common`'s
//! `MAX_TRANSFER_SIZE` raised to 960 (a crate patch in `MODULE.bazel`).

#![no_main]
#![no_std]

use openprot_mctp_client_ipc::IpcMctpClient;
use pldm_fd_codegen::handle;
use userspace::process_entry;
use veer_pldm_fd::FdConfig;

/// Per-chunk cap, well past one fragment.
const XFER_CAP: usize = 960;

/// Assumed until `UpdateComponent` gives the real size; the host sets it.
const IMAGE_SIZE: usize = 4096;

/// How long the FD waits for a command before it gives up, and so how long a run
/// takes to finish after the update (the host UA stops talking). Built-in 2 s;
/// the build can set `PLDM_FD_IDLE_MS`: the native update agent's first send
/// waits up to 50M emulator ticks (tens of seconds on the emulator) for a
/// `FIRMWARE_PLDM_READY` boot milestone this firmware never raises, so its image
/// needs a timeout that outlasts that.
const IDLE_TIMEOUT_MILLIS: u32 = match option_env!("PLDM_FD_IDLE_MS") {
    Some(ms) => parse_millis(ms),
    None => 2_000,
};

const fn parse_millis(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut value = 0u32;
    let mut i = 0;
    while i < bytes.len() {
        assert!(bytes[i].is_ascii_digit(), "PLDM_FD_IDLE_MS is a number");
        value = value * 10 + (bytes[i] - b'0') as u32;
        i += 1;
    }
    value
}

#[process_entry("pldm_fd")]
fn entry() {
    veer_pldm_fd::run(
        FdConfig {
            xfer_cap: XFER_CAP,
            default_image_size: IMAGE_SIZE,
            idle_timeout_millis: IDLE_TIMEOUT_MILLIS,
        },
        IpcMctpClient::new(handle::MCTP),
        IpcMctpClient::new(handle::MCTP),
    )
}
