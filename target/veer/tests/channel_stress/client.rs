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

#![no_main]
#![no_std]

use chanstress_client_codegen::handle;
use userspace::process_entry;
use userspace::syscall;
use userspace::time::Instant;

/// Request payload size, matching the ~180-byte firmware-data chunks the PLDM
/// download transfers.
const MSG_LEN: usize = 180;
/// How many round-trips to drive. Far more than the few hundred transactions the
/// PLDM download crashed within, so a load-accumulating bug is caught early.
const NUM_TRANSACTIONS: u32 = 50_000;
/// Log cadence, so the last line before a crash shows how far it got.
const LOG_EVERY: u32 = 5_000;

#[process_entry("client")]
fn entry() {
    pw_log::info!("channel stress client: starting");

    let req = [0xa5u8; MSG_LEN];
    let mut resp = [0u8; 256];

    for i in 0..NUM_TRANSACTIONS {
        if i % LOG_EVERY == 0 {
            pw_log::info!("channel stress client: tick {}", i);
        }
        if syscall::channel_transact(handle::CHANNEL, &req[..], &mut resp[..], Instant::MAX).is_err()
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
