// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Bringup client app for PFR mailbox-over-I2C validation.
//!
//! This app wires the userspace I2C IPC client to the in-process SWMBX
//! controller bridge (`I2cPfrSmbusClient`) and runs the mailbox target flow
//! used by PFR bringup.
//!
//! Startup sequence:
//! 1. Bind to the I2C server channel (`handle::I2C`).
//! 2. Construct SWMBX over the platform mailbox MMIO region
//!    (`SWMBX_BUF_BASE`, `SWMBX_NODE_COUNT`).
//! 3. Configure SWMBX behavior bits (PROTECT/NOTIFY/FIFO), FIFO mappings, and
//!    per-port protection bitmaps.
//! 4. Enter I2C target mode at `SLAVE_ADDR` and enable notifications.
//! 5. Emit `MAILBOX_READY` to provide an explicit external readiness marker.
//!
//! Runtime behavior:
//! - Receives slave notifications from the I2C server as `Signals::USER`.
//! - Blocks on `Signals::USER` from the server channel.
//! - Drains one pending slave event per wake via `process_one_event()`.
//! - Drops unknown-source events with rate-limited warnings.
//! - Fails fast on setup/runtime errors that indicate bringup misconfiguration.

#![no_main]
#![no_std]

use app_pfr_i2c_notify_client::handle;
use ast10x0_pfr::{
    I2cPfrClientError, I2cPfrSmbusClient, SourceAddressMap, SwmbxCtrl,
    SWMBX_BUF_BASE, SWMBX_NODE_COUNT,
    SWMBX_FIFO, SWMBX_FIFO_NOTIFY_STOP, SWMBX_NOTIFY, SWMBX_PROTECT,
};
use i2c_client::I2cClient;
use i2c_client_ipc::IpcTransport;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

const SLAVE_ADDR: u8 = 0x38;
const BMC_SOURCE_ADDR: u8 = 0x20;
const PCH_SOURCE_ADDR: u8 = 0x22;

const UFM_WRITE_FIFO: u8 = 0x0d;
const UFM_READ_FIFO: u8 = 0x0e;
const SWMBX_WRITE_FIFO_SIZE: usize = 64;
const SWMBX_READ_FIFO_SIZE: usize = 128;
const BMC_UPDATE_INTENT: u8 = 0x13;

macro_rules! fail {
    ($msg:literal) => {{
        pw_log::error!($msg);
        loop {}
    }};
}

#[entry]
fn entry() {
    let i2c = I2cClient::new(IpcTransport::new(handle::I2C));
    // SAFETY: SWMBX_BUF_BASE is the platform-defined SWMBX mailbox region.
    let swmbx = unsafe { SwmbxCtrl::new_with_regions(SWMBX_NODE_COUNT, SWMBX_BUF_BASE) };
    let sources = SourceAddressMap {
        bmc: BMC_SOURCE_ADDR,
        pch_cpu: PCH_SOURCE_ADDR,
    };
    let mut mailbox = I2cPfrSmbusClient::new(i2c, swmbx, sources);

    // Enable SWMBX policy engines used by the mailbox flow.
    // PROTECT enforces access masks, NOTIFY latches mailbox events, FIFO enables
    // FIFO-backed register semantics for selected addresses.
    if mailbox
        .controller_mut()
        .enable_behavior(SWMBX_PROTECT | SWMBX_NOTIFY | SWMBX_FIFO, true)
        .is_err()
    {
        fail!("enable_behavior failed");
    }
    // Configure write FIFO (0x0d): host writes are queued and notify on STOP.
    if mailbox
        .controller_mut()
        .update_fifo(
            0,
            UFM_WRITE_FIFO,
            SWMBX_WRITE_FIFO_SIZE,
            SWMBX_FIFO_NOTIFY_STOP,
            true,
        )
        .is_err()
    {
        fail!("update_fifo write failed");
    }
    // Configure read FIFO (0x0e): host reads drain queued data and notify on STOP.
    if mailbox
        .controller_mut()
        .update_fifo(
            1,
            UFM_READ_FIFO,
            SWMBX_READ_FIFO_SIZE,
            SWMBX_FIFO_NOTIFY_STOP,
            true,
        )
        .is_err()
    {
        fail!("update_fifo read failed");
    }
    // Enable notification on update-intent mailbox register for BMC port.
    if mailbox
        .controller_mut()
        .update_notify(0, BMC_UPDATE_INTENT, true)
        .is_err()
    {
        fail!("update_notify failed");
    }

    let bmc_access_control: [u32; 8] = [
        0xfff704ff,
        0xffffffff,
        0xffffffff,
        0xfffffff2,
        0xffffffff,
        0xffffffff,
        0x00000000,
        0x00000000,
    ];
    let pch_access_control: [u32; 8] = [
        0xfff884ff,
        0xffffffff,
        0xffffffff,
        0xfffffff5,
        0x00000000,
        0x00000000,
        0xffffffff,
        0xffffffff,
    ];
    // Apply per-port protection bitmaps across the mailbox address space.
    // A set bit marks an address protected for that port.
    if mailbox
        .controller_mut()
        .apply_protect(0, &bmc_access_control, 0)
        .is_err()
    {
        fail!("apply_protect BMC failed");
    }
    if mailbox
        .controller_mut()
        .apply_protect(1, &pch_access_control, 0)
        .is_err()
    {
        fail!("apply_protect PCH failed");
    }

    if mailbox.start(SLAVE_ADDR).is_err() {
        fail!("mailbox start failed");
    }

    // Explicit readiness marker for external orchestration.
    pw_log::info!("MAILBOX_READY addr=0x{:02x}", SLAVE_ADDR as u32);

    pw_log::info!(
        "PFR bringup listening addr=0x{:02x}; waiting for target notifications",
        SLAVE_ADDR as u32,
    );

    let mut unknown_source_events: u32 = 0;
    loop {
        if syscall::object_wait(handle::I2C, Signals::USER, Instant::MAX).is_err() {
            fail!("object_wait USER failed");
        }

        match mailbox.process_one_event() {
            Ok(()) => {}
            Err(I2cPfrClientError::UnknownSource) => {
                unknown_source_events = unknown_source_events.wrapping_add(1);
                if unknown_source_events == 1 || (unknown_source_events % 32) == 0 {
                    pw_log::warn!(
                        "dropping event with unknown source (count={})",
                        unknown_source_events as u32
                    );
                }
            }
            Err(_) => fail!("process_one_event failed"),
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
