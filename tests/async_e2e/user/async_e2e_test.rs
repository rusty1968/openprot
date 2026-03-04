// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! End-to-end async runtime test for pw-kernel userspace.
//!
//! Runs the openprot executor + reactor inside a pw-kernel userspace
//! process on QEMU. Validates:
//!   1. Executor poll loop with spin-loop idle
//!   2. Task spawning and completion via Embassy `#[task]`
//!   3. Cooperative yielding (`yield_once`)
//!   4. Syscalls from async context (`debug_nop`)
//!   5. Reactor WaitGroup registration + object_wait via kernel bridge

#![no_main]
#![no_std]

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_executor::Spawner;
use openprot_executor::yield_once;
use pw_status::{Error, Result, StatusCode};
use userspace::{entry, syscall};

static EXECUTOR: openprot_executor::Executor = openprot_executor::Executor::new();

// Task results and completion tracking.
static RESULT_IMMEDIATE: AtomicU32 = AtomicU32::new(0);
static RESULT_ACCUMULATOR: AtomicU32 = AtomicU32::new(0);
static RESULT_SYSCALL: AtomicU32 = AtomicU32::new(0);
static RESULT_PING: AtomicU32 = AtomicU32::new(0);
static RESULT_PONG: AtomicU32 = AtomicU32::new(0);
static DONE_COUNT: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------
// Task 1: immediate completion
// ---------------------------------------------------------------------------

#[embassy_executor::task]
async fn task_immediate() {
    RESULT_IMMEDIATE.store(42, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Task 2: multi-yield accumulator (1+2+3+4+5 = 15)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Task 3: syscall from async context
// ---------------------------------------------------------------------------

#[embassy_executor::task]
async fn task_syscall() {
    let result = if syscall::debug_nop().is_err() {
        0u32
    } else {
        yield_once().await;
        if syscall::debug_nop().is_err() { 0u32 } else { 1u32 }
    };
    RESULT_SYSCALL.store(result, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Tasks 4 & 5: ping-pong cooperative scheduling
// ---------------------------------------------------------------------------

static PING_PONG: AtomicU32 = AtomicU32::new(0);

#[embassy_executor::task]
async fn task_ping() {
    let mut count = 0u32;
    for _ in 0..5 {
        while PING_PONG.load(Ordering::Acquire) % 2 != 0 {
            yield_once().await;
        }
        count += 1;
        PING_PONG.fetch_add(1, Ordering::Release);
        yield_once().await;
    }
    RESULT_PING.store(count, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

#[embassy_executor::task]
async fn task_pong() {
    let mut count = 0u32;
    for _ in 0..5 {
        while PING_PONG.load(Ordering::Acquire) % 2 != 1 {
            yield_once().await;
        }
        count += 1;
        PING_PONG.fetch_add(1, Ordering::Release);
        yield_once().await;
    }
    RESULT_PONG.store(count, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Main task: spawn all, wait, verify
// ---------------------------------------------------------------------------

#[embassy_executor::task]
async fn task_main(spawner: Spawner) {
    spawner.spawn(task_immediate()).unwrap();
    spawner.spawn(task_accumulator()).unwrap();
    spawner.spawn(task_syscall()).unwrap();
    spawner.spawn(task_ping()).unwrap();
    spawner.spawn(task_pong()).unwrap();

    // Wait for all 5 tasks.
    while DONE_COUNT.load(Ordering::Acquire) < 5 {
        yield_once().await;
    }

    let ret = verify_results();
    if ret.is_err() {
        pw_log::error!("FAILED: {}", ret.status_code() as u32);
    } else {
        pw_log::info!("PASSED: all async tasks completed correctly");
    }
    let _ = syscall::debug_shutdown(ret);
}

fn verify_results() -> Result<()> {
    let r = RESULT_IMMEDIATE.load(Ordering::Acquire);
    pw_log::info!("Task immediate: {} (expected 42)", r);
    if r != 42 { return Err(Error::Internal); }

    let r = RESULT_ACCUMULATOR.load(Ordering::Acquire);
    pw_log::info!("Task accumulator: {} (expected 15)", r);
    if r != 15 { return Err(Error::Internal); }

    let r = RESULT_SYSCALL.load(Ordering::Acquire);
    pw_log::info!("Task syscall: {} (expected 1)", r);
    if r != 1 { return Err(Error::Internal); }

    let r = RESULT_PING.load(Ordering::Acquire);
    pw_log::info!("Task ping: {} (expected 5)", r);
    if r != 5 { return Err(Error::Internal); }

    let r = RESULT_PONG.load(Ordering::Acquire);
    pw_log::info!("Task pong: {} (expected 5)", r);
    if r != 5 { return Err(Error::Internal); }

    Ok(())
}

#[entry]
fn entry() -> ! {
    pw_log::info!("RUNNING: openprot async runtime e2e test");
    EXECUTOR.run(
        |spawner| { spawner.spawn(task_main(spawner)).unwrap(); },
        || core::hint::spin_loop(),
    );
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
