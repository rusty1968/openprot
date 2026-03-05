// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Async E2E test for executor + reactor on pw-kernel userspace.
//!
//! This test verifies the async runtime works correctly in pw-kernel userspace:
//! - Executor spawns and runs multiple concurrent tasks
//! - Tasks can yield and resume correctly
//! - Basic async/await patterns work

#![no_std]
#![no_main]

use core::sync::atomic::Ordering;
use critical_section::RawRestoreState;
use portable_atomic::{AtomicBool, AtomicU32};
use userspace::{entry, syscall};

// Pull in generated app code
use app_async_e2e_test as _;

// Null critical section for pw-kernel userspace.
// Userspace is single-threaded (cooperative scheduling), so no synchronization
// is needed. The kernel won't preempt us except at explicit yield points.
struct NullCriticalSection;
critical_section::set_impl!(NullCriticalSection);

// SAFETY: pw-kernel userspace is single-threaded. Interrupts are handled by
// the kernel, not userspace. Critical sections are only needed to prevent
// concurrent access from the same thread, which can't happen without yielding.
unsafe impl critical_section::Impl for NullCriticalSection {
    unsafe fn acquire() -> RawRestoreState {
        false
    }
    unsafe fn release(_token: RawRestoreState) {}
}

static TASK1_DONE: AtomicBool = AtomicBool::new(false);
static TASK2_DONE: AtomicBool = AtomicBool::new(false);
static TASK3_DONE: AtomicBool = AtomicBool::new(false);
static TASK4_DONE: AtomicBool = AtomicBool::new(false);
static COUNTER: AtomicU32 = AtomicU32::new(0);

// --- Async trait demonstration ----------------------------------------------

/// An async processing trait using native `async fn in trait` syntax
/// (stable since Rust 1.75; no heap allocation, no `async-trait` crate).
///
/// The trait is generic over concrete implementations (monomorphised),
/// which is the correct pattern for `no_std` embedded code — `dyn AsyncProcessor`
/// would require boxing and heap allocation.
trait AsyncProcessor {
    async fn process(&self, value: u32) -> u32;
}

/// Doubles the input value, yielding once before returning.
///
/// The yield proves the executor suspends and resumes across a suspension
/// point *inside* a trait method implementation.
struct DoublingProcessor;

impl AsyncProcessor for DoublingProcessor {
    async fn process(&self, value: u32) -> u32 {
        // Suspend here — the executor will drive this state machine back to
        // Poll::Ready on the next iteration.
        executor::yield_once().await;
        value.saturating_mul(2)
    }
}

/// Drive a processor through the trait interface, accumulating results.
///
/// Monomorphised over `P` — zero-cost abstraction, no vtable.
async fn run_processor<P: AsyncProcessor>(processor: &P) -> u32 {
    let mut total: u32 = 0;
    for i in 0..4_u32 {
        // .await on a trait async fn — exercises AFIT dispatch.
        let result = processor.process(i).await;
        total = total.saturating_add(result);
    }
    total // process(0..3) doubled: 0+2+4+6 = 12
}

/// Simple async task that increments counter and marks done.
#[embassy_executor::task]
async fn task1() {
    pw_log::info!("Task 1: starting");
    for _ in 0..5 {
        COUNTER.fetch_add(1, Ordering::SeqCst);
        executor::yield_once().await;
    }
    TASK1_DONE.store(true, Ordering::SeqCst);
    pw_log::info!("Task 1: done");
}

#[embassy_executor::task]
async fn task2() {
    pw_log::info!("Task 2: starting");
    for _ in 0..3 {
        COUNTER.fetch_add(10, Ordering::SeqCst);
        executor::yield_once().await;
    }
    TASK2_DONE.store(true, Ordering::SeqCst);
    pw_log::info!("Task 2: done");
}

#[embassy_executor::task]
async fn task3() {
    pw_log::info!("Task 3: starting");
    COUNTER.fetch_add(100, Ordering::SeqCst);
    TASK3_DONE.store(true, Ordering::SeqCst);
    pw_log::info!("Task 3: done");
}

/// Task 4: exercises `AsyncProcessor` trait — calls an async trait method
/// four times, each of which yields once inside the implementation.
/// Accumulates 0+2+4+6 = 12 into COUNTER.
#[embassy_executor::task]
async fn task4() {
    pw_log::info!("Task 4: starting async trait demo");
    let processor = DoublingProcessor;
    let total = run_processor(&processor).await;
    COUNTER.fetch_add(total, Ordering::SeqCst);
    TASK4_DONE.store(true, Ordering::SeqCst);
    pw_log::info!("Task 4: done, processor total={}", total as u32);
}

#[entry]
fn entry_point() -> ! {
    pw_log::info!("Async E2E Test");

    // Start executor with idle closure that checks for completion
    executor::start_async_with_idle(
        |spawner| {
            spawner.spawn(task1()).unwrap();
            spawner.spawn(task2()).unwrap();
            spawner.spawn(task3()).unwrap();
            spawner.spawn(task4()).unwrap();
        },
        || {
            // Check if all tasks done
            if TASK1_DONE.load(Ordering::SeqCst)
                && TASK2_DONE.load(Ordering::SeqCst)
                && TASK3_DONE.load(Ordering::SeqCst)
                && TASK4_DONE.load(Ordering::SeqCst)
            {
                let counter = COUNTER.load(Ordering::SeqCst);
                // task1: 5×1=5, task2: 3×10=30, task3: 100, task4: 0+2+4+6=12 → 147
                if counter == 147 {
                    pw_log::info!("PASSED counter={}", counter as u32);
                    let _ = syscall::debug_shutdown(Ok(()));
                } else {
                    pw_log::error!("FAILED counter={} expected 147", counter as u32);
                    let _ = syscall::debug_shutdown(Err(pw_status::Error::Unknown));
                }
            }
        },
    );
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    pw_log::error!("PANIC");
    let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
    loop {}
}
