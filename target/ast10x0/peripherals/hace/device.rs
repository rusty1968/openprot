// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! HACE device binding with cooperative yield.

use super::constants::DEFAULT_POLL_BUDGET;
use super::registers::HaceRegisters;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HashAlgo {
    Sha256,
}

pub struct HaceDevice<Y: FnMut(u32)> {
    pub(crate) regs: HaceRegisters,
    /// Cooperative yield hook invoked between completion polls.
    /// Argument is a suggested wait window in nanoseconds.
    #[allow(dead_code)]
    yield_fn: Y,
    pub(crate) poll_budget: u32,
}

impl<Y: FnMut(u32)> HaceDevice<Y> {
    /// Create a device bound to a raw HACE register block with a caller-provided
    /// cooperative yield strategy.
    ///
    /// # Safety
    /// Caller must uphold the same safety contract as [`HaceRegisters::new`].
    /// This type is non-reentrant: only one `HaceDevice` may be active at a time.
    pub unsafe fn new_with_yield(
        base: *const ast1060_pac::hace::RegisterBlock,
        yield_fn: Y,
    ) -> Self {
        Self {
            // SAFETY: Caller upholds register-pointer validity/ownership.
            regs: unsafe { HaceRegisters::new(base) },
            yield_fn,
            poll_budget: DEFAULT_POLL_BUDGET,
        }
    }

    /// Create a device bound to a raw HACE register block.
    ///
    /// # Safety
    /// Caller must uphold the same safety contract as [`HaceRegisters::new`].
    /// This type is non-reentrant: only one `HaceDevice` may be active at a time.
    pub unsafe fn new(base: *const ast1060_pac::hace::RegisterBlock, yield_fn: Y) -> Self {
        // SAFETY: Same contract as this wrapper.
        unsafe { Self::new_with_yield(base, yield_fn) }
    }

    /// Create a device bound to the singleton HACE instance with a caller-provided
    /// cooperative yield strategy.
    ///
    /// # Safety
    /// Caller must coordinate singleton access globally.
    /// This type is non-reentrant: only one `HaceDevice` may be active at a time.
    pub unsafe fn new_global_with_yield(yield_fn: Y) -> Self {
        Self {
            // SAFETY: Caller coordinates singleton access.
            regs: unsafe { HaceRegisters::new_global() },
            yield_fn,
            poll_budget: DEFAULT_POLL_BUDGET,
        }
    }

    /// Create a device bound to the singleton HACE instance.
    ///
    /// # Safety
    /// Caller must coordinate singleton access globally.
    /// This type is non-reentrant: only one `HaceDevice` may be active at a time.
    pub unsafe fn new_global(yield_fn: Y) -> Self {
        // SAFETY: Same contract as this wrapper.
        unsafe { Self::new_global_with_yield(yield_fn) }
    }

    /// Override polling timeout budget for operation completion.
    #[must_use]
    pub const fn with_timeout_polls(mut self, timeout_polls: u32) -> Self {
        self.poll_budget = timeout_polls;
        self
    }
}
