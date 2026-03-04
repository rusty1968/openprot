// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! I/O reactor for embedded kernels.
//!
//! Provides a [`Reactor`] that multiplexes waits across kernel objects using
//! a WaitGroup, and I/O futures that register with it. When no tasks are
//! ready, the executor calls [`Reactor::wait_for_events`] which blocks on
//! the WaitGroup — waking only when a registered object becomes ready.
//!
//! # Kernel abstraction
//!
//! The reactor is kernel-agnostic: it talks to the kernel through the
//! [`Kernel`] trait.  pw-kernel provides the concrete implementation; other
//! kernels (or test stubs) can implement the same trait.
//!
//! # Setup
//!
//! ```rust,ignore
//! // Initialize the reactor with your app's WaitGroup handle:
//! reactor::REACTOR.init(handle::WAIT_GROUP);
//!
//! // Pass reactor as the executor's idle strategy:
//! EXECUTOR.run(
//!     |spawner| { spawner.spawn(main_task(spawner)).unwrap(); },
//!     || reactor::REACTOR.wait_for_events(),
//! );
//! ```
//!
//! # Usage in tasks
//!
//! ```rust,ignore
//! let wr = reactor::object_wait(handle::IPC, Signals::READABLE).await?;
//! let signals = reactor::wait_interrupt(handle::IRQ, signals::MY_IRQ).await?;
//! ```

#![no_std]

use core::cell::{Cell, UnsafeCell};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

// ---------------------------------------------------------------------------
// Kernel abstraction
// ---------------------------------------------------------------------------

/// Signal bitmask for kernel object readiness.
///
/// Mirrors pw-kernel's `Signals` — a 32-bit bitflag.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Signals(pub u32);

impl Signals {
    pub const READABLE: Self = Self(1 << 0);
    pub const WRITEABLE: Self = Self(1 << 1);
    pub const ERROR: Self = Self(1 << 2);
    pub const USER: Self = Self(1 << 15);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn empty() -> Self {
        Self(0)
    }
}

impl core::ops::BitOr for Signals {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitAnd for Signals {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/// Return value from an `object_wait` syscall.
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct WaitReturn {
    /// `user_data` of the WaitGroup member that fired.
    pub user_data: usize,
    /// Signals pending on the object.
    pub pending_signals: Signals,
}

/// Error type for kernel syscall results.
///
/// Kept minimal — the reactor only cares about `DeadlineExceeded` (meaning
/// "not ready yet") vs other failures.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The deadline was exceeded before the condition was met.
    DeadlineExceeded,
    /// No free slots (or WaitGroup capacity reached).
    ResourceExhausted,
    /// Invalid handle or argument.
    InvalidArgument,
    /// Catch-all for other kernel errors.
    Other(i32),
}

/// Result type for kernel syscalls.
pub type Result<T> = core::result::Result<T, Error>;

/// Deadline constants for non-blocking and indefinite waits.
pub struct Deadline;

impl Deadline {
    /// Non-blocking: check and return immediately.
    pub const MIN: u64 = 0;
    /// Block forever (until condition is met).
    pub const MAX: u64 = u64::MAX;
}

/// Kernel interface trait.
///
/// Implementors provide the actual syscall bridge. pw-kernel's userspace
/// crate is the primary implementation; test environments can provide stubs.
pub trait Kernel {
    /// Non-blocking readiness check or blocking wait on a single object.
    ///
    /// - `deadline = Deadline::MIN` → non-blocking poll
    /// - `deadline = Deadline::MAX` → block until signals match
    fn object_wait(handle: u32, signal_mask: Signals, deadline: u64) -> Result<WaitReturn>;

    /// Add an object to a WaitGroup for multiplexed waiting.
    fn wait_group_add(
        wait_group: u32,
        object: u32,
        signal_mask: Signals,
        user_data: usize,
    ) -> Result<()>;

    /// Remove an object from a WaitGroup.
    fn wait_group_remove(wait_group: u32, object: u32) -> Result<()>;

    /// Acknowledge an interrupt (clear + re-enable).
    fn interrupt_ack(handle: u32, signal_mask: Signals) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Reactor
// ---------------------------------------------------------------------------

/// Maximum number of kernel objects that can be registered with the reactor
/// simultaneously.
pub const MAX_REACTOR_SLOTS: usize = 16;

/// I/O reactor that multiplexes waits across kernel objects via WaitGroup.
///
/// Futures register their kernel object handle with the reactor when they
/// return `Pending`. The executor's idle closure calls [`wait_for_events`]
/// which blocks on the WaitGroup until any registered object becomes ready,
/// then wakes the corresponding task.
///
/// # Type parameter
///
/// `K` is the [`Kernel`] implementation (e.g., pw-kernel's userspace syscall
/// bridge).  It is `PhantomData`-carried — no runtime storage.
///
/// # Safety model
///
/// Interior mutability via `UnsafeCell` is sound because:
/// - The executor is single-threaded and `!Send`
/// - `poll()` is never called reentrantly
/// - `wait_for_events()` is only called when no tasks are being polled
pub struct Reactor<K: Kernel> {
    wg_handle: Cell<u32>,
    wakers: [UnsafeCell<Option<Waker>>; MAX_REACTOR_SLOTS],
    used: Cell<u16>,
    _kernel: core::marker::PhantomData<K>,
}

// SAFETY: Reactor is used from a single-threaded, !Send executor.
// The global static requires Sync, but all access is non-concurrent.
unsafe impl<K: Kernel> Sync for Reactor<K> {}

impl<K: Kernel> Reactor<K> {
    /// Create an uninitialized reactor. Call [`init`] before use.
    pub const fn new() -> Self {
        const EMPTY_WAKER: UnsafeCell<Option<Waker>> = UnsafeCell::new(None);
        Self {
            wg_handle: Cell::new(0),
            wakers: [EMPTY_WAKER; MAX_REACTOR_SLOTS],
            used: Cell::new(0),
            _kernel: core::marker::PhantomData,
        }
    }

    /// Set the WaitGroup handle. Must be called before starting the executor.
    pub fn init(&self, wg_handle: u32) {
        self.wg_handle.set(wg_handle);
    }

    /// Register a kernel object handle for async readiness notification.
    ///
    /// Adds the object to the WaitGroup and stores the waker. Returns the
    /// slot index (used to update or deregister later).
    ///
    /// # Errors
    ///
    /// Returns `Error::ResourceExhausted` if all slots are occupied, or
    /// propagates errors from `wait_group_add`.
    pub fn register(
        &self,
        handle: u32,
        signals: Signals,
        waker: &Waker,
    ) -> Result<usize> {
        let used = self.used.get();
        let slot = find_free_slot(used).ok_or(Error::ResourceExhausted)?;

        K::wait_group_add(self.wg_handle.get(), handle, signals, slot)?;

        // SAFETY: single-threaded, non-reentrant — no concurrent access.
        unsafe { *self.wakers[slot].get() = Some(waker.clone()) };
        self.used.set(used | (1 << slot));

        Ok(slot)
    }

    /// Update the waker for an existing registration.
    ///
    /// Must be called on each `poll()` because the waker may change between
    /// polls (e.g., if the task is moved to a different executor slot).
    pub fn update_waker(&self, slot: usize, waker: &Waker) {
        // SAFETY: single-threaded, non-reentrant.
        unsafe { *self.wakers[slot].get() = Some(waker.clone()) };
    }

    /// Deregister a kernel object handle and free the slot.
    pub fn deregister(&self, slot: usize, handle: u32) {
        let _ = K::wait_group_remove(self.wg_handle.get(), handle);
        // SAFETY: single-threaded, non-reentrant.
        unsafe { *self.wakers[slot].get() = None };
        self.used.set(self.used.get() & !(1 << slot));
    }

    /// Block until any registered object becomes ready, then wake the
    /// corresponding task.
    ///
    /// Call this from the executor's idle closure. If no objects are
    /// registered, falls back to a spin-loop hint.
    pub fn wait_for_events(&self) {
        if self.used.get() == 0 {
            core::hint::spin_loop();
            return;
        }

        match K::object_wait(self.wg_handle.get(), Signals::READABLE, Deadline::MAX) {
            Ok(wait_return) => {
                let slot = wait_return.user_data;
                if slot < MAX_REACTOR_SLOTS {
                    // SAFETY: single-threaded, non-reentrant.
                    if let Some(waker) = unsafe { &*self.wakers[slot].get() } {
                        waker.wake_by_ref();
                    }
                }
            }
            Err(_) => {
                // Timeout or error — wake all registered tasks so they can
                // re-check their conditions.
                for i in 0..MAX_REACTOR_SLOTS {
                    if self.used.get() & (1 << i) != 0 {
                        // SAFETY: single-threaded, non-reentrant.
                        if let Some(waker) = unsafe { &*self.wakers[i].get() } {
                            waker.wake_by_ref();
                        }
                    }
                }
            }
        }
    }
}

/// Find the lowest free slot in the bitmask.
fn find_free_slot(used: u16) -> Option<usize> {
    if used == u16::MAX {
        return None;
    }
    Some((!used).trailing_zeros() as usize)
}

// ---------------------------------------------------------------------------
// I/O Futures
// ---------------------------------------------------------------------------

/// Future that waits for signals on a kernel object.
///
/// On first `Pending`, registers the object with the reactor.
/// The reactor's WaitGroup will wake this task when the object's signals
/// match. Automatically deregisters on completion or drop.
pub struct ObjectWaitFuture<'a, K: Kernel> {
    reactor: &'a Reactor<K>,
    handle: u32,
    signal_mask: Signals,
    slot: Option<usize>,
}

impl<K: Kernel> ObjectWaitFuture<'_, K> {
    fn deregister_if_needed(&mut self) {
        if let Some(slot) = self.slot.take() {
            self.reactor.deregister(slot, self.handle);
        }
    }
}

impl<K: Kernel> Future for ObjectWaitFuture<'_, K> {
    type Output = Result<WaitReturn>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match K::object_wait(self.handle, self.signal_mask, Deadline::MIN) {
            Ok(wait_return) => {
                self.deregister_if_needed();
                Poll::Ready(Ok(wait_return))
            }
            Err(Error::DeadlineExceeded) => {
                // Not ready — register or update waker.
                match self.slot {
                    Some(slot) => self.reactor.update_waker(slot, cx.waker()),
                    None => {
                        match self.reactor.register(self.handle, self.signal_mask, cx.waker()) {
                            Ok(slot) => self.slot = Some(slot),
                            Err(e) => return Poll::Ready(Err(e)),
                        }
                    }
                }
                Poll::Pending
            }
            Err(e) => {
                self.deregister_if_needed();
                Poll::Ready(Err(e))
            }
        }
    }
}

impl<K: Kernel> Drop for ObjectWaitFuture<'_, K> {
    fn drop(&mut self) {
        self.deregister_if_needed();
    }
}

/// Wait for signals on a kernel object handle.
///
/// Returns when any signal in `signal_mask` becomes pending on the object.
/// Registers with the provided reactor for efficient WaitGroup-based blocking.
pub fn object_wait<K: Kernel>(
    reactor: &Reactor<K>,
    handle: u32,
    signal_mask: Signals,
) -> ObjectWaitFuture<'_, K> {
    ObjectWaitFuture {
        reactor,
        handle,
        signal_mask,
        slot: None,
    }
}

/// Future that waits for an interrupt signal and acknowledges it.
///
/// On first `Pending`, registers with the reactor. When the interrupt fires,
/// automatically calls `interrupt_ack` to clear and re-enable it.
/// Deregisters on completion or drop.
pub struct InterruptFuture<'a, K: Kernel> {
    reactor: &'a Reactor<K>,
    handle: u32,
    signal_mask: Signals,
    slot: Option<usize>,
}

impl<K: Kernel> InterruptFuture<'_, K> {
    fn deregister_if_needed(&mut self) {
        if let Some(slot) = self.slot.take() {
            self.reactor.deregister(slot, self.handle);
        }
    }
}

impl<K: Kernel> Future for InterruptFuture<'_, K> {
    type Output = Result<Signals>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match K::object_wait(self.handle, self.signal_mask, Deadline::MIN) {
            Ok(wait_return) => {
                self.deregister_if_needed();
                let _ = K::interrupt_ack(self.handle, wait_return.pending_signals);
                Poll::Ready(Ok(wait_return.pending_signals))
            }
            Err(Error::DeadlineExceeded) => {
                match self.slot {
                    Some(slot) => self.reactor.update_waker(slot, cx.waker()),
                    None => {
                        match self.reactor.register(self.handle, self.signal_mask, cx.waker()) {
                            Ok(slot) => self.slot = Some(slot),
                            Err(e) => return Poll::Ready(Err(e)),
                        }
                    }
                }
                Poll::Pending
            }
            Err(e) => {
                self.deregister_if_needed();
                Poll::Ready(Err(e))
            }
        }
    }
}

impl<K: Kernel> Drop for InterruptFuture<'_, K> {
    fn drop(&mut self) {
        self.deregister_if_needed();
    }
}

/// Wait for an interrupt signal and auto-acknowledge it.
///
/// Returns the pending signals when the interrupt fires. The interrupt is
/// acknowledged automatically so it can fire again. Registers with the
/// provided reactor for efficient WaitGroup-based blocking.
pub fn wait_interrupt<K: Kernel>(
    reactor: &Reactor<K>,
    handle: u32,
    signal_mask: Signals,
) -> InterruptFuture<'_, K> {
    InterruptFuture {
        reactor,
        handle,
        signal_mask,
        slot: None,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};

    /// Mock kernel for testing — tracks calls and returns configurable results.
    static MOCK_WAIT_RESULT: AtomicU32 = AtomicU32::new(0);
    static MOCK_CALL_COUNT: AtomicU32 = AtomicU32::new(0);

    struct MockKernel;

    impl Kernel for MockKernel {
        fn object_wait(_handle: u32, _signal_mask: Signals, deadline: u64) -> Result<WaitReturn> {
            MOCK_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
            let val = MOCK_WAIT_RESULT.load(Ordering::Relaxed);
            if val > 0 {
                Ok(WaitReturn {
                    user_data: 0,
                    pending_signals: Signals::from_bits(val),
                })
            } else if deadline == Deadline::MIN {
                Err(Error::DeadlineExceeded)
            } else {
                // Blocking wait — pretend something fired on slot 0
                Ok(WaitReturn {
                    user_data: 0,
                    pending_signals: Signals::READABLE,
                })
            }
        }

        fn wait_group_add(
            _wait_group: u32,
            _object: u32,
            _signal_mask: Signals,
            _user_data: usize,
        ) -> Result<()> {
            Ok(())
        }

        fn wait_group_remove(_wait_group: u32, _object: u32) -> Result<()> {
            Ok(())
        }

        fn interrupt_ack(_handle: u32, _signal_mask: Signals) -> Result<()> {
            Ok(())
        }
    }

    fn reset_mock() {
        MOCK_WAIT_RESULT.store(0, Ordering::Relaxed);
        MOCK_CALL_COUNT.store(0, Ordering::Relaxed);
    }

    #[test]
    fn test_find_free_slot() {
        assert_eq!(find_free_slot(0b0000), Some(0));
        assert_eq!(find_free_slot(0b0001), Some(1));
        assert_eq!(find_free_slot(0b0011), Some(2));
        assert_eq!(find_free_slot(0b1111), Some(4));
        assert_eq!(find_free_slot(u16::MAX), None);
    }

    #[test]
    fn test_reactor_init() {
        let reactor: Reactor<MockKernel> = Reactor::new();
        reactor.init(42);
        assert_eq!(reactor.wg_handle.get(), 42);
        assert_eq!(reactor.used.get(), 0);
    }

    #[test]
    fn test_signals_bitops() {
        let s = Signals::READABLE | Signals::WRITEABLE;
        assert!(s.contains(Signals::READABLE));
        assert!(s.contains(Signals::WRITEABLE));
        assert!(!s.contains(Signals::ERROR));
        assert_eq!(s.bits(), 0b11);
    }

    #[test]
    fn test_reactor_register_deregister() {
        reset_mock();
        let reactor: Reactor<MockKernel> = Reactor::new();
        reactor.init(1);

        let waker = noop_waker();
        let slot = reactor.register(10, Signals::READABLE, &waker).unwrap();
        assert_eq!(slot, 0);
        assert_eq!(reactor.used.get(), 0b1);

        reactor.deregister(slot, 10);
        assert_eq!(reactor.used.get(), 0b0);
    }

    #[test]
    fn test_reactor_multiple_slots() {
        reset_mock();
        let reactor: Reactor<MockKernel> = Reactor::new();
        reactor.init(1);

        let waker = noop_waker();
        let s0 = reactor.register(10, Signals::READABLE, &waker).unwrap();
        let s1 = reactor.register(11, Signals::WRITEABLE, &waker).unwrap();
        let s2 = reactor.register(12, Signals::USER, &waker).unwrap();

        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(reactor.used.get(), 0b111);

        // Free middle slot, re-register takes it
        reactor.deregister(s1, 11);
        assert_eq!(reactor.used.get(), 0b101);

        let s3 = reactor.register(13, Signals::ERROR, &waker).unwrap();
        assert_eq!(s3, 1); // reuses freed slot
    }

    /// Minimal no-op waker for tests.
    fn noop_waker() -> Waker {
        use core::task::RawWaker;
        use core::task::RawWakerVTable;

        const VTABLE: RawWakerVTable = RawWakerVTable::new(
            |data| RawWaker::new(data, &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );

        // SAFETY: vtable functions are valid no-ops.
        unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
    }
}
