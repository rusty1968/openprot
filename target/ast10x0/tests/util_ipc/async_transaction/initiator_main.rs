// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Initiator side of the util/ipc AsyncTransaction QEMU test.
//!
//! Runs three cases against `handler` and calls `debug_shutdown(Ok(()))` on
//! full pass or `debug_shutdown(Err(_))` on the first failure. The kernel
//! target writes `TEST_RESULT:PASS/FAIL` to UART.
//!
//! | Case               | Exercises                          | Expect            |
//! |--------------------|-------------------------------------|-------------------|
//! | blocking transact  | `IpcInitiator::transact`            | byte incremented  |
//! | async cancel       | `AsyncTransaction::start`/`cancel`  | channel freed     |
//! | async roundtrip    | `AsyncTransaction::start`/`try_recv`| byte incremented  |

#![no_main]
#![no_std]

use app_initiator::handle;
use pw_status::{Error, Result};
use userspace::syscall::{self, Signals};
use userspace::time::Instant;
use userspace::entry;
use util_ipc::{AsyncTransaction, IpcHandle, IpcInitiator};

static SEND_BUF: [u8; 1] = [0x10];
static mut RECV_BUF: [u8; 1] = [0u8; 1];

/// # Safety
/// Only called from this single-threaded app, and only while no
/// `AsyncTransaction` still holds a prior borrow of `RECV_BUF`.
unsafe fn recv_buf() -> &'static mut [u8] {
    // Safety: see function doc.
    unsafe { &mut *core::ptr::addr_of_mut!(RECV_BUF) }
}

fn test_blocking_transact() -> Result<()> {
    let ipc = IpcHandle::new(handle::IPC);
    let send = [0x20u8];
    let mut recv = [0u8; 1];

    let len = ipc.transact(&send, &mut recv, Instant::MAX)?;
    if len != 1 || recv[0] != 0x21 {
        pw_log::error!("blocking transact: unexpected response");
        return Err(Error::Internal);
    }
    Ok(())
}

fn test_async_cancel() -> Result<()> {
    let mut txn = AsyncTransaction::new(IpcHandle::new(handle::IPC));
    // Safety: no other AsyncTransaction is live right now.
    txn.start(&SEND_BUF, unsafe { recv_buf() })?;

    txn.cancel()?;
    if txn.is_pending() {
        pw_log::error!("async cancel: still pending after cancel()");
        return Err(Error::Internal);
    }

    // Verify the channel is free again.
    // Safety: the previous transaction was cancelled above.
    txn.start(&SEND_BUF, unsafe { recv_buf() })?;
    txn.cancel()?;
    Ok(())
}

fn test_async_roundtrip() -> Result<()> {
    let mut txn = AsyncTransaction::new(IpcHandle::new(handle::IPC));
    // Safety: no other AsyncTransaction is live right now.
    txn.start(&SEND_BUF, unsafe { recv_buf() })?;

    syscall::object_wait(txn.as_raw(), Signals::READABLE, Instant::MAX)?;

    let completion = txn.try_recv()?;
    if completion.len != 1 || completion.buffers.recv[0] != 0x11 {
        pw_log::error!("async roundtrip: unexpected response");
        return Err(Error::Internal);
    }
    Ok(())
}

#[entry]
fn entry() {
    let ret = test_blocking_transact()
        .and_then(|_| test_async_cancel())
        .and_then(|_| test_async_roundtrip());

    match &ret {
        Ok(()) => pw_log::info!("All test cases PASSED"),
        Err(e) => pw_log::error!("FAILED: status code {}", *e as u32),
    }

    let _ = syscall::debug_shutdown(ret);
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = userspace::syscall::debug_shutdown(Err(pw_status::Error::Internal));
    loop {}
}
