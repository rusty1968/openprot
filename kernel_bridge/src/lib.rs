// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! pw-kernel bridge for the OpenPRoT reactor.
//!
//! Provides [`PwKernel`], the concrete [`Kernel`] implementation that
//! forwards reactor trait methods to pw-kernel's userspace syscalls.
//!
//! # Build requirements
//!
//! This crate only builds when targeting a pw-kernel userspace
//! configuration (`@pigweed//pw_kernel/userspace:userspace_build` is
//! True). It is not buildable on host or bare-metal QEMU — those
//! should use a mock kernel or the test stubs in the reactor crate.
//!
//! # Usage
//!
//! ```rust,ignore
//! use openprot_reactor::{Reactor, Signals};
//! use openprot_kernel_bridge::PwKernel;
//!
//! // Type alias for convenience:
//! type AppReactor = Reactor<PwKernel>;
//! static REACTOR: AppReactor = Reactor::new();
//!
//! // In your entry point:
//! REACTOR.init(handle::WAIT_GROUP);
//! EXECUTOR.run(
//!     |spawner| { spawner.spawn(main_task(spawner)).unwrap(); },
//!     || REACTOR.wait_for_events(),
//! );
//! ```

#![no_std]

use openprot_reactor::{self as reactor, Kernel, Signals, WaitReturn};
use userspace::syscall;
use userspace::time::Instant;

// ---------------------------------------------------------------------------
// Type conversion helpers
// ---------------------------------------------------------------------------

/// Convert our kernel-agnostic `Signals` to pw-kernel's `syscall_defs::Signals`.
///
/// Both are u32 newtypes — we go through `.bits()` for safety.
#[inline(always)]
fn to_kernel_signals(s: Signals) -> syscall::Signals {
    syscall::Signals::from_bits_truncate(s.bits())
}

/// Convert pw-kernel's `syscall_defs::Signals` to our kernel-agnostic `Signals`.
#[inline(always)]
fn from_kernel_signals(s: syscall::Signals) -> Signals {
    Signals::from_bits(s.bits())
}

/// Convert pw-kernel's `WaitReturn` to our kernel-agnostic `WaitReturn`.
#[inline(always)]
fn from_kernel_wait_return(wr: syscall::WaitReturn) -> WaitReturn {
    WaitReturn {
        user_data: wr.user_data,
        pending_signals: from_kernel_signals(wr.pending_signals),
    }
}

/// Convert `pw_status::Error` to our reactor `Error`.
#[inline(always)]
fn convert_error(e: pw_status::Error) -> reactor::Error {
    match e {
        pw_status::Error::DeadlineExceeded => reactor::Error::DeadlineExceeded,
        pw_status::Error::ResourceExhausted => reactor::Error::ResourceExhausted,
        pw_status::Error::InvalidArgument => reactor::Error::InvalidArgument,
        other => reactor::Error::Other(other as i32),
    }
}

/// Convert a `pw_status::Result<T>` to a `reactor::Result<T>` with a mapper
/// for the `Ok` value.
#[inline(always)]
fn convert_result<T, U>(
    result: pw_status::Result<T>,
    map_ok: impl FnOnce(T) -> U,
) -> reactor::Result<U> {
    result.map(map_ok).map_err(convert_error)
}

// ---------------------------------------------------------------------------
// Kernel implementation
// ---------------------------------------------------------------------------

/// pw-kernel userspace implementation of the reactor's [`Kernel`] trait.
///
/// This is a zero-sized type — all methods are stateless forwarding calls
/// to `userspace::syscall::*`.
pub struct PwKernel;

impl Kernel for PwKernel {
    /// Non-blocking or blocking wait on a kernel object.
    ///
    /// The `deadline` parameter is interpreted as raw timer ticks
    /// (matching `Instant::from_ticks`):
    /// - `0` → non-blocking poll (`Instant::MIN`)
    /// - `u64::MAX` → block forever (`Instant::MAX`)
    fn object_wait(
        handle: u32,
        signal_mask: Signals,
        deadline: u64,
    ) -> reactor::Result<WaitReturn> {
        convert_result(
            syscall::object_wait(handle, to_kernel_signals(signal_mask), Instant::from_ticks(deadline)),
            from_kernel_wait_return,
        )
    }

    /// Add a kernel object to a WaitGroup for multiplexed waiting.
    fn wait_group_add(
        wait_group: u32,
        object: u32,
        signal_mask: Signals,
        user_data: usize,
    ) -> reactor::Result<()> {
        syscall::wait_group_add(wait_group, object, to_kernel_signals(signal_mask), user_data)
            .map_err(convert_error)
    }

    /// Remove a kernel object from a WaitGroup.
    fn wait_group_remove(wait_group: u32, object: u32) -> reactor::Result<()> {
        syscall::wait_group_remove(wait_group, object).map_err(convert_error)
    }

    /// Acknowledge an interrupt (clear + re-enable).
    fn interrupt_ack(handle: u32, signal_mask: Signals) -> reactor::Result<()> {
        syscall::interrupt_ack(handle, to_kernel_signals(signal_mask)).map_err(convert_error)
    }
}
