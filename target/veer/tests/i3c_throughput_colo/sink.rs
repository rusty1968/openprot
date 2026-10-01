// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! MCTP-over-I3C inbound throughput benchmark app.
//!
//! Receives a fixed total of bytes from the host as a sequence of MCTP messages,
//! acking each one (request/response flow control so the single-frame-at-a-time
//! controller never overruns the i3c inbound ring), and reports the achieved
//! throughput in emulated time.
//!
//! Timing uses the RV timer `mtime` MMIO at `0x21000000 + 0xe4` — the same
//! counter `target/veer/syscall_latency` reads. On the emulator it advances at
//! 1 MHz, so one tick is one microsecond of emulated time. This is the clock the
//! Caliptra emulator exposes; it has no built-in throughput instrumentation, so
//! the measurement is derived here from bytes transferred over elapsed ticks.
//!
//! What the number means: it is end-to-end request/response throughput — the
//! firmware sits in `recv` between messages while the host completes its
//! private-read/write round-trip, and that latency is part of the emulated
//! elapsed time. It is therefore a *relative* instrument: hold the host round
//! trip constant and vary one transport parameter (message size / chunk fill,
//! inbound ring depth, multi-fragment reassembly) to see the effect. Do not read
//! the absolute B/s as the raw transport ceiling.

#![no_main]
#![no_std]

use mctp_throughput_sink_codegen::handle;
use openprot_mctp_api::{MctpListener, MctpRespChannel, Stack};
use openprot_mctp_client_ipc::IpcMctpClient;
use openprot_mctp_echo::prepare_listener;
use userspace::process_entry;
use userspace::syscall;

/// Total payload bytes to receive (excluding the first, uncounted message)
/// before reporting. 8 KiB gives a stable number without a long run.
const TARGET_BYTES: usize = 8 * 1024;

/// `mtime` counter frequency on the emulator (1 MHz => 1 tick = 1 microsecond).
const MTIME_HZ: u64 = 1_000_000;

/// Read the 64-bit RV timer `mtime` (emulated microseconds on the emulator).
#[inline(always)]
fn mtime() -> u64 {
    // SAFETY: 0x21000000 is the RV timer device region mapped into this process
    // by system.json5; +0xe4 is the 64-bit mtime register. The read is volatile
    // and side-effect-free.
    let p = core::ptr::with_exposed_provenance::<u64>(0x21000000 + 0xe4);
    unsafe { p.read_volatile() }
}

fn run() {
    let stack = Stack::new(IpcMctpClient::new(handle::MCTP));
    let mut listener = match prepare_listener(&stack) {
        Ok(l) => l,
        Err(e) => {
            pw_log::error!("throughput: listener setup failed code={}", e.code as u32);
            let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
            return;
        }
    };

    pw_log::info!("mctp throughput: waiting for data");

    let ack = [0u8; 1];
    // Sized for a full multi-fragment MCTP message (stack MAX_PAYLOAD is 1023),
    // so the host can send messages that span several i3c fragments.
    let mut buf = [0u8; 1024];
    let mut total = 0usize;
    let mut msgs = 0u32;
    let mut t0 = 0u64;
    let mut first = true;

    while total < TARGET_BYTES {
        match listener.recv(&mut buf) {
            Ok((_meta, msg, mut resp)) => {
                let n = msg.len();
                // Ack to pace the host; a 1-byte response keeps the outbound
                // cost negligible next to the inbound data being measured.
                let _ = resp.send(&ack);
                if first {
                    // Start the clock at the first message and do not count it,
                    // so the measured interval holds only steady-state transfer.
                    t0 = mtime();
                    first = false;
                } else {
                    total = total.saturating_add(n);
                    msgs = msgs.saturating_add(1);
                }
            }
            Err(e) => {
                if !e.is_timeout() {
                    pw_log::error!("throughput: recv failed code={}", e.code as u32);
                    let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
                    return;
                }
            }
        }
    }

    let elapsed = mtime().saturating_sub(t0).max(1);
    // bytes/s = total * MTIME_HZ / elapsed_ticks.
    let bytes_per_s = (total as u64).saturating_mul(MTIME_HZ) / elapsed;
    let kbps = bytes_per_s / 1024;
    let per_msg = if msgs > 0 { total / msgs as usize } else { 0 };
    pw_log::info!(
        "THROUGHPUT: {} bytes in {} msgs ({} B/msg) in {} ticks ({} B/s)",
        total as u32,
        msgs as u32,
        per_msg as u32,
        elapsed as u32,
        bytes_per_s as u32,
    );
    pw_log::info!("THROUGHPUT_KBPS: {}", kbps as u32);

    let _ = syscall::debug_shutdown(Ok(()));
}

#[process_entry("throughput_sink")]
fn entry() {
    run();
    #[expect(clippy::empty_loop)]
    loop {}
}
