// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Async executor integration test for RISC-V 32 QEMU (virt machine).
//!
//! Runs on `qemu-system-riscv32 -machine virt` using semihosting for I/O
//! and test pass/fail reporting.
//!
//! Three async tasks are spawned:
//!   1. Immediate completion — verifies basic future mechanics.
//!   2. Multi-yield accumulator — yields N times, summing values.
//!   3. Cooperative multitasking — two tasks ping-pong via yield.

#![no_main]
#![no_std]

use portable_atomic::{AtomicU32, Ordering};

use embassy_executor::Spawner;
use openprot_executor::{start_async, yield_once};
use riscv_semihosting::debug::{self, EXIT_FAILURE, EXIT_SUCCESS};
use riscv_semihosting::hprintln;

// ---------------------------------------------------------------------------
// Task results stored in atomics for cross-task verification
// ---------------------------------------------------------------------------
static RESULT_IMMEDIATE: AtomicU32 = AtomicU32::new(0);
static RESULT_ACCUMULATOR: AtomicU32 = AtomicU32::new(0);
static RESULT_PING: AtomicU32 = AtomicU32::new(0);
static RESULT_PONG: AtomicU32 = AtomicU32::new(0);

static DONE_COUNT: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------
// Test tasks
// ---------------------------------------------------------------------------

/// Task 1: Immediate completion — stores 42, verifies basic future mechanics.
#[embassy_executor::task]
async fn task_immediate() {
    RESULT_IMMEDIATE.store(42, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

/// Task 2: Multi-yield accumulator — yields 5 times, summing 1..=5 (expects 15).
#[embassy_executor::task]
async fn task_accumulator() {
    let mut sum = 0u32;
    for i in 1..=5u32 {
        sum += i;
        yield_once().await;
    }
    RESULT_ACCUMULATOR.store(sum, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

/// Task 3a: Ping — increments counter and yields, cooperating with pong.
#[embassy_executor::task]
async fn task_ping() {
    for _ in 0..5 {
        RESULT_PING.fetch_add(1, Ordering::Release);
        yield_once().await;
    }
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

/// Task 3b: Pong — increments counter and yields, cooperating with ping.
#[embassy_executor::task]
async fn task_pong() {
    for _ in 0..5 {
        RESULT_PONG.fetch_add(1, Ordering::Release);
        yield_once().await;
    }
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

/// Main orchestrator task — spawns all test tasks, waits, then verifies.
#[embassy_executor::task]
async fn task_main(spawner: Spawner) {
    hprintln!("[executor-test] Starting async executor integration test");

    spawner.spawn(task_immediate()).unwrap();
    spawner.spawn(task_accumulator()).unwrap();
    spawner.spawn(task_ping()).unwrap();
    spawner.spawn(task_pong()).unwrap();

    // Wait for all 4 tasks to complete.
    while DONE_COUNT.load(Ordering::Acquire) < 4 {
        yield_once().await;
    }

    // Verify results.
    let mut pass = true;

    let r1 = RESULT_IMMEDIATE.load(Ordering::Acquire);
    hprintln!("[executor-test] Task immediate: {} (expected 42)", r1);
    if r1 != 42 {
        hprintln!("[executor-test] FAIL: task_immediate returned wrong value");
        pass = false;
    }

    let r2 = RESULT_ACCUMULATOR.load(Ordering::Acquire);
    hprintln!("[executor-test] Task accumulator: {} (expected 15)", r2);
    if r2 != 15 {
        hprintln!("[executor-test] FAIL: task_accumulator returned wrong value");
        pass = false;
    }

    let r3 = RESULT_PING.load(Ordering::Acquire);
    hprintln!("[executor-test] Task ping: {} (expected 5)", r3);
    if r3 != 5 {
        hprintln!("[executor-test] FAIL: task_ping returned wrong value");
        pass = false;
    }

    let r4 = RESULT_PONG.load(Ordering::Acquire);
    hprintln!("[executor-test] Task pong: {} (expected 5)", r4);
    if r4 != 5 {
        hprintln!("[executor-test] FAIL: task_pong returned wrong value");
        pass = false;
    }

    if pass {
        hprintln!("[executor-test] PASSED: all tasks completed correctly");
        debug::exit(EXIT_SUCCESS);
    } else {
        hprintln!("[executor-test] FAILED");
        debug::exit(EXIT_FAILURE);
    }

    loop {}
}

// ---------------------------------------------------------------------------
// Entry point — riscv-rt provides _start, calls main
// ---------------------------------------------------------------------------

#[riscv_rt::entry]
fn main() -> ! {
    hprintln!("[executor-test] Booting on RISC-V 32 QEMU virt");
    start_async(|spawner| {
        spawner.spawn(task_main(spawner)).unwrap();
    });
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    hprintln!("[executor-test] PANIC: {}", info);
    debug::exit(EXIT_FAILURE);
    loop {}
}

// ---------------------------------------------------------------------------
// Critical-section implementation for single-hart RISC-V.
//
// The `critical-section` crate expects two extern "C" symbols to be provided
// by *some* crate in the final binary.  Normally the `riscv` crate supplies
// them via its `critical-section-single-hart` feature, but Bazel's
// crate-universe resolves multiple `riscv` versions (0.11, 0.12, 0.13) and
// the one that actually gets linked may not carry the feature.  Providing the
// symbols here avoids the version-conflict entirely.
//
// acquire: clear MIE (bit 3) in mstatus, return previous MIE state.
// release: restore MIE if it was set before acquire.
// ---------------------------------------------------------------------------
mod critical_section_impl {
    use core::arch::asm;

    #[unsafe(no_mangle)]
    unsafe extern "C" fn _critical_section_1_0_acquire() -> u8 {
        let mstatus: usize;
        // csrrci: read CSR, then clear immediate bits.  Bit 3 = MIE.
        unsafe { asm!("csrrci {}, mstatus, 0b1000", out(reg) mstatus) };
        ((mstatus & 0b1000) != 0) as u8
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn _critical_section_1_0_release(token: u8) {
        if token != 0 {
            // Re-enable machine interrupts.
            unsafe { asm!("csrsi mstatus, 0b1000") };
        }
    }
}
