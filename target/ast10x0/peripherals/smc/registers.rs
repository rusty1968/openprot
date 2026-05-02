// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Low-level register access
//!
//! Consolidates all unsafe hardware register access into a single unsafe
//! perimeter, following the AST1060 PAC guide pattern.

use ast1060_pac as device;
use core::marker::PhantomData;
use core::cell::UnsafeCell;

/// Safe wrapper around SMC hardware registers
///
/// This struct consolidates all unsafe hardware access. All register operations
/// go through this single point, making it easy to audit safety invariants.
///
/// Register naming follows AST1060 PAC convention: methods are named by their
/// hex offsets (e.g., `fmc000()` for offset 0x00, `fmc080()` for offset 0x80).
pub struct SmcRegisters {
    base: *const device::fmc::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>, // Prevent Sync, allow Send
}

impl SmcRegisters {
    /// Create a new register accessor
    ///
    /// # Safety
    /// Caller must ensure:
    /// - `base` points to a valid FMC register block
    /// - Only one SmcRegisters instance exists per register block
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
        // Only one SmcRegisters instance can be created per hardware controller.
        unsafe { &*self.base }
    }

    // ====== Safe read/write wrappers (FMC register methods by offset) ======

    /// FMC000: Configuration register
    pub fn read_config(&self) -> u32 {
        self.regs().fmc000().read().bits()
    }

    pub fn write_config(&self, value: u32) {
        // SAFETY: Value always valid; PAC ensures register width.
        self.regs().fmc000().write(|w| unsafe { w.bits(value) });
    }

    pub fn modify_config<F>(&self, f: F)
    where
        F: FnOnce(&mut u32),
    {
        self.regs().fmc000().modify(|r, w| {
            let mut bits = r.bits();
            f(&mut bits);
            // SAFETY: Caller's function produces valid bits.
            unsafe { w.bits(bits) }
        });
    }

    /// FMC004: 4-byte mode and address width control
    pub fn read_addr_width(&self) -> u32 {
        self.regs().fmc004().read().bits()
    }

    pub fn write_addr_width(&self, value: u32) {
        self.regs().fmc004().write(|w| unsafe { w.bits(value) });
    }

    /// FMC008: DMA status
    pub fn read_dma_status(&self) -> u32 {
        self.regs().fmc008().read().bits()
    }

    /// FMC008: Clear DMA status bits (write-1-to-clear).
    pub fn clear_dma_status(&self, clear_mask: u32) {
        self.regs().fmc008().write(|w| unsafe { w.bits(clear_mask) });
    }

    /// FMC008: Enable DMA interrupt (bit 3, `dmaintenbl`).
    ///
    /// Call at end of each DMA launch, not at init time.
    pub fn enable_dma_irq(&self) {
        self.regs().fmc008().modify(|_, w| w.dmaintenbl().set_bit());
    }

    /// FMC008: Disable DMA interrupt (bit 3, `dmaintenbl`).
    ///
    /// Call at the top of the IRQ handler before processing status bits.
    pub fn disable_dma_irq(&self) {
        self.regs().fmc008().modify(|_, w| w.dmaintenbl().clear_bit());
    }

    /// FMC010: CS0 control register
    pub fn read_cs0_ctrl(&self) -> u32 {
        self.regs().fmc010().read().bits()
    }

    pub fn write_cs0_ctrl(&self, value: u32) {
        self.regs().fmc010().write(|w| unsafe { w.bits(value) });
    }

    /// FMC014: CS1 control register
    pub fn read_cs1_ctrl(&self) -> u32 {
        self.regs().fmc014().read().bits()
    }

    pub fn write_cs1_ctrl(&self, value: u32) {
        self.regs().fmc014().write(|w| unsafe { w.bits(value) });
    }

    /// Dispatch CS control register read by index.
    pub fn read_cs_ctrl(&self, cs: crate::smc::types::ChipSelect) -> u32 {
        match cs {
            crate::smc::types::ChipSelect::Cs0 => self.read_cs0_ctrl(),
            crate::smc::types::ChipSelect::Cs1 => self.read_cs1_ctrl(),
        }
    }

    /// Dispatch CS control register write by index.
    pub fn write_cs_ctrl(&self, cs: crate::smc::types::ChipSelect, value: u32) {
        match cs {
            crate::smc::types::ChipSelect::Cs0 => self.write_cs0_ctrl(value),
            crate::smc::types::ChipSelect::Cs1 => self.write_cs1_ctrl(value),
        }
    }

    /// FMC030: CS0 segment register (memory mapping)
    pub fn read_cs0_segment(&self) -> u32 {
        self.regs().fmc030().read().bits()
    }

    pub fn write_cs0_segment(&self, value: u32) {
        self.regs().fmc030().write(|w| unsafe { w.bits(value) });
    }

    /// FMC034: CS1 segment register (memory mapping)
    pub fn read_cs1_segment(&self) -> u32 {
        self.regs().fmc034().read().bits()
    }

    pub fn write_cs1_segment(&self, value: u32) {
        self.regs().fmc034().write(|w| unsafe { w.bits(value) });
    }

    /// FMC06C: SPI I/O mode register
    pub fn read_spi_mode(&self) -> u32 {
        self.regs().fmc06c().read().bits()
    }

    pub fn write_spi_mode(&self, value: u32) {
        self.regs().fmc06c().write(|w| unsafe { w.bits(value) });
    }

    pub fn modify_spi_mode<F>(&self, f: F)
    where
        F: FnOnce(&mut u32),
    {
        self.regs().fmc06c().modify(|r, w| {
            let mut bits = r.bits();
            f(&mut bits);
            // SAFETY: Caller's function produces valid bits.
            unsafe { w.bits(bits) }
        });
    }

    /// FMC080: DMA control register
    pub fn read_dma_ctrl(&self) -> u32 {
        self.regs().fmc080().read().bits()
    }

    pub fn write_dma_ctrl(&self, value: u32) {
        self.regs().fmc080().write(|w| unsafe { w.bits(value) });
    }

    /// FMC084: DMA flash address / DRAM address
    pub fn read_dma_addr(&self) -> u32 {
        self.regs().fmc084().read().bits()
    }

    pub fn write_dma_addr(&self, value: u32) {
        self.regs().fmc084().write(|w| unsafe { w.bits(value) });
    }

    /// FMC088: DMA flash window size / DRAM size
    pub fn read_dma_len(&self) -> u32 {
        self.regs().fmc088().read().bits()
    }

    pub fn write_dma_len(&self, value: u32) {
        self.regs().fmc088().write(|w| unsafe { w.bits(value) });
    }

    /// FMC090: DMA checksum (CRC)
    pub fn read_dma_checksum(&self) -> u32 {
        self.regs().fmc090().read().bits()
    }

    /// FMC094: CS0 calibration status
    pub fn read_cs0_calib_status(&self) -> u32 {
        self.regs().fmc094().read().bits()
    }

    /// FMC098: CS1 calibration status
    pub fn read_cs1_calib_status(&self) -> u32 {
        self.regs().fmc098().read().bits()
    }
}
