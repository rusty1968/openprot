// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Type-erased ECDSA operation adapter: owns the bounded completion-poll loop.
//!
//! Operation adapter of the *Cooperative-Yield Bounded-Poll Device* pattern.
//! Built from an [`EcdsaDevice`](super::device::EcdsaDevice) by a borrow split
//! — `regs`/`poll_budget` are `Copy`d out, `yield_fn` is reborrowed and
//! **type-erased** to `&mut dyn FnMut(u32)` so this adapter (and any future
//! verify/protocol impls on it) need not be generic over the strategy.

use super::constants::POLL_YIELD_NS;
use super::error::EcdsaError;
use super::registers::EcdsaRegisters;

pub struct EcdsaOp<'a> {
    pub(crate) regs: EcdsaRegisters,
    pub(crate) poll_budget: u32,
    /// Cooperative yield hook, borrowed from the originating
    /// [`EcdsaDevice`](super::device::EcdsaDevice) and invoked once between
    /// every completion poll. Type-erased so the adapter (and the future
    /// verify/protocol impls) need not be generic over the strategy.
    pub(crate) yield_fn: &'a mut dyn FnMut(u32),
}

impl<'a> EcdsaOp<'a> {
    /// Construct an operation adapter from an existing register handle, poll
    /// budget, and cooperative yield hook.
    pub(crate) fn new(
        regs: EcdsaRegisters,
        poll_budget: u32,
        yield_fn: &'a mut dyn FnMut(u32),
    ) -> Self {
        Self {
            regs,
            poll_budget,
            yield_fn,
        }
    }

    /// Construct an operation adapter from an [`EcdsaDevice`].
    ///
    /// `regs`/`poll_budget` are `Copy`; the only retained borrow is the
    /// disjoint `yield_fn` field, reborrowed (never moved/copied) for `'a`.
    ///
    /// # Safety
    /// Caller must ensure no concurrent or reentrant ECDSA access for the
    /// lifetime of the returned [`EcdsaOp`].
    pub unsafe fn from_device<Y: FnMut(u32)>(
        device: &'a mut super::device::EcdsaDevice<Y>,
    ) -> Self {
        let regs = device.regs;
        let poll_budget = device.poll_budget;
        let yield_fn: &'a mut dyn FnMut(u32) = &mut device.yield_fn;
        Self::new(regs, poll_budget, yield_fn)
    }

    /// Wait for the in-flight ECDSA verification to complete, bounded by the
    /// poll budget.
    ///
    /// This is the *wait-policy seam only*: the bounded loop polls the safe
    /// façade predicate [`EcdsaRegisters::verify_is_done`], invokes the
    /// injected strategy once per non-completing poll with an advisory ns
    /// window, and on budget exhaustion performs façade cleanup then returns a
    /// typed [`EcdsaError::Timeout`] — never an unbounded spin.
    ///
    /// The façade predicate is currently a `todo!()` stub (it maps to
    /// `secure014` bit-20 per `aspeed-rust/src/ecdsa.rs`); the verify *result*
    /// decode and the trigger sequence land with the verify semantics under
    /// the `peripheral-parity-port` workflow. This method wires the structural
    /// seam, not the behavior.
    pub fn wait_verify_done(&mut self) -> Result<(), EcdsaError> {
        for _ in 0..self.poll_budget {
            if self.regs.verify_is_done() {
                // safe façade predicate
                return Ok(());
            }
            (self.yield_fn)(POLL_YIELD_NS); // injected strategy, advisory ns
        }
        self.regs.clear_status(); // façade cleanup
        Err(EcdsaError::Timeout) // typed, bounded failure
    }
}
