// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Shared register backend trait
//!
//! Abstracts register access operations used by the generic controller,
//! allowing different concrete backends (FMC, SPI) to implement register
//! interpretation according to their hardware semantics.
//!
//! This trait enforces a contract at compile time: only operations explicitly
//! listed here are part of the shared controller's public API. Backend-specific
//! registers and operations are NOT exposed through this trait and must be
//! accessed directly on the backend implementation.
//!
//! # Phase 5: Topology Logic Boundary
//!
//! **This trait is pure register abstraction. No topology logic belongs here.**
//! 
//! All decisions about when to call these operations, how to interpret results
//! based on controller role, and topology-gated behaviors (decode-range sizing,
//! calibration skip, control register programming per role) live in the
//! controller layer (`controller.rs`), not in backends or this trait.
//!
//! Backends are transport only; they answer "how to read/write hardware registers."
//! The controller layer answers "what to do with the data based on the topology."

use crate::smc::types::ChipSelect;

/// Shared register backend trait
///
/// Provides safe access to SMC register operations that are truly shared
/// between FMC and SPI backends. Register interpretation may differ by
/// controller (e.g., FMC uses named field accessors while SPI uses raw bits),
/// but the semantic behavior must be equivalent for shared operations.
///
/// # Semantic Note
///
/// Although FMC and SPI may use the same register offsets, the PAC generates
/// different struct field layouts for each controller. Implementations of this
/// trait must respect the semantic interpretation of their hardware.
/// Misuse (e.g., calling FMC-specific field methods on SPI data) is a
/// compile-time type error, not a runtime bug.
pub trait SmcRegisterBackend: Sized {
    /// FMC000 / SPI000: Configuration register
    fn read_config(&self) -> u32;
    fn write_config(&self, value: u32);
    fn modify_config<F>(&self, f: F)
    where
        F: FnOnce(&mut u32);

    /// FMC004 / SPI004: 4-byte mode and address width control
    fn read_addr_width(&self) -> u32;
    fn write_addr_width(&self, value: u32);

    /// FMC008 / SPI008: DMA status
    fn read_dma_status(&self) -> u32;
    fn clear_dma_status(&self, clear_mask: u32);
    fn enable_dma_irq(&self);
    fn disable_dma_irq(&self);

    /// FMC010 / SPI010: CS0 control register
    fn read_cs0_ctrl(&self) -> u32;
    fn write_cs0_ctrl(&self, value: u32);

    /// FMC014 / SPI014: CS1 control register
    fn read_cs1_ctrl(&self) -> u32;
    fn write_cs1_ctrl(&self, value: u32);

    /// Dispatch CS control register read by index
    fn read_cs_ctrl(&self, cs: ChipSelect) -> u32 {
        match cs {
            ChipSelect::Cs0 => self.read_cs0_ctrl(),
            ChipSelect::Cs1 => self.read_cs1_ctrl(),
        }
    }

    /// Dispatch CS control register write by index
    fn write_cs_ctrl(&self, cs: ChipSelect, value: u32) {
        match cs {
            ChipSelect::Cs0 => self.write_cs0_ctrl(value),
            ChipSelect::Cs1 => self.write_cs1_ctrl(value),
        }
    }

    /// FMC030 / SPI030: CS0 segment register (memory mapping)
    fn read_cs0_segment(&self) -> u32;
    fn write_cs0_segment(&self, value: u32);

    /// FMC034 / SPI034: CS1 segment register (memory mapping)
    fn read_cs1_segment(&self) -> u32;
    fn write_cs1_segment(&self, value: u32);

    /// FMC06C / SPI06C: SPI I/O mode register
    /// Note: SPI-specific semantics (not used by FMC)
    fn read_spi_mode(&self) -> u32;
    fn write_spi_mode(&self, value: u32);
    fn modify_spi_mode<F>(&self, f: F)
    where
        F: FnOnce(&mut u32);

    /// FMC080 / SPI080: DMA control register
    fn read_dma_ctrl(&self) -> u32;
    fn write_dma_ctrl(&self, value: u32);

    /// FMC084 / SPI084: DMA flash address / DRAM address
    fn read_dma_addr(&self) -> u32;
    fn write_dma_addr(&self, value: u32);

    /// FMC088 / SPI088: DMA flash window size / DRAM size
    fn read_dma_len(&self) -> u32;
    fn write_dma_len(&self, value: u32);

    /// FMC090 / SPI090: DMA checksum (CRC)
    fn read_dma_checksum(&self) -> u32;

    /// FMC094 / SPI094: CS0 calibration status
    fn read_cs0_calib_status(&self) -> u32;

    /// FMC098 / SPI098: CS1 calibration status
    fn read_cs1_calib_status(&self) -> u32;
}
