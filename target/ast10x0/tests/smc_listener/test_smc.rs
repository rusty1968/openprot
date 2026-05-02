// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

use pw_status::{Error, Result};
use test_smc_codegen::handle;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

const PAYLOAD_VERSION: u8 = 1;
const EXPECT_DMA_COMPLETE_BIT: u8 = 11;
const EXPECT_DMA_ERROR_BIT: u8 = 10;
const EXPECT_WRITE_PROTECT_BIT: u8 = 9;

fn run() -> Result<()> {
    let wait_return = syscall::object_wait(handle::IPC, Signals::READABLE, Instant::MAX)?;
    if !wait_return.pending_signals.contains(Signals::READABLE) || wait_return.user_data != 0 {
        return Err(Error::Internal);
    }

    let mut rx = [0u8; 5];
    let len = syscall::channel_read(handle::IPC, 0, &mut rx)?;
    if len != 5 {
        return Err(Error::OutOfRange);
    }

    if rx[0] != PAYLOAD_VERSION
        || rx[1] != EXPECT_DMA_COMPLETE_BIT
        || rx[2] != EXPECT_DMA_ERROR_BIT
        || rx[3] != EXPECT_WRITE_PROTECT_BIT
    {
        return Err(Error::FailedPrecondition);
    }

    let ack = [0u8; 0];
    syscall::channel_respond(handle::IPC, &ack)?;

    Ok(())
}

#[entry]
fn entry() -> ! {
    let ret = run();
    let _ = syscall::debug_shutdown(ret);
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
