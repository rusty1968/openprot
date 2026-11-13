// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! PLDM firmware device process for the PLDM-over-I3C correctness test: the
//! shared FD (`//target/veer/tests/pldm_fd`) over two `IpcMctpClient`s to the
//! `mctp_server`, one per direction as `FirmwareDevice` wants.
//!
//! Small chunks: each `RequestFirmwareData` response is one MCTP fragment, so
//! this test covers the plain PLDM flow. The throughput images raise the cap.

#![no_main]
#![no_std]

use openprot_mctp_client_ipc::IpcMctpClient;
use pldm_fd_codegen::handle;
use userspace::process_entry;
use veer_pldm_fd::FdConfig;

/// Chunk cap: with the MCTP and PLDM headers, a chunk of this size stays within
/// the single-fragment MTU.
const XFER_CAP: usize = 180;

/// The host test serves an image of this size, so three chunks.
const IMAGE_SIZE: usize = 512;

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
