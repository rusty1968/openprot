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

/// How long the FD waits for a command after the update before it exits.
const IDLE_TIMEOUT_MILLIS: u32 = 2_000;

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
