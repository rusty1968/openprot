// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Minimal async executor for embedded systems.
//!
//! Wraps Embassy's [`raw::Executor`] to provide waker-based task scheduling
//! with dynamic spawning via [`Spawner`]. Tasks are only re-polled when their
//! waker is invoked, avoiding unnecessary work.
//!
//! This is the Phase 1 executor — a pure-Rust poll loop with no kernel
//! dependencies. Phase 2 adds a reactor that blocks via `object_wait` when
//! no tasks are ready.

#![no_std]

use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

pub use embassy_executor::Spawner;
use embassy_executor::raw;
use portable_atomic::{AtomicBool, Ordering};

/// Global flag set by the pender when any task becomes ready.
///
/// Embassy calls [`__pender`] whenever a waker fires. The executor checks
/// this flag each iteration to decide whether to poll or spin.
static SIGNAL_WORK: AtomicBool = AtomicBool::new(false);

/// Pender callback invoked by Embassy when a task is woken.
///
/// Can be called from any context (interrupt, another thread, etc.).
/// Must NOT call `poll()` — just sets the flag for the main loop.
#[unsafe(export_name = "__pender")]
fn __pender(_context: *mut ()) {
    SIGNAL_WORK.store(true, Ordering::SeqCst);
}

/// Async executor backed by Embassy's raw executor.
///
/// Provides dynamic task spawning via [`Spawner`] and waker-based scheduling.
/// Create with [`Executor::new()`], then call [`Executor::run()`] with an
/// init closure that spawns your tasks.
///
/// # Construction
///
/// `Executor::new()` is **not** const (embassy-executor 0.9.x). Use one of:
/// - [`start_async()`] convenience function (recommended for `#[entry]`)
/// - Runtime init + unsafe transmute to `&'static` if you need a global
pub struct Executor {
    inner: raw::Executor,
    not_send: PhantomData<*mut ()>,
}

impl Executor {
    /// Create a new executor.
    ///
    /// This is a runtime constructor — `raw::Executor::new()` is not const
    /// in embassy-executor 0.9.x.
    pub fn new() -> Self {
        Self {
            inner: raw::Executor::new(core::ptr::null_mut()),
            not_send: PhantomData,
        }
    }

    /// Run the executor forever.
    ///
    /// The `init` closure receives a [`Spawner`] to spawn the initial tasks.
    /// After `init` returns, the executor polls tasks in a loop. Tasks can
    /// spawn additional tasks by holding a copy of the `Spawner`.
    ///
    /// The `idle` closure is called when no tasks are ready. Use it to block
    /// efficiently (e.g., via the reactor's `wait_for_events()`) or spin.
    ///
    /// # Idle strategies
    ///
    /// | Strategy | Code | When to use |
    /// |----------|------|-------------|
    /// | Spin | `\|\| core::hint::spin_loop()` | Testing, tasks that self-wake |
    /// | Reactor | `\|\| reactor.wait_for_events()` | Production — blocks until I/O |
    /// | Kernel | `\|\| object_wait(h, Signals::USER, MAX)` | Single-object blocking |
    ///
    /// This function never returns.
    pub fn run(&'static self, init: impl FnOnce(Spawner), idle: impl Fn()) -> ! {
        init(self.inner.spawner());

        loop {
            // SAFETY: we are the only caller of poll() and we don't call it
            // reentrantly (the pender only sets a flag).
            unsafe { self.inner.poll() };

            // Check-and-clear inside a critical section to avoid races
            // with the pender (which may fire from an interrupt).
            // On rv32imc (no A extension) AtomicBool::swap is unavailable,
            // so we use load+store guarded by critical_section.
            critical_section::with(|_| {
                if SIGNAL_WORK.load(Ordering::SeqCst) {
                    SIGNAL_WORK.store(false, Ordering::SeqCst);
                } else {
                    idle();
                }
            });
        }
    }
}

/// Convenience: create an executor on the stack, leak it to `&'static`, and
/// run with a spin-loop idle strategy.
///
/// This is the simplest way to use the executor from an entry point:
/// ```ignore
/// #[entry]
/// fn main() -> ! {
///     start_async(|spawner| {
///         spawner.spawn(my_task()).unwrap();
///     });
/// }
/// ```
///
/// For production use with a reactor, prefer constructing `Executor::new()`
/// and calling `run()` with a custom idle closure.
///
/// # Safety
///
/// Uses `core::mem::transmute` to extend the executor's lifetime to `'static`.
/// This is safe because `run()` never returns, so the stack frame is never
/// deallocated.
pub fn start_async(init: impl FnOnce(Spawner)) -> ! {
    let executor = Executor::new();
    // SAFETY: run() diverges (-> !), so the stack-allocated executor lives
    // forever. The transmute converts &Executor to &'static Executor.
    let executor: &'static Executor = unsafe { core::mem::transmute(&executor) };
    executor.run(init, || core::hint::spin_loop());
}

// --- Utility futures --------------------------------------------------------

/// A future that yields once (returning [`Poll::Pending`]), then completes.
///
/// Properly wakes itself before returning `Pending` so the executor knows
/// to re-poll on the next iteration.
pub struct YieldOnce {
    yielded: bool,
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Create a future that yields execution once, then completes.
pub fn yield_once() -> YieldOnce {
    YieldOnce { yielded: false }
}
