// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! SPI-specific register backend
//!
//! Wraps the SPI PAC register block and implements `SmcRegisterBackend`.
//! Uses `ast1060_pac::spi::RegisterBlock` — distinct from the FMC backend
//! which uses `ast1060_pac::fmc::RegisterBlock`.
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
//!
//! Key semantic differences vs FMC:
//! - DMA interrupt enable/disable uses raw bit manipulation (SPI lacks named
//!   field `dmaintenbl`; FMC exposes it as a structured accessor)
//! - Registers 0x6C and 0x74 are SPI-specific (HostSpi mode); absent in FMC

use ast1060_pac as device;
use core::cell::UnsafeCell;
use core::marker::PhantomData;

use crate::smc::register_traits::SmcRegisterBackend;
use crate::smc::types::ChipSelect;

/// Safe wrapper around SPI hardware registers.
///
/// Implements `SmcRegisterBackend` directly — no redundant concrete methods.
///
/// **No topology logic here.** This backend is pure register transport.
/// Topology-aware behavior (when to use which registers, decode-range sizing,
/// calibration gating) is gated by `SmcConfig.topology` in the controller layer.
pub struct SpiRegisterBackend {
    base: *const device::spi::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>,
}

impl SpiRegisterBackend {
    /// Create a new SPI register backend.
    ///
    /// # Safety
    /// Caller must ensure:
    /// - `base` points to a valid SPI register block
    /// - Only one `SpiRegisterBackend` instance exists per register block
    /// - Caller maintains exclusive access (no concurrent mutations)
    pub const unsafe fn new(base: *const device::spi::RegisterBlock) -> Self {
        Self { base, _not_sync: PhantomData }
    }

    #[inline]
    fn regs(&self) -> &device::spi::RegisterBlock {
        // SAFETY: Constructor ensures pointer validity and exclusive access.
        unsafe { &*self.base }
    }
}

impl SmcRegisterBackend for SpiRegisterBackend {
    fn read_config(&self) -> u32 {
        self.regs().spi000().read().bits()
    }

    fn write_config(&self, value: u32) {
        self.regs().spi000().write(|w| unsafe { w.bits(value) });
    }

    fn modify_config<F>(&self, f: F) where F: FnOnce(&mut u32) {
        self.regs().spi000().modify(|r, w| {
            let mut bits = r.bits();
            f(&mut bits);
            unsafe { w.bits(bits) }
        });
    }

    fn read_addr_width(&self) -> u32 {
        self.regs().spi004().read().bits()
    }

    fn write_addr_width(&self, value: u32) {
        self.regs().spi004().write(|w| unsafe { w.bits(value) });
    }

    // NOTE: SPI uses raw bit access; FMC exposes named fields (e.g. dmaintenbl).
    fn read_dma_status(&self) -> u32 {
        self.regs().spi008().read().bits()
    }

    fn clear_dma_status(&self, clear_mask: u32) {
        self.regs().spi008().write(|w| unsafe { w.bits(clear_mask) });
    }

    fn enable_dma_irq(&self) {
        self.regs().spi008().modify(|r, w| unsafe { w.bits(r.bits() | (1 << 3)) });
    }

    fn disable_dma_irq(&self) {
        self.regs().spi008().modify(|r, w| unsafe { w.bits(r.bits() & !(1 << 3)) });
    }

    fn read_cs0_ctrl(&self) -> u32 {
        self.regs().spi010().read().bits()
    }

    fn write_cs0_ctrl(&self, value: u32) {
        self.regs().spi010().write(|w| unsafe { w.bits(value) });
    }

    fn read_cs1_ctrl(&self) -> u32 {
        self.regs().spi014().read().bits()
    }

    fn write_cs1_ctrl(&self, value: u32) {
        self.regs().spi014().write(|w| unsafe { w.bits(value) });
    }

    fn read_cs_ctrl(&self, cs: ChipSelect) -> u32 {
        match cs {
            ChipSelect::Cs0 => self.regs().spi010().read().bits(),
            ChipSelect::Cs1 => self.regs().spi014().read().bits(),
        }
    }

    fn write_cs_ctrl(&self, cs: ChipSelect, value: u32) {
        match cs {
            ChipSelect::Cs0 => self.regs().spi010().write(|w| unsafe { w.bits(value) }),
            ChipSelect::Cs1 => self.regs().spi014().write(|w| unsafe { w.bits(value) }),
        };
    }

    fn read_cs0_segment(&self) -> u32 {
        self.regs().spi030().read().bits()
    }

    fn write_cs0_segment(&self, value: u32) {
        self.regs().spi030().write(|w| unsafe { w.bits(value) });
    }

    fn read_cs1_segment(&self) -> u32 {
        self.regs().spi034().read().bits()
    }

    fn write_cs1_segment(&self, value: u32) {
        self.regs().spi034().write(|w| unsafe { w.bits(value) });
    }

    // SPI06C: SPI-specific I/O mode register (absent in FMC)
    fn read_spi_mode(&self) -> u32 {
        self.regs().spi06c().read().bits()
    }

    fn write_spi_mode(&self, value: u32) {
        self.regs().spi06c().write(|w| unsafe { w.bits(value) });
    }

    fn modify_spi_mode<F>(&self, f: F) where F: FnOnce(&mut u32) {
        self.regs().spi06c().modify(|r, w| {
            let mut bits = r.bits();
            f(&mut bits);
            unsafe { w.bits(bits) }
        });
    }

    fn read_dma_ctrl(&self) -> u32 {
        self.regs().spi080().read().bits()
    }

    fn write_dma_ctrl(&self, value: u32) {
        self.regs().spi080().write(|w| unsafe { w.bits(value) });
    }

    fn read_dma_addr(&self) -> u32 {
        self.regs().spi084().read().bits()
    }

    fn write_dma_addr(&self, value: u32) {
        self.regs().spi084().write(|w| unsafe { w.bits(value) });
    }

    fn read_dma_len(&self) -> u32 {
        self.regs().spi088().read().bits()
    }

    fn write_dma_len(&self, value: u32) {
        self.regs().spi088().write(|w| unsafe { w.bits(value) });
    }

    fn read_dma_checksum(&self) -> u32 {
        self.regs().spi090().read().bits()
    }

    fn read_cs0_calib_status(&self) -> u32 {
        self.regs().spi094().read().bits()
    }

    fn read_cs1_calib_status(&self) -> u32 {
        self.regs().spi098().read().bits()
    }
}
