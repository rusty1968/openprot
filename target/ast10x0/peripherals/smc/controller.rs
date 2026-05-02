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

const ASPEED_SPI_USER: u32 = 0x3;
const ASPEED_SPI_USER_INACTIVE: u32 = 0x4;
/// Mask for bits that are not IO mode or mode-type fields — preserves
/// frequency divisor and other config bits across per-phase ctrl writes.
const SPI_CTRL_IO_MODE_MASK: u32 = !0x7000_0000;

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
    /// Per-CS normal-read control register values stored at init time.
    /// Indexed by `ChipSelect as usize`. Restored unconditionally after every
    /// user-mode transaction, matching aspeed-rust `deactivate_user()` behavior.
    normal_read_ctrl: [u32; 2],
    /// Per-CS AHB flash window base addresses.
    /// CS0 starts at `controller_id.flash_window_address()`;
    /// CS1 starts immediately after the CS0 segment.
    flash_window_base: [usize; 2],
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
            normal_read_ctrl: [0; 2],
            flash_window_base: [0; 2],
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

        // Snapshot per-CS normal-read control register values after all init writes.
        // CS1 value is captured even if cs1 is None (safe: register read is harmless).
        let cs0_normal_read = self.regs.read_cs0_ctrl();
        let cs1_normal_read = self.regs.read_cs1_ctrl();

        // Compute per-CS AHB flash window base addresses.
        let base = self.controller_id.flash_window_address();
        let cs0_size = flash_capacity_bytes(self.config.cs0).unwrap_or(0);
        let flash_window_base = [base, base + cs0_size];

        Ok(Smc {
            regs: self.regs,
            controller_id: self.controller_id,
            config: self.config,
            state: SmcState::Ready,
            normal_read_ctrl: [cs0_normal_read, cs1_normal_read],
            flash_window_base,
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
            return Err(SmcError::ControllerNotReady);
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

    /// Return configured total flash capacity for this controller in bytes.
    pub fn capacity_bytes(&self) -> Result<usize, SmcError> {
        total_capacity_bytes(self.config.cs0, self.config.cs1)
    }

    /// Execute a raw user-mode SPI transfer on CS0 for this controller.
    ///
    /// The `mode` parameter controls the IO width written to the CS control
    /// register for each phase (cmd / addr+payload / rx), matching the
    /// per-phase register update pattern used by aspeed-rust's
    /// `spi_nor_transceive_user()`.
    pub fn transceive_user(
        &self,
        cs: ChipSelect,
        cmd: &[u8],
        tx_payload: &[u8],
        rx: &mut [u8],
        mode: TransferMode,
    ) -> Result<(), SmcError> {
        if self.state != SmcState::Ready {
            return Err(SmcError::ControllerNotReady);
        }
        if cs == ChipSelect::Cs1 && self.config.cs1.is_none() {
            return Err(SmcError::InvalidChipSelect);
        }

        let cs_idx = cs as usize;
        // Derive user-mode base from the stored normal-read value: preserve
        // frequency bits and replace mode type with ASPEED_SPI_USER.
        let user_base = (self.normal_read_ctrl[cs_idx] & SPI_CTRL_FREQ_MASK) | ASPEED_SPI_USER;
        let window = self.flash_window_base[cs_idx] as *mut u32;

        // Assert CS: inactive first, then active (matches aspeed-rust activate_user).
        self.regs.write_cs_ctrl(cs, user_base | ASPEED_SPI_USER_INACTIVE);
        self.regs.write_cs_ctrl(cs, user_base);

        // SAFETY: user mode is active; the flash aperture is the hardware-defined
        // byte-stream port for SPI command traffic while user mode is held.
        unsafe {
            // Command phase — always single-wire.
            let cmd_ctrl = (user_base & SPI_CTRL_IO_MODE_MASK) | mode.cmd_io_bits();
            self.regs.write_cs_ctrl(cs, cmd_ctrl);
            spi_write_data(window, cmd);

            // Address / TX payload phase.
            let addr_ctrl = (user_base & SPI_CTRL_IO_MODE_MASK) | mode.addr_io_bits();
            self.regs.write_cs_ctrl(cs, addr_ctrl);
            spi_write_data(window, tx_payload);

            // RX data phase.
            let data_ctrl = (user_base & SPI_CTRL_IO_MODE_MASK) | mode.data_io_bits();
            self.regs.write_cs_ctrl(cs, data_ctrl);
            spi_read_data(window as *const u32, rx);
        }

        // Deassert CS, then restore the pre-computed normal-read configuration
        // (matches aspeed-rust deactivate_user restoring cmd_mode[cs].normal_read).
        self.regs.write_cs_ctrl(cs, user_base | ASPEED_SPI_USER_INACTIVE);
        self.regs.write_cs_ctrl(cs, self.normal_read_ctrl[cs_idx]);
        Ok(())
    }
}

unsafe fn spi_read_data(ahb_addr: *const u32, read_arr: &mut [u8]) {
    let len = read_arr.len();
    let mut index = 0usize;

    while index + 4 <= len {
        let word = unsafe { core::ptr::read_volatile(ahb_addr.add(index / 4)) };
        read_arr[index..index + 4].copy_from_slice(&word.to_le_bytes());
        index += 4;
    }

    while index < len {
        read_arr[index] = unsafe { core::ptr::read_volatile(ahb_addr.cast::<u8>().add(index)) };
        index += 1;
    }
}

unsafe fn spi_write_data(ahb_addr: *mut u32, write_arr: &[u8]) {
    let len = write_arr.len();
    let mut index = 0usize;

    while index + 4 <= len {
        let word = u32::from_le_bytes([
            write_arr[index],
            write_arr[index + 1],
            write_arr[index + 2],
            write_arr[index + 3],
        ]);
        unsafe { core::ptr::write_volatile(ahb_addr.add(index / 4), word) };
        index += 4;
    }

    while index < len {
        unsafe { core::ptr::write_volatile(ahb_addr.cast::<u8>().add(index), write_arr[index]) };
        index += 1;
    }
}
