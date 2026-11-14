// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Client side of the hello-async test.
//!
//! One thread, no blocking `transact`. Both channels are started, then one
//! WaitGroup wait services whichever lands first.
//!
//! | Case            | Expect                                                    |
//! |-----------------|-----------------------------------------------------------|
//! | overlap         | `fast` completes while `slow` is still pending (`poll` is  |
//! |                 | `Ok(None)`), then `slow` completes after its delay         |
//! | cancel deferred | cancelling a parked `slow` frees the channel; the server   |
//! |                 | survives its failed `complete` and serves `slow` again     |

#![no_main]
#![no_std]

use app_client::handle;
use pw_status::{Error, Result};
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::{Clock, Duration, Instant, SystemClock};
use util_ipc::{AsyncChannelTransport, IpcHandle};
use util_transport::AsyncTransport;

static mut SLOW_SEND: [u8; 8] = [0; 8];
static mut SLOW_RECV: [u8; 8] = [0; 8];
static mut FAST_SEND: [u8; 8] = [0; 8];
static mut FAST_RECV: [u8; 8] = [0; 8];

type Transport = AsyncChannelTransport<IpcHandle>;

/// # Safety
/// Called once per case, and only while no transport from an earlier case
/// is alive (each case ends with both idle and dropped).
unsafe fn transports() -> (Transport, Transport) {
    // Safety: see function doc.
    unsafe {
        (
            AsyncChannelTransport::new(
                IpcHandle::new(handle::SLOW),
                &mut *core::ptr::addr_of_mut!(SLOW_SEND),
                &mut *core::ptr::addr_of_mut!(SLOW_RECV),
            ),
            AsyncChannelTransport::new(
                IpcHandle::new(handle::FAST),
                &mut *core::ptr::addr_of_mut!(FAST_SEND),
                &mut *core::ptr::addr_of_mut!(FAST_RECV),
            ),
        )
    }
}

/// Log `$msg` and bail out of the case. A macro because the tokenized log
/// backend needs the message as a literal.
macro_rules! fail {
    ($msg:literal) => {{
        pw_log::error!($msg);
        return Err(Error::Internal);
    }};
}

/// Start both channels, then service them from one WaitGroup. `fast` must
/// land first, with `slow` still pending, and `slow` must take about its
/// delay.
fn test_overlap() -> Result<()> {
    // Safety: no other transports are alive.
    let (mut slow, mut fast) = unsafe { transports() };
    let mut resp = [0u8; 8];

    let t0 = SystemClock::now();
    slow.start(b"hi").map_err(|_| Error::Internal)?;
    fast.start(b"hi").map_err(|_| Error::Internal)?;

    syscall::wait_group_add(handle::WG, slow.as_raw(), Signals::READABLE, 0)?;
    syscall::wait_group_add(handle::WG, fast.as_raw(), Signals::READABLE, 1)?;

    // First wake must be `fast`.
    let first = syscall::object_wait(handle::WG, Signals::READABLE, Instant::MAX)?;
    if first.user_data != 1 {
        fail!("overlap: slow landed before fast");
    }
    match fast.poll(&mut resp) {
        Ok(Some(n)) if resp.get(..n) == Some(b"fast") => {}
        _ => fail!("overlap: bad fast response"),
    }
    syscall::wait_group_remove(handle::WG, fast.as_raw())?;

    // The deferral must not have completed `slow` yet, and the client was
    // free to ask.
    if !matches!(slow.poll(&mut resp), Ok(None)) {
        fail!("overlap: slow not pending while parked");
    }

    // Now wait for `slow` through the same WaitGroup.
    let second = syscall::object_wait(handle::WG, Signals::READABLE, Instant::MAX)?;
    if second.user_data != 0 {
        fail!("overlap: unexpected second wake");
    }
    match slow.poll(&mut resp) {
        Ok(Some(n)) if resp.get(..n) == Some(b"slow") => {}
        _ => fail!("overlap: bad slow response"),
    }
    syscall::wait_group_remove(handle::WG, slow.as_raw())?;

    if SystemClock::now() - t0 < Duration::from_millis(40) {
        fail!("overlap: slow answered before its delay");
    }
    Ok(())
}

/// Cancel while the server holds the reply. The server's `complete` then
/// fails; it must carry on and serve `slow` again.
fn test_cancel_deferred() -> Result<()> {
    // Safety: the previous case's transports are gone.
    let (mut slow, mut fast) = unsafe { transports() };
    let mut resp = [0u8; 8];

    slow.start(b"hi").map_err(|_| Error::Internal)?;
    // Give the server time to read and park it.
    let settle = SystemClock::now()
        .checked_add_duration(Duration::from_millis(10))
        .unwrap_or(Instant::MAX);
    let _ = syscall::object_wait(slow.as_raw(), Signals::READABLE, settle);
    slow.cancel().map_err(|_| Error::Internal)?;

    // Outlive the server's deadline so its failed `complete` has happened.
    let past = SystemClock::now()
        .checked_add_duration(Duration::from_millis(80))
        .unwrap_or(Instant::MAX);
    let _ = syscall::object_wait(fast.as_raw(), Signals::READABLE, past);

    // Both channels still work.
    fast.start(b"hi").map_err(|_| Error::Internal)?;
    syscall::object_wait(fast.as_raw(), Signals::READABLE, Instant::MAX)?;
    if !matches!(fast.poll(&mut resp), Ok(Some(4))) {
        fail!("cancel: fast broken");
    }
    slow.start(b"hi").map_err(|_| Error::Internal)?;
    syscall::object_wait(slow.as_raw(), Signals::READABLE, Instant::MAX)?;
    match slow.poll(&mut resp) {
        Ok(Some(n)) if resp.get(..n) == Some(b"slow") => Ok(()),
        _ => fail!("cancel: slow not served again"),
    }
}

const ROUNDS: u32 = 20;

fn run_all() -> Result<()> {
    for round in 0..ROUNDS {
        let ret = test_overlap().and_then(|_| test_cancel_deferred());
        if ret.is_err() {
            pw_log::error!("failed in round {}", round as u32);
            return ret;
        }
    }
    Ok(())
}

#[entry]
fn entry() {
    let ret = run_all();
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
