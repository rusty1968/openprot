// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Channel-stress client.
//!
//! Hammers `channel_transact` against the echo server in a tight loop, then
//! exits 0. This mirrors the sustained request/response channel traffic that the
//! PLDM-over-I3C download drives (the FD's requester issuing RequestFirmwareData
//! and reading the reply, over and over) — but with no I3C, MCTP, or PLDM in the
//! path. If the kernel's channel syscalls corrupt a process context under load,
//! this reproduces the jump-to-null crash with everything else stripped away.
//!
//! Reaching "completed" and exiting 0 means the raw channel path survived the
//! load; a mid-run kernel terminal exception (non-zero exit) reproduces the bug.
//!
//! Observed: on pigweed 578e9b00 (upstream HEAD, 2026-09-30) and f9b83b60 this
//! crashes *deterministically* between transaction 3300 and 3400 — `epc=0`,
//! `mcause=1` (jump to null) then a kernel terminal exception. Same crash the
//! PLDM-over-I3C download hits; still present on the latest pigweed.

#![no_main]
#![no_std]

use chanstress_client_codegen::handle;
use userspace::process_entry;
use userspace::syscall;
use userspace::time::Instant;

/// Request payload size, matching the ~180-byte firmware-data chunks the PLDM
/// download transfers.
const MSG_LEN: usize = 180;
/// How many round-trips to drive. The bug trips deterministically around
/// transaction 3300, so this cap only matters if the bug is fixed — 10k is ample
/// to prove the channel path then survives sustained load.
const NUM_TRANSACTIONS: u32 = 10_000;
/// Log cadence. Fine enough to bound the crash to a 100-transaction window for
/// the upstream report, without flooding the UART on a clean run.
const LOG_EVERY: u32 = 100;

#[process_entry("client")]
fn entry() {
    pw_log::info!("channel stress client: starting");

    let req = [0xa5u8; MSG_LEN];
    let mut resp = [0u8; 256];

    for i in 0..NUM_TRANSACTIONS {
        if i % LOG_EVERY == 0 {
            pw_log::info!("channel stress client: tick {}", i);
        }
        if syscall::channel_transact(handle::CHANNEL, &req[..], &mut resp[..], Instant::MAX)
            .is_err()
        {
            pw_log::error!("channel stress client: channel_transact failed at {}", i);
            let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
            #[expect(clippy::empty_loop)]
            loop {}
        }
    }

    pw_log::info!(
        "channel stress client: completed {} transactions, exiting",
        NUM_TRANSACTIONS
    );
    let _ = syscall::debug_shutdown(Ok(()));
    #[expect(clippy::empty_loop)]
    loop {}
}
