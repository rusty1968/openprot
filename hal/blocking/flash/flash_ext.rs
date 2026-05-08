// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Flash HAL extensibility traits and types.
//!
//! This module provides semantic parity with Zephyr's flash subsystem for:
//! - Vendor-specific extended operations (`FlashExOp`)
//! - Erased-byte value discovery (`FlashErasedByte`)
//! - Page layout introspection (`FlashPageLayout`)
//! - JESD216 / SFDP capability (`FlashJesd216`)
//! - Partition / region abstraction (`FlashRegion`, `flash_flatten`)

#![cfg_attr(not(test), no_std)]

use core::num::NonZero;
use util_error::ErrorCode;
use util_types::PowerOf2Usize;

use crate::{Flash, FlashAddress};

// ============================================================================
// Gap 1: Vendor Escape Hatch
// ============================================================================

/// Optional capability: vendor-specific extended operations.
///
/// Implement this in addition to `Flash` for devices that support
/// vendor-defined operations beyond standard read/write/erase.
/// Generic code that requires extended operations should add
/// `where F: Flash + FlashExOp`.
///
/// # Example
///
/// ```ignore
/// fn vendor_operation<F: Flash + FlashExOp>(
///     flash: &mut F,
///     code: u16,
///     params: &[u8],
/// ) -> Result<(), ErrorCode> {
///     let mut output = [0u8; 64];
///     flash.ex_op(code, params, &mut output)
/// }
/// ```
pub trait FlashExOp: Flash {
    /// Perform a vendor-specific or platform-specific extended operation.
    ///
    /// # Arguments
    /// - `code`: Operation identifier (driver-defined)
    /// - `input`: Operation parameters
    /// - `output`: Result buffer
    ///
    /// # Returns
    /// - `Ok(())` if the operation succeeded
    /// - `Err(ErrorCode::NotSupported)` if `code` is not implemented
    /// - Other `ErrorCode` variants for driver-specific errors
    fn ex_op(&mut self, code: u16, input: &[u8], output: &mut [u8]) -> Result<(), ErrorCode>;
}

// ============================================================================
// Gap 2: Erased Byte Value
// ============================================================================

/// Optional capability: erased-byte value discovery.
///
/// Implement this in addition to `Flash` for devices that support
/// querying the byte value that cells contain after a successful erase.
/// Generic code that needs blank-check or erase-skip logic should add
/// `where F: Flash + FlashErasedByte`.
///
/// # Example
///
/// ```ignore
/// fn blank_check<F: Flash + FlashErasedByte>(
///     flash: &mut F,
///     offset: usize,
///     len: usize,
/// ) -> Result<bool, ErrorCode> {
///     let erased_val = flash.erased_byte();
///     let mut buf = [0u8; 256];
///     let mut pos = 0;
///     while pos < len {
///         let chunk = core::cmp::min(buf.len(), len - pos);
///         flash.read(FlashAddress::data((offset + pos) as u32), &mut buf[..chunk])?;
///         if buf[..chunk].iter().any(|&b| b != erased_val) {
///             return Ok(false);
///         }
///         pos += chunk;
///     }
///     Ok(true)
/// }
/// ```
pub trait FlashErasedByte: Flash {
    /// The byte value that all storage cells contain immediately after a
    /// successful erase operation.
    ///
    /// - NOR flash typically returns `0xFF` (default).
    /// - Some MRAM, FeRAM, or emulated-flash devices return `0x00`.
    /// - Generic code MUST use this method rather than hardcoding `0xFF`.
    fn erased_byte(&self) -> u8;
}

// ============================================================================
// Gap 3: Page Layout Capability
// ============================================================================

/// Geometry information for a single erase page.
#[derive(Clone, Copy, Debug)]
pub struct FlashPageInfo {
    /// Byte offset of the first byte of this page, relative to the
    /// start of the device.
    pub start_offset: usize,
    /// Size of this page in bytes.
    pub size: usize,
    /// Zero-based page index.
    pub index: usize,
}

/// Optional capability: page layout introspection.
///
/// Implement this in addition to `Flash` for devices that can report
/// their internal page geometry. Generic code that requires page-level
/// introspection should add `where F: Flash + FlashPageLayout`.
///
/// # Example
///
/// ```ignore
/// fn page_traverse<F: Flash + FlashPageLayout>(flash: &F) {
///     for idx in 0..flash.page_count() {
///         if let Some(info) = flash.page_info_by_index(idx) {
///             println!("Page {}: offset=0x{:x}, size={}", idx, info.start_offset, info.size);
///         }
///     }
/// }
/// ```
pub trait FlashPageLayout: Flash {
    /// Returns the total number of erase pages on the device.
    fn page_count(&self) -> usize;

    /// Returns geometry information for the page that contains `offset`.
    ///
    /// Returns `None` if `offset` is out of range.
    fn page_info_by_offset(&self, offset: usize) -> Option<FlashPageInfo>;

    /// Returns geometry information for the page at position `index`
    /// (0-based, in address order).
    ///
    /// Returns `None` if `index >= page_count()`.
    fn page_info_by_index(&self, index: usize) -> Option<FlashPageInfo>;
}

// ============================================================================
// Gap 4: JESD216 / SFDP Capability
// ============================================================================

/// Optional capability: JESD216 / SFDP introspection.
///
/// Implement this in addition to `Flash` for SPI NOR devices that
/// expose standard JEDEC IDs and SFDP parameter tables.
/// Generic code that requires JESD216 capability should add
/// `where F: Flash + FlashJesd216`.
///
/// # Example
///
/// ```ignore
/// fn read_device_id<F: Flash + FlashJesd216>(flash: &mut F) -> Result<(), ErrorCode> {
///     let id = flash.read_jedec_id()?;
///     println!("Manufacturer: 0x{:02x}, Type: 0x{:02x}, Capacity: 0x{:02x}", id[0], id[1], id[2]);
///     Ok(())
/// }
/// ```
pub trait FlashJesd216: Flash {
    /// Read the 3-byte JEDEC ID (manufacturer, memory type, capacity).
    fn read_jedec_id(&mut self) -> Result<[u8; 3], ErrorCode>;

    /// Read `len` bytes from the SFDP address space at `offset`.
    fn sfdp_read(&mut self, offset: usize, buf: &mut [u8]) -> Result<(), ErrorCode>;
}

// ============================================================================
// Gap 5: Partition / Region Layer
// ============================================================================

/// A view into a contiguous sub-range of a `Flash` device.
///
/// All addresses passed to the inner `Flash` are rebased to
/// `base + caller_offset`, and all accesses are bounds-checked
/// against `size` before the underlying driver is called.
///
/// This is the static, type-safe analogue of Zephyr's `flash_area`.
/// Unlike `flash_area`, there is no integer-ID registry; instead,
/// callers hold a `FlashRegion` value that embeds the device reference
/// and bounds, eliminating the `flash_area_open` / `flash_area_close`
/// lifecycle.
///
/// # Example
///
/// ```ignore
/// let inner_flash = BlockingFlash::new(driver, blocking);
/// let region = FlashRegion::new(inner_flash, 0x8000, 0x4000);
///
/// // All reads/writes are bounds-checked and rebased
/// region.read(FlashAddress::data(0x100), &mut buf)?;  // Reads from inner address 0x8100
/// ```
pub struct FlashRegion<F: Flash> {
    flash: F,
    base: usize,
    size: usize,
}

impl<F: Flash> FlashRegion<F> {
    /// Create a new bounded region of flash.
    ///
    /// # Arguments
    /// - `flash`: The underlying flash device
    /// - `base`: Base offset within the flash device
    /// - `size`: Size of the region in bytes
    pub fn new(flash: F, base: usize, size: usize) -> Self {
        Self { flash, base, size }
    }

    /// Consume this region and return the underlying flash device.
    pub fn into_inner(self) -> F {
        self.flash
    }

    /// Get a reference to the underlying flash device.
    pub fn inner(&self) -> &F {
        &self.flash
    }

    /// Get a mutable reference to the underlying flash device.
    pub fn inner_mut(&mut self) -> &mut F {
        &mut self.flash
    }
}

impl<F: Flash> Flash for FlashRegion<F> {
    fn page_size(&self) -> PowerOf2Usize {
        self.flash.page_size()
    }

    fn size(&self) -> NonZero<usize> {
        NonZero::new(self.size).unwrap_or_else(|| NonZero::new(1).unwrap())
    }

    fn read(&mut self, start_addr: FlashAddress, buf: &mut [u8]) -> Result<(), ErrorCode> {
        let offset = start_addr.offset() as usize;
        // Bounds check before delegating
        if offset.checked_add(buf.len()).map_or(true, |end| end > self.size) {
            return Err(ErrorCode::InvalidArgument);
        }
        self.flash.read(FlashAddress::data((self.base + offset) as u32), buf)
    }

    fn erase_page(&mut self, start_addr: FlashAddress) -> Result<(), ErrorCode> {
        let offset = start_addr.offset() as usize;
        if offset >= self.size {
            return Err(ErrorCode::InvalidArgument);
        }
        self.flash
            .erase_page(FlashAddress::data((self.base + offset) as u32))
    }

    fn program(&mut self, start_addr: FlashAddress, data: &[u8]) -> Result<(), ErrorCode> {
        let offset = start_addr.offset() as usize;
        if offset.checked_add(data.len()).map_or(true, |end| end > self.size) {
            return Err(ErrorCode::InvalidArgument);
        }
        self.flash
            .program(FlashAddress::data((self.base + offset) as u32), data)
    }
}

// Forward optional trait implementations to the inner flash
impl<F: Flash + FlashExOp> FlashExOp for FlashRegion<F> {
    fn ex_op(&mut self, code: u16, input: &[u8], output: &mut [u8]) -> Result<(), ErrorCode> {
        self.flash.ex_op(code, input, output)
    }
}

impl<F: Flash + FlashErasedByte> FlashErasedByte for FlashRegion<F> {
    fn erased_byte(&self) -> u8 {
        self.flash.erased_byte()
    }
}

impl<F: Flash + FlashPageLayout> FlashPageLayout for FlashRegion<F> {
    fn page_count(&self) -> usize {
        self.flash.page_count()
    }

    fn page_info_by_offset(&self, offset: usize) -> Option<FlashPageInfo> {
        self.flash.page_info_by_offset(self.base + offset)
    }

    fn page_info_by_index(&self, index: usize) -> Option<FlashPageInfo> {
        self.flash.page_info_by_index(index)
    }
}

impl<F: Flash + FlashJesd216> FlashJesd216 for FlashRegion<F> {
    fn read_jedec_id(&mut self) -> Result<[u8; 3], ErrorCode> {
        self.flash.read_jedec_id()
    }

    fn sfdp_read(&mut self, offset: usize, buf: &mut [u8]) -> Result<(), ErrorCode> {
        self.flash.sfdp_read(offset, buf)
    }
}

/// Erase or zero-fill a region of flash.
///
/// If `flash.erased_byte() == 0xFF`, performs erase operations (NOR model).
/// Otherwise, programs zeros over the region (MRAM / emulated model).
/// This is the analogue of Zephyr's `flash_area_flatten`.
///
/// # Arguments
/// - `flash`: The flash device to flatten (must implement `FlashErasedByte`)
/// - `offset`: Starting offset within the device
/// - `len`: Number of bytes to flatten
///
/// # Errors
/// Returns errors from the underlying `flash.erase_page()` or `flash.program()` calls.
///
/// # Example
///
/// ```ignore
/// let mut region = FlashRegion::new(inner_flash, 0x8000, 0x4000);
/// flash_flatten(&mut region, 0x100, 0x1000)?;  // Flatten a 4KB region
/// ```
pub fn flash_flatten<F: Flash + FlashErasedByte>(flash: &mut F, offset: usize, len: usize) -> Result<(), ErrorCode> {
    if flash.erased_byte() == 0xFF {
        // NOR model: erase pages
        let page_size = flash.page_size().get();
        let mut addr = (offset / page_size) * page_size;
        let end = offset + len;
        while addr < end {
            flash.erase_page(FlashAddress::data(addr as u32))?;
            addr += page_size;
        }
    } else {
        // MRAM / emulated model: write zeros
        let mut buf = [0u8; 256];
        let mut written = 0;
        while written < len {
            let chunk = core::cmp::min(buf.len(), len - written);
            flash.program(FlashAddress::data((offset + written) as u32), &buf[..chunk])?;
            written += chunk;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZero;
    use std::vec::Vec;

    #[derive(Clone)]
    struct MemoryFlash {
        data: Vec<u8>,
        page_size: usize,
        erased_byte: u8,
        erase_count: usize,
        program_count: usize,
    }

    impl MemoryFlash {
        fn new(size: usize, page_size: usize, erased_byte: u8, fill: u8) -> Self {
            Self {
                data: vec![fill; size],
                page_size,
                erased_byte,
                erase_count: 0,
                program_count: 0,
            }
        }
    }

    impl Flash for MemoryFlash {
        fn page_size(&self) -> PowerOf2Usize {
            PowerOf2Usize::new(self.page_size).unwrap()
        }

        fn size(&self) -> NonZero<usize> {
            NonZero::new(self.data.len()).unwrap()
        }

        fn read(&mut self, start_addr: FlashAddress, buf: &mut [u8]) -> Result<(), ErrorCode> {
            let start = start_addr.offset() as usize;
            let end = start
                .checked_add(buf.len())
                .ok_or(ErrorCode::InvalidArgument)?;
            if end > self.data.len() {
                return Err(ErrorCode::InvalidArgument);
            }
            buf.copy_from_slice(&self.data[start..end]);
            Ok(())
        }

        fn erase_page(&mut self, start_addr: FlashAddress) -> Result<(), ErrorCode> {
            let start = start_addr.offset() as usize;
            if start % self.page_size != 0 {
                return Err(ErrorCode::InvalidArgument);
            }
            let end = start
                .checked_add(self.page_size)
                .ok_or(ErrorCode::InvalidArgument)?;
            if end > self.data.len() {
                return Err(ErrorCode::InvalidArgument);
            }
            self.data[start..end].fill(self.erased_byte);
            self.erase_count += 1;
            Ok(())
        }

        fn program(&mut self, start_addr: FlashAddress, data: &[u8]) -> Result<(), ErrorCode> {
            let start = start_addr.offset() as usize;
            let end = start
                .checked_add(data.len())
                .ok_or(ErrorCode::InvalidArgument)?;
            if end > self.data.len() {
                return Err(ErrorCode::InvalidArgument);
            }
            self.data[start..end].copy_from_slice(data);
            self.program_count += 1;
            Ok(())
        }
    }

    impl FlashErasedByte for MemoryFlash {
        fn erased_byte(&self) -> u8 {
            self.erased_byte
        }
    }

    #[test]
    fn flash_page_info_basic() {
        let info = FlashPageInfo {
            start_offset: 0x1000,
            size: 0x1000,
            index: 1,
        };
        assert_eq!(info.start_offset, 0x1000);
        assert_eq!(info.size, 0x1000);
        assert_eq!(info.index, 1);
    }

    #[test]
    fn region_rebases_and_checks_bounds() {
        let inner = MemoryFlash::new(128, 16, 0xff, 0xaa);
        let mut region = FlashRegion::new(inner, 32, 32);

        region
            .program(FlashAddress::data(4), &[1, 2, 3, 4])
            .unwrap();

        let mut out = [0u8; 4];
        region.read(FlashAddress::data(4), &mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4]);

        let err = region.read(FlashAddress::data(30), &mut out).unwrap_err();
        assert_eq!(err, ErrorCode::InvalidArgument);
    }

    #[test]
    fn flatten_uses_erase_path_for_ff() {
        let mut flash = MemoryFlash::new(64, 16, 0xff, 0x11);

        flash_flatten(&mut flash, 16, 16).unwrap();

        assert_eq!(flash.erase_count, 1);
        assert_eq!(flash.program_count, 0);
        assert!(flash.data[16..32].iter().all(|&b| b == 0xff));
    }

    #[test]
    fn flatten_uses_program_path_for_non_ff() {
        let mut flash = MemoryFlash::new(64, 16, 0x00, 0xaa);

        flash_flatten(&mut flash, 8, 10).unwrap();

        assert_eq!(flash.erase_count, 0);
        assert!(flash.program_count > 0);
        assert!(flash.data[8..18].iter().all(|&b| b == 0x00));
    }
}
