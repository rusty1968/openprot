// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Generic SMC controller implementation

use crate::smc::registers::SmcRegisters;
use crate::smc::types::*;

/// Internal controller state
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmcState {
    Uninitialized,
    Ready,
    DmaInFlight,
    #[allow(dead_code)]
    Error,
}

/// Generic Static Memory Controller (SMC)
///
/// Provides unified interface for FMC, SPI1, and SPI2 controllers.
pub struct Smc {
    regs: SmcRegisters,
    controller_id: SmcController,
    config: SmcConfig,
    state: SmcState,
}

impl Smc {
    /// Create a new SMC controller instance
    ///
    /// # Safety
    /// Caller must ensure:
    /// - No other Smc instance exists for this hardware controller
    /// - The controller's base address points to valid hardware
    pub unsafe fn new(config: SmcConfig) -> Result<Self, SmcError> {
        // Validate configuration
        if config.cs0.is_none() && config.cs1.is_none() {
            return Err(SmcError::InvalidCapacity);
        }

        let base = config.controller_id.base_address() as *const _;
        // SAFETY: Caller ensures base address is valid and no other instance exists
        let regs = unsafe { SmcRegisters::new(base) };

        Ok(Self {
            regs,
            controller_id: config.controller_id,
            config,
            state: SmcState::Uninitialized,
        })
    }

    /// Initialize the controller hardware
    ///
    /// Must be called once before any I/O operations.
    pub fn init(&mut self) -> Result<(), SmcError> {
        if self.state != SmcState::Uninitialized {
            return Err(SmcError::HardwareError);
        }

        // 1. Configure flash types and write-enable per CS
        let mut conf = 0u32;
        if self.config.cs0.is_some() {
            conf |= 1 << 16; // CONF_ENABLE_W0
            conf |= 0x2 << 0; // FLASH_TYPE_SPI
        }
        if self.config.cs1.is_some() {
            conf |= 1 << 17; // CONF_ENABLE_W1
            conf |= 0x2 << 2; // FLASH_TYPE_SPI
        }
        self.regs.write_config(conf);

        // 2. Configure timing for each CS
        if let Some(cs_cfg) = self.config.cs0 {
            self.configure_timing(0, &cs_cfg)?;
        }
        if let Some(cs_cfg) = self.config.cs1 {
            self.configure_timing(1, &cs_cfg)?;
        }

        // 3. Set up segment addresses (memory mapping)
        self.setup_segments()?;

        // 4. Enable interrupts if requested
        if self.config.enable_interrupts {
            self.regs.modify_spi_mode(|mode| {
                *mode |= 1 << 3; // DMA_EN
            });
        }

        self.state = SmcState::Ready;
        Ok(())
    }

    /// Configure timing parameters for a chip select
    fn configure_timing(&self, _cs: usize, config: &FlashConfig) -> Result<(), SmcError> {
        // Calculate clock divisor: SYSCLK = 200 MHz
        // TODO: Timing configuration via CS0/CS1 control registers
        let sysclk_mhz = 200u32;
        let _ideal_clk_div = Self::calculate_clock_divisor(sysclk_mhz, config.spi_clock_mhz)?;
        Ok(())
    }

    /// Set up segment registers for memory mapping
    fn setup_segments(&self) -> Result<(), SmcError> {
        let cs0_size = self.config.cs0.map(|c| c.capacity_mb as usize * 1024 * 1024).unwrap_or(0);
        let cs1_size = self.config.cs1.map(|c| c.capacity_mb as usize * 1024 * 1024).unwrap_or(0);

        // Validate no overflow of 256 MB window
        if cs0_size + cs1_size > 256 * 1024 * 1024 {
            return Err(SmcError::InvalidCapacity);
        }

        // Configure CS0 segment
        if cs0_size > 0 {
            let seg = Self::encode_segment(0, cs0_size)?;
            self.regs.write_cs0_segment(seg);
        }

        // Configure CS1 segment (starts after CS0)
        if cs1_size > 0 {
            let seg = Self::encode_segment(cs0_size, cs0_size + cs1_size)?;
            self.regs.write_cs1_segment(seg);
        }

        Ok(())
    }

    /// Encode a memory segment into hardware register format
    ///
    /// Hardware uses 4 KB units for addressing
    fn encode_segment(start: usize, end: usize) -> Result<u32, SmcError> {
        let start_4k = (start >> 12) as u32;
        let end_4k = (end >> 12) as u32;

        // Check for overflow (16-bit fields)
        if start_4k > 0xFFFF || end_4k > 0xFFFF {
            return Err(SmcError::InvalidCapacity);
        }

        Ok((end_4k << 16) | start_4k)
    }

    /// Calculate SPI clock divisor from desired frequency
    fn calculate_clock_divisor(sysclk_mhz: u32, desired_mhz: u32) -> Result<u32, SmcError> {
        if desired_mhz == 0 {
            return Err(SmcError::HardwareError);
        }

        let mut div = 0u32;
        while (sysclk_mhz >> div) > desired_mhz && div < 7 {
            div += 1;
        }
        Ok(div)
    }

    // ====== Public I/O API ======

    /// Perform a programmed I/O read via memory window
    ///
    /// Reads directly from the flash memory window. Hardware automatically
    /// converts memory accesses to SPI transactions.
    pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        if self.state != SmcState::Ready {
            return Err(SmcError::HardwareError);
        }

        let window = self.controller_id.flash_window_address() as *const u8;
        let flash_ptr = unsafe { window.add(offset as usize) };

        // SAFETY: Window address is valid per SmcController definition.
        // Read from flash window (no concurrent writes due to SmcRegisters' !Sync).
        unsafe {
            core::ptr::copy_nonoverlapping(flash_ptr, buf.as_mut_ptr(), buf.len());
        }

        Ok(buf.len())
    }

    /// Initiate a DMA read operation (non-blocking)
    pub fn dma_read(&mut self, flash_offset: u32, dram_addr: usize, len: u32) -> Result<(), SmcError> {
        if self.state != SmcState::Ready {
            return Err(SmcError::HardwareError);
        }

        // Set up DMA registers
        self.regs.write_dma_addr(dram_addr as u32 & 0x000BFFFC); // Apply DRAM mask
        self.regs.write_dma_len(len - 1);

        // Set segment for flash address range
        let seg = Self::encode_segment(
            flash_offset as usize,
            (flash_offset + len) as usize,
        )?;
        self.regs.write_cs0_segment(seg);

        // Trigger DMA
        self.regs.write_dma_ctrl(0x1); // DMA_CTRL_REQUEST

        self.state = SmcState::DmaInFlight;
        Ok(())
    }

    /// Check if controller is ready for operations
    pub fn is_ready(&self) -> bool {
        self.state == SmcState::Ready
    }

    /// Get the controller identifier
    pub fn controller_id(&self) -> SmcController {
        self.controller_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_segment() {
        // Test encoding of 16 MB segment
        let seg = Smc::encode_segment(0, 16 * 1024 * 1024).unwrap();
        let start_4k = seg & 0xFFFF;
        let end_4k = (seg >> 16) & 0xFFFF;
        assert_eq!(start_4k, 0);
        assert_eq!(end_4k, 4096); // 16 MB / 4 KB = 4096
    }

    #[test]
    fn test_clock_divisor() {
        let div = Smc::calculate_clock_divisor(200, 25).unwrap();
        assert_eq!(div, 3); // 200 / 2^3 = 25
    }

    #[test]
    fn test_clock_divisor_high_speed() {
        let div = Smc::calculate_clock_divisor(200, 50).unwrap();
        assert_eq!(div, 2); // 200 / 2^2 = 50
    }

    #[test]
    fn test_segment_overflow() {
        // Test that oversized segments are rejected
        let result = Smc::encode_segment(0, 512 * 1024 * 1024); // 512 MB > 256 MB max
        assert!(result.is_err());
    }
}
