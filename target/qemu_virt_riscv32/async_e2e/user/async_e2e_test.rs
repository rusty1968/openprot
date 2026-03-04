// Licensed under the Apache-2.0 license

//! Async E2E test for executor + reactor on pw-kernel userspace.
//!
//! This test verifies the async runtime works correctly in pw-kernel userspace:
//! - Executor spawns and runs multiple concurrent tasks
//! - Tasks can yield and resume correctly
//! - Basic async/await patterns work

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};
use critical_section::RawRestoreState;
use portable_atomic::AtomicBool;
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
static COUNTER: AtomicU32 = AtomicU32::new(0);

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

#[entry]
fn entry_point() -> ! {
    pw_log::info!("Async E2E Test");

    // Start executor with idle closure that checks for completion
    executor::start_async_with_idle(
        |spawner| {
            spawner.spawn(task1()).unwrap();
            spawner.spawn(task2()).unwrap();
            spawner.spawn(task3()).unwrap();
        },
        || {
            // Check if all tasks done
            if TASK1_DONE.load(Ordering::SeqCst)
                && TASK2_DONE.load(Ordering::SeqCst)
                && TASK3_DONE.load(Ordering::SeqCst)
            {
                let counter = COUNTER.load(Ordering::SeqCst);
                if counter == 135 {
                    pw_log::info!("PASSED counter={}", counter as u32);
                    let _ = syscall::debug_shutdown(Ok(()));
                } else {
                    pw_log::error!("FAILED counter={} expected 135", counter as u32);
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
