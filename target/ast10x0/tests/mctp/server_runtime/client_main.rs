// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Client exerciser for the server-runtime QEMU test.
//!
//! Drives the real `mctp_server_runtime::run()` loop (in `server_test`)
//! over three channels: `mctp_a`/`mctp_b` (normal MCTP clients) and
//! `transport` (a loopback stand-in for the I2C IRQ). Calls
//! `debug_shutdown(Ok(()))` on full pass or `debug_shutdown(Err(_))` on the
//! first failure.
//!
//! ## Test cases
//!
//! | TC   | Exercises                                                          |
//! |------|---------------------------------------------------------------------|
//! | TC-1 | set_eid/get_eid/listener/req/drop_handle — non-deferred Reply path |
//! | TC-2 | deferred Recv, resolved by a loopback transport poke               |
//! | TC-3 | deferred Recv, timed out with no transport activity                |
//!
//! TC-2 uses `util_ipc::AsyncTransaction` instead of the blocking
//! `IpcMctpClient::recv` so the Recv genuinely reaches `Pending` on the
//! server (removed from its WaitGroup) before anything else happens — no
//! second "sender" app or fixed delay is needed, since everything runs on
//! this one thread in strict sequence.

#![no_main]
#![no_std]

use openprot_mctp_api::wire::{self, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE};
use openprot_mctp_api::{MctpClient, ResponseCode};
use openprot_mctp_client_ipc::IpcMctpClient;
use pw_status::Error;
use userspace::syscall::Signals;
use userspace::time::{Clock, Duration, Instant, SystemClock};
use userspace::{entry, syscall};
use util_ipc::{AsyncTransaction, IpcHandle};

use app_client_test::handle;

const OWN_EID: u8 = 8;

static mut TC2_SEND_BUF: [u8; MAX_REQUEST_SIZE] = [0u8; MAX_REQUEST_SIZE];
static mut TC2_RECV_BUF: [u8; MAX_RESPONSE_SIZE] = [0u8; MAX_RESPONSE_SIZE];

/// # Safety
/// Single-threaded test app: only touched once, to encode the TC-2 request
/// before `AsyncTransaction::start`, and not read/written again until that
/// transaction's `try_recv` returns.
unsafe fn tc2_send_buf() -> &'static mut [u8] {
    unsafe { &mut *core::ptr::addr_of_mut!(TC2_SEND_BUF) }
}

/// # Safety
/// Same single-use contract as `tc2_send_buf`, for the response buffer.
unsafe fn tc2_recv_buf() -> &'static mut [u8] {
    unsafe { &mut *core::ptr::addr_of_mut!(TC2_RECV_BUF) }
}

#[entry]
fn entry() {
    match run() {
        Ok(()) => {
            pw_log::info!("All test cases PASSED");
            let _ = syscall::debug_shutdown(Ok(()));
        }
        Err(()) => {
            let _ = syscall::debug_shutdown(Err(Error::Internal));
        }
    }
    loop {}
}

fn run() -> Result<(), ()> {
    let client_a = IpcMctpClient::new(handle::MCTP_A);
    let client_b = IpcMctpClient::new(handle::MCTP_B);

    // ── TC-1: non-deferred ops through the real dispatch loop ───────────────
    pw_log::info!("TC-1: set_eid/get_eid/listener/req/drop_handle");
    client_a.set_eid(OWN_EID).map_err(|e| {
        pw_log::error!("TC-1 FAIL: set_eid code={}", e.code as u32);
    })?;
    let eid = client_a.get_eid();
    if eid != OWN_EID {
        pw_log::error!(
            "TC-1 FAIL: get_eid expected {}, got {}",
            OWN_EID as u32,
            eid as u32
        );
        return Err(());
    }
    let probe_listener = client_a.listener(9).map_err(|e| {
        pw_log::error!("TC-1 FAIL: listener code={}", e.code as u32);
    })?;
    let probe_req = client_a.req(OWN_EID).map_err(|e| {
        pw_log::error!("TC-1 FAIL: req code={}", e.code as u32);
    })?;
    client_a.drop_handle(probe_req);
    client_a.drop_handle(probe_listener);

    // ── TC-2: deferred recv, resolved via a loopback transport poke ─────────
    pw_log::info!("TC-2: deferred recv + transport poke");
    let listener = client_a.listener(5).map_err(|e| {
        pw_log::error!("TC-2 FAIL: listener code={}", e.code as u32);
    })?;

    // Safety: no other AsyncTransaction is live on channel A right now.
    let req_len = wire::encode_recv(unsafe { tc2_send_buf() }, listener.0, 0).map_err(|_| {
        pw_log::error!("TC-2 FAIL: encode_recv");
    })?;
    // Safety: TC2_SEND_BUF was just written above and won't be touched again
    // until the transaction below completes.
    let send_buf: &'static [u8] = unsafe { &*core::ptr::addr_of!(TC2_SEND_BUF) };

    let mut txn = AsyncTransaction::new(IpcHandle::new(handle::MCTP_A));
    // Non-blocking: the server genuinely enters `DispatchOutcome::Pending`
    // here, removing channel A from its WaitGroup, before we do anything else.
    if txn
        .start(&send_buf[..req_len], unsafe { tc2_recv_buf() })
        .is_err()
    {
        pw_log::error!("TC-2 FAIL: async start");
        return Err(());
    }

    let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
    client_b
        .send(None, 5, Some(OWN_EID), None, false, &payload)
        .map_err(|e| {
            pw_log::error!("TC-2 FAIL: send code={}", e.code as u32);
        })?;

    // Sticky signal: wakes `run()`'s transport wait whenever it next polls,
    // so there's no race even though the server may not be waiting yet.
    if IpcHandle::new(handle::TRANSPORT)
        .set_peer_user_signal(true)
        .is_err()
    {
        pw_log::error!("TC-2 FAIL: set_peer_user_signal");
        return Err(());
    }

    let deadline = SystemClock::now()
        .checked_add_duration(Duration::from_millis(2000))
        .unwrap_or(Instant::MAX);
    if syscall::object_wait(txn.as_raw(), Signals::READABLE, deadline).is_err() {
        pw_log::error!("TC-2 FAIL: recv never became readable");
        return Err(());
    }

    let completion = txn.try_recv().map_err(|e| {
        pw_log::error!("TC-2 FAIL: try_recv status={}", e as u32);
    })?;
    let resp = &completion.buffers.recv[..completion.len];

    let header = wire::decode_response_header(resp).map_err(|_| {
        pw_log::error!("TC-2 FAIL: decode_response_header");
    })?;
    if !header.is_success() {
        pw_log::error!(
            "TC-2 FAIL: response code={}",
            header.response_code() as u32
        );
        return Err(());
    }
    let resp_payload = wire::get_response_payload(resp, &header).map_err(|_| {
        pw_log::error!("TC-2 FAIL: get_response_payload");
    })?;
    if resp_payload != &payload[..] {
        pw_log::error!("TC-2 FAIL: payload mismatch");
        return Err(());
    }
    if header.eid != OWN_EID {
        pw_log::error!(
            "TC-2 FAIL: remote_eid expected {}, got {}",
            OWN_EID as u32,
            header.eid as u32
        );
        return Err(());
    }

    // ── TC-3: recv timeout with no transport activity ────────────────────────
    pw_log::info!("TC-3: recv timeout, no data ever arrives");
    let idle_listener = client_a.listener(6).map_err(|e| {
        pw_log::error!("TC-3 FAIL: listener code={}", e.code as u32);
    })?;
    let mut buf = [0u8; 16];
    match client_a.recv(idle_listener, 200, &mut buf) {
        Err(e) if e.code == ResponseCode::TimedOut => {}
        Err(e) => {
            pw_log::error!("TC-3 FAIL: expected TimedOut, got {}", e.code as u32);
            return Err(());
        }
        Ok(_) => {
            pw_log::error!("TC-3 FAIL: expected Err, got Ok");
            return Err(());
        }
    }

    Ok(())
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
