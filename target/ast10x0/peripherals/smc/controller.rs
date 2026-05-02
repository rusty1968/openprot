// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Generic SMC controller implementation

use core::marker::PhantomData;

use crate::smc::helpers::{
    SPI_CTRL_FREQ_MASK, encode_segment, flash_capacity_bytes, spi_freq_div, total_capacity_bytes,
    validate_dma_read, validate_mapped_range,
};
use crate::smc::registers::SmcRegisters;
use crate::smc::types::*;

/// Internal controller state
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmcState {
    Ready,
    DmaInFlight,
    #[allow(dead_code)]
    Error,
}

/// Type-state marker: controller is constructed but not initialized.
pub struct Uninitialized;

/// Type-state marker: controller has completed hardware initialization.
pub struct Ready;

/// Generic Static Memory Controller (SMC)
///
/// The `Mode` type parameter enforces init ordering at compile time.
pub struct Smc<Mode> {
    regs: SmcRegisters,
    controller_id: SmcController,
    config: SmcConfig,
    state: SmcState,
    _mode: PhantomData<fn() -> Mode>,
}

/// Ergonomic alias for the uninitialized controller handle.
pub type UninitSmc = Smc<Uninitialized>;

/// Ergonomic alias for the initialized controller handle.
pub type ReadySmc = Smc<Ready>;

impl Smc<Uninitialized> {
    /// Create a new SMC controller instance.
    ///
    /// # Safety
    /// Caller must ensure:
    /// - No other Smc instance exists for this hardware controller
    /// - The controller's base address points to valid hardware
    pub unsafe fn new(config: SmcConfig) -> Result<Self, SmcError> {
        if config.cs0.is_none() && config.cs1.is_none() {
            return Err(SmcError::InvalidCapacity);
        }

        let base = config.controller_id.base_address() as *const _;
        // SAFETY: Caller ensures base address is valid and no other instance exists.
        let regs = unsafe { SmcRegisters::new(base) };

        Ok(Self {
            regs,
            controller_id: config.controller_id,
            config,
            state: SmcState::Ready,
            _mode: PhantomData,
        })
    }

    /// Initialize hardware and transition to `Ready` mode.
    pub fn init(self) -> Result<Smc<Ready>, SmcError> {
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
            Self::configure_timing(&self, 0, &cs_cfg)?;
        }
        if let Some(cs_cfg) = self.config.cs1 {
            Self::configure_timing(&self, 1, &cs_cfg)?;
        }

        // 3. Set up segment addresses (memory mapping)
        Self::setup_segments(&self)?;

        // 4. Enable interrupts if requested
        if self.config.enable_interrupts {
            self.regs.modify_spi_mode(|mode| {
                *mode |= 1 << 3; // DMA_EN
            });
        }

        Ok(Smc {
            regs: self.regs,
            controller_id: self.controller_id,
            config: self.config,
            state: SmcState::Ready,
            _mode: PhantomData,
        })
    }

    fn configure_timing(&self, cs: usize, config: &FlashConfig) -> Result<(), SmcError> {
        let sysclk_mhz = 200u32;
        let encoded_div = spi_freq_div(sysclk_mhz, config.spi_clock_mhz)?;

        match cs {
            0 => {
                let reg = self.regs.read_cs0_ctrl();
                self.regs
                    .write_cs0_ctrl((reg & !SPI_CTRL_FREQ_MASK) | encoded_div);
            }
            1 => {
                let reg = self.regs.read_cs1_ctrl();
                self.regs
                    .write_cs1_ctrl((reg & !SPI_CTRL_FREQ_MASK) | encoded_div);
            }
            _ => return Err(SmcError::HardwareError),
        }

        Ok(())
    }

    fn setup_segments(&self) -> Result<(), SmcError> {
        let cs0_size = flash_capacity_bytes(self.config.cs0)?;
        let cs1_size = flash_capacity_bytes(self.config.cs1)?;
        total_capacity_bytes(self.config.cs0, self.config.cs1)?;

        if cs0_size > 0 {
            let seg = encode_segment(0, cs0_size)?;
            self.regs.write_cs0_segment(seg);
        }

        if cs1_size > 0 {
            let seg = encode_segment(cs0_size, cs0_size + cs1_size)?;
            self.regs.write_cs1_segment(seg);
        }

        Ok(())
    }
}

impl Smc<Ready> {
    /// Perform a programmed I/O read via memory window.
    ///
    /// Reads directly from the flash memory window. Hardware automatically
    /// converts memory accesses to SPI transactions.
    pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        let capacity_bytes = total_capacity_bytes(self.config.cs0, self.config.cs1)?;
        let window = self.controller_id.flash_window_address() as *const u8;
        let offset = validate_mapped_range(offset, buf.len(), capacity_bytes)?;
        let flash_ptr = window.wrapping_add(offset);

        // SAFETY: `flash_ptr` is derived from the controller's fixed MMIO flash
        // window using `wrapping_add`, which avoids imposing Rust allocation
        // provenance rules on the raw address arithmetic itself. The actual read
        // below requires the requested `[offset, offset + buf.len())` range to be
        // backed by the controller's mapped flash aperture, and `buf` provides a
        // valid, writable destination that does not overlap this MMIO window.
        unsafe {
            core::ptr::copy_nonoverlapping(flash_ptr, buf.as_mut_ptr(), buf.len());
        }

        Ok(buf.len())
    }

    /// Initiate a DMA read operation (non-blocking).
    pub fn dma_read(&mut self, flash_offset: u32, dram_addr: usize, len: u32) -> Result<(), SmcError> {
        if self.state != SmcState::Ready {
            return Err(SmcError::HardwareError);
        }

        let capacity_bytes = total_capacity_bytes(self.config.cs0, self.config.cs1)?;
        let validated = validate_dma_read(flash_offset, dram_addr, len, capacity_bytes)?;

        self.regs.write_dma_addr(validated.dram_addr);
        self.regs.write_dma_len(validated.dma_len_reg);

        let seg = encode_segment(validated.flash_start, validated.flash_end)?;
        self.regs.write_cs0_segment(seg);

        self.regs.write_dma_ctrl(0x1); // DMA_CTRL_REQUEST

        self.state = SmcState::DmaInFlight;
        Ok(())
    }

    /// Check if controller is ready for operations.
    pub fn is_ready(&self) -> bool {
        self.state == SmcState::Ready
    }

    /// Get the controller identifier.
    pub fn controller_id(&self) -> SmcController {
        self.controller_id
    }
}
