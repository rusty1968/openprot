// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 ECDSA low-level register access.
//!
//! The AST10x0 ECDSA/ECC engine has **no dedicated PAC register block**: it is
//! part of the Secure Boot Controller (SBC), exposed by `ast1060_pac` as the
//! `secure` peripheral (`ast1060_pac::secure::RegisterBlock`, base
//! `0x7e6f_2000`). This is confirmed against the reference driver
//! `aspeed-rust/src/ecdsa.rs` (`ECDSA_BASE = 0x7e6f_2000 // SBC base address`).
//!
//! This is the Confined-`unsafe` MMIO Façade for that engine. The curated
//! operation surface is intentionally a stub: the ECDSA API is filled in later
//! under the `peripheral-parity-port` workflow, once its behavioral spec
//! exists. No driver and no behavior live here.

use core::marker::PhantomData;

use ast1060_pac as device;

/// Safe wrapper around the AST10x0 ECDSA engine register block.
///
/// The ECDSA engine is hosted by the SBC, so the underlying PAC block is
/// `device::secure::RegisterBlock`.
///
/// This is `Copy`/`Clone` by design: the façade *confines* `unsafe` and
/// *restricts threading* (`!Send`/`!Sync`); it deliberately does **not**
/// enforce exclusive access. Serialization/exclusivity is delegated to the
/// constructor's caller and to a future scoped/owned session layer added on
/// top under `peripheral-parity-port`.
// `allow(dead_code)`: the façade currently has no consumer — the driver/session
// layer lands later under `peripheral-parity-port`. Remove this attribute when
// the first consumer exists.
#[allow(dead_code)]
#[derive(Copy, Clone)]
pub struct EcdsaRegisters {
    ptr: *mut device::secure::RegisterBlock,
    _not_send: PhantomData<*mut ()>,
}

#[allow(dead_code)] // see note on `EcdsaRegisters`; drop when a consumer lands.
impl EcdsaRegisters {
    /// Create a register accessor from a raw SBC/ECDSA register block pointer.
    ///
    /// # Safety
    /// Caller must ensure:
    /// - `base` points to a valid SBC (`secure`) register block.
    /// - access to the ECDSA engine instance is serialized appropriately.
    pub const unsafe fn new(base: *const device::secure::RegisterBlock) -> Self {
        Self {
            ptr: base as *mut device::secure::RegisterBlock,
            _not_send: PhantomData,
        }
    }

    /// Create a register accessor for the global ECDSA (SBC) instance.
    ///
    /// # Safety
    /// Caller must ensure access to the singleton SBC is coordinated.
    pub const unsafe fn new_global() -> Self {
        // SAFETY: Caller upholds the singleton access contract.
        unsafe { Self::new(device::Secure::ptr()) }
    }

    #[inline]
    pub(crate) fn regs(&self) -> &device::secure::RegisterBlock {
        // SAFETY: Constructor guarantees a valid SBC register block pointer.
        unsafe { &*self.ptr }
    }

    // --- curated, intent-named, safe operations ---
    //
    // STUB: deliberately not implemented yet. The ECDSA register-level API is
    // designed and filled in under the `peripheral-parity-port` workflow once
    // the behavioral spec is pinned. Each operation will document its own
    // register-ordering / completion assumptions when added. Raw `.bits()` and
    // all PAC types stay confined below this line — they never escape the
    // façade.

    /// Trigger an ECDSA signature verification on the engine.
    ///
    /// STUB — fill under `peripheral-parity-port` (will document the
    /// parameter/start-bit ordering it relies on).
    #[inline]
    pub(crate) fn start_verify(&self) {
        let _ = self.regs();
        todo!("ECDSA façade operation not yet implemented")
    }

    /// Report whether the engine has completed the in-flight operation.
    ///
    /// STUB — fill under `peripheral-parity-port`.
    #[inline]
    pub(crate) fn verify_is_done(&self) -> bool {
        let _ = self.regs();
        todo!("ECDSA façade operation not yet implemented")
    }

    /// Clear the engine status/interrupt flags.
    ///
    /// STUB — fill under `peripheral-parity-port`.
    #[inline]
    pub(crate) fn clear_status(&self) {
        let _ = self.regs();
        todo!("ECDSA façade operation not yet implemented")
    }
}
