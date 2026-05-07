// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Low-level register access
//!
//! Consolidates all unsafe hardware register access into a single unsafe
//! perimeter, following the AST1060 PAC guide pattern.
//!
//! # Architecture: Pure Register Transport
//!
//! This backend implements the `SmcRegisterBackend` trait with zero topology logic.
//! All register operations are direct hardware accessors:
//! - `read_*()`: Read raw register bits
//! - `write_*()`: Write raw register bits
//! - `modify_*()`: Read-modify-write with closure
//!
//! Topology-aware decisions (when to call these accessors, which registers matter
//! for a given controller role, decode-range sizing, calibration gating, etc.)
//! live exclusively in `controller.rs`, not here.
//!
//! This keeps the backend simple and the topology logic centralized.

use ast1060_pac as device;
use core::marker::PhantomData;
use core::cell::UnsafeCell;

/// Safe wrapper around SMC hardware registers
///
/// This struct consolidates all unsafe hardware access. All register operations
/// go through this single point, making it easy to audit safety invariants.
///
/// **No topology logic here.** This backend is pure register transport.
/// Topology-aware behavior (when to use which registers, decode-range sizing,
/// calibration gating) is gated by `SmcConfig.topology` in the controller layer.
///
/// Register naming follows AST1060 PAC convention: methods are named by their
/// hex offsets (e.g., `fmc000()` for offset 0x00, `fmc080()` for offset 0x80).
pub struct FmcRegisterBackend {
    base: *const device::fmc::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>, // Prevent Sync, allow Send
}

impl FmcRegisterBackend {
    /// Create a new register accessor
    ///
    /// # Safety
    /// Caller must ensure:
    /// - `base` points to a valid FMC register block
    /// - Only one FmcRegisterBackend instance exists per register block
    /// - Caller maintains exclusive access (no concurrent mutations)
    pub const unsafe fn new(base: *const device::fmc::RegisterBlock) -> Self {
        Self {
            base,
            _not_sync: PhantomData,
        }
    }

    /// Access the register block (single consolidation point for unsafe)
    ///
    /// # Safety
    /// Constructor must have ensured pointer validity and single ownership
    #[inline]
    fn regs(&self) -> &device::fmc::RegisterBlock {
        // SAFETY: Constructor ensures pointer validity and exclusive access.
        // Only one FmcRegisterBackend instance can be created per hardware controller.
        unsafe { &*self.base }
    }
}

// ====== Trait implementation (direct — no redundant delegation) ======

impl crate::smc::register_traits::SmcRegisterBackend for FmcRegisterBackend {
    fn read_config(&self) -> u32 {
        self.regs().fmc000().read().bits()
    }

    fn write_config(&self, value: u32) {
        self.regs().fmc000().write(|w| unsafe { w.bits(value) });
    }

    fn modify_config<F>(&self, f: F) where F: FnOnce(&mut u32) {
        self.regs().fmc000().modify(|r, w| {
            let mut bits = r.bits();
            f(&mut bits);
            unsafe { w.bits(bits) }
        });
    }

    fn read_addr_width(&self) -> u32 {
        self.regs().fmc004().read().bits()
    }

    fn write_addr_width(&self, value: u32) {
        self.regs().fmc004().write(|w| unsafe { w.bits(value) });
    }

    // NOTE: FMC uses named field accessors; SPI uses raw bits for same register.
    fn read_dma_status(&self) -> u32 {
        self.regs().fmc008().read().bits()
    }

    fn clear_dma_status(&self, clear_mask: u32) {
        self.regs().fmc008().write(|w| unsafe { w.bits(clear_mask) });
    }

    fn enable_dma_irq(&self) {
        self.regs().fmc008().modify(|_, w| w.dmaintenbl().set_bit());
    }

    fn disable_dma_irq(&self) {
        self.regs().fmc008().modify(|_, w| w.dmaintenbl().clear_bit());
    }

    fn read_cs0_ctrl(&self) -> u32 {
        self.regs().fmc010().read().bits()
    }

    fn write_cs0_ctrl(&self, value: u32) {
        self.regs().fmc010().write(|w| unsafe { w.bits(value) });
    }

    fn read_cs1_ctrl(&self) -> u32 {
        self.regs().fmc014().read().bits()
    }

    fn write_cs1_ctrl(&self, value: u32) {
        self.regs().fmc014().write(|w| unsafe { w.bits(value) });
    }

    fn read_cs0_segment(&self) -> u32 {
        self.regs().fmc030().read().bits()
    }

    fn write_cs0_segment(&self, value: u32) {
        self.regs().fmc030().write(|w| unsafe { w.bits(value) });
    }

    fn read_cs1_segment(&self) -> u32 {
        self.regs().fmc034().read().bits()
    }

    fn write_cs1_segment(&self, value: u32) {
        self.regs().fmc034().write(|w| unsafe { w.bits(value) });
    }

    fn read_spi_mode(&self) -> u32 {
        self.regs().fmc06c().read().bits()
    }

    fn write_spi_mode(&self, value: u32) {
        self.regs().fmc06c().write(|w| unsafe { w.bits(value) });
    }

    fn modify_spi_mode<F>(&self, f: F) where F: FnOnce(&mut u32) {
        self.regs().fmc06c().modify(|r, w| {
            let mut bits = r.bits();
            f(&mut bits);
            unsafe { w.bits(bits) }
        });
    }

    fn read_dma_ctrl(&self) -> u32 {
        self.regs().fmc080().read().bits()
    }

    fn write_dma_ctrl(&self, value: u32) {
        self.regs().fmc080().write(|w| unsafe { w.bits(value) });
    }

    fn read_dma_addr(&self) -> u32 {
        self.regs().fmc084().read().bits()
    }

    fn write_dma_addr(&self, value: u32) {
        self.regs().fmc084().write(|w| unsafe { w.bits(value) });
    }

    fn read_dma_len(&self) -> u32 {
        self.regs().fmc088().read().bits()
    }

    fn write_dma_len(&self, value: u32) {
        self.regs().fmc088().write(|w| unsafe { w.bits(value) });
    }

    fn read_dma_checksum(&self) -> u32 {
        self.regs().fmc090().read().bits()
    }

    fn read_cs0_calib_status(&self) -> u32 {
        self.regs().fmc094().read().bits()
    }

    fn read_cs1_calib_status(&self) -> u32 {
        self.regs().fmc098().read().bits()
    }
}
