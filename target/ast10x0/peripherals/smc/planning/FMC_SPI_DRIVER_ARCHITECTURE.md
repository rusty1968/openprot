# FMC/SPI Driver Architecture Recommendation

## Executive Summary

**Layered, modular design** with clear separation of concerns across three layers:

1. **HAL Register Layer** — Thin wrapper around hardware registers (unsafe, consolidated)
2. **Controller Layer** — Per-controller state management (FMC, SPI1, SPI2)
3. **Device Layer** — Flash device abstractions and operations (PIO vs DMA, read vs write)

This guide recommends **starting minimal (layer 1 + layer 2) and evolving into layer 3** as needed.

---

## 1. Recommended Directory Structure

```
src/
├── smc/
│   ├── mod.rs                      # Top-level driver exports
│   ├── registers.rs                # Register definitions & safe accessors
│   ├── types.rs                    # Error enums, configuration types
│   ├── helpers.rs                  # Pure logic: validation, encoding, timing
│   ├── controller.rs               # Generic SMC controller (base class)
│   ├── fmc.rs                      # FMC-specific implementation
│   ├── spi.rs                      # SPI1/SPI2 shared implementation
│   ├── device/
│   │   ├── mod.rs                  # Flash device trait
│   │   ├── flash.rs                # SPI NOR flash standard operations
│   │   └── config.rs               # Device detection & configuration
│   ├── interrupts.rs               # DMA/error IRQ handlers
│   ├── dma.rs                      # DMA operation abstractions
│   └── tests/
│       ├── unit_tests.rs           # Register access tests
│       ├── integration_tests.rs    # Multi-controller tests
│       └── fixtures.rs             # Test data & mocks
```

---

## 2. Module Responsibilities

### 2.1 `registers.rs` — Single Unsafe Perimeter

**Purpose**: Consolidate ALL hardware register access in one place.

**Key Principle**: **One inline method per register block**, following AST1060 PAC guide.

```rust
use core::marker::PhantomData;
use core::cell::UnsafeCell;

/// Low-level register access for a single SMC controller instance
pub struct SmcRegisters {
    base: *const ast1030_pac::smc::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>, // Prevent Sync
}

impl SmcRegisters {
    /// # Safety
    /// Caller must ensure:
    /// - `base` points to a valid SMC register block
    /// - Only one SmcRegisters instance exists per register block
    /// - Caller maintains exclusive access (no concurrent mutations)
    pub const unsafe fn new(base: *const ast1030_pac::smc::RegisterBlock) -> Self {
        Self {
            base,
            _not_sync: PhantomData,
        }
    }

    /// Consolidated register access: all unsafe code in one place
    #[inline]
    fn regs(&self) -> &ast1030_pac::smc::RegisterBlock {
        // SAFETY: Constructor ensures pointer validity and single ownership.
        unsafe { &*self.base }
    }

    // Per-register safe wrappers (no more unsafe):
    
    pub fn read_conf(&self) -> u32 {
        self.regs().conf().read().bits()
    }

    pub fn write_conf(&self, value: u32) {
        // SAFETY: Value comes from HAL configuration; always valid.
        self.regs().conf().write(|w| unsafe { w.bits(value) })
    }

    pub fn modify_conf<F>(&self, f: F)
    where
        F: FnOnce(&mut u32),
    {
        let mut value = self.read_conf();
        f(&mut value);
        self.write_conf(value);
    }

    pub fn read_ce_ctrl(&self) -> u32 { ... }
    pub fn write_ce_ctrl(&self, value: u32) { ... }
    
    pub fn read_intr_ctrl(&self) -> u32 { ... }
    pub fn write_intr_ctrl(&self, value: u32) { ... }
    pub fn modify_intr_ctrl<F>(&self, f: F) where F: FnOnce(&mut u32) { ... }
    
    // DMA registers
    pub fn read_dma_ctrl(&self) -> u32 { ... }
    pub fn write_dma_ctrl(&self, value: u32) { ... }
    pub fn read_dma_addr(&self) -> u32 { ... }
    pub fn write_dma_addr(&self, value: u32) { ... }
    pub fn read_dma_len(&self) -> u32 { ... }
    pub fn write_dma_len(&self, value: u32) { ... }
    
    // Segment registers (CS0, CS1)
    pub fn read_segment(&self, cs: usize) -> u32 { ... }
    pub fn write_segment(&self, cs: usize, value: u32) { ... }
    
    // Timing registers
    pub fn read_timings(&self) -> u32 { ... }
    pub fn write_timings(&self, value: u32) { ... }
    pub fn read_timings2(&self) -> u32 { ... }
    pub fn write_timings2(&self, value: u32) { ... }
    
    // FMC-specific: WDT boot control
    pub fn read_wdt2_ctrl(&self) -> u32 { ... }
    pub fn write_wdt2_ctrl(&self, value: u32) { ... }
}
```

**Benefits:**
- **Single audit point** for all unsafe code
- **Consistent interface** for all registers
- **Easy to mock** for testing (`struct MockRegisters` in tests)
- **Type-safe wrappers** eliminate bit-shifting errors

---

### 2.2 `types.rs` — Error Handling & Configuration

**Error types distinguish retryable from terminal**, following AST1060 PAC guide:

```rust
/// Terminal errors: operation failed, don't retry
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcError {
    // Hardware errors (terminal)
    HardwareError,
    Timeout,
    
    // DMA-specific
    DmaAborted,
    DmaLengthMismatch,
    
    // Configuration errors (terminal)
    InvalidChipSelect,
    InvalidCapacity,
    DeviceNotSupported,
    
    // Flash operation errors (retryable or terminal)
    WriteProtected,  // Terminal; fix configuration
    WriteInProgress, // Terminal; device is busy
}

/// Retryable errors returned as `Err(nb::Error::WouldBlock)`
#[derive(Clone, Copy, Debug)]
pub enum SmcRetryable {
    NotReady,           // DMA controller busy
    DmaTransferPending, // DMA in-flight
}

impl From<SmcError> for nb::Error<SmcError> {
    fn from(e: SmcError) -> Self {
        nb::Error::Other(e)
    }
}

impl From<SmcRetryable> for nb::Error<SmcError> {
    fn from(_: SmcRetryable) -> Self {
        nb::Error::WouldBlock
    }
}

impl embedded_storage::Error for SmcError {
    fn kind(&self) -> embedded_storage::ErrorKind {
        embedded_storage::ErrorKind::Other
    }
}

/// Configuration for a single flash device
#[derive(Clone, Copy, Debug)]
pub struct FlashConfig {
    pub capacity_mb: u32,
    pub page_size: u32,      // 256 bytes typical
    pub sector_size: u32,    // 4096 bytes typical
    pub block_size: u32,     // 64 KB typical
    pub spi_clock_mhz: u32,  // Desired SPI bus frequency
}

impl FlashConfig {
    pub const fn winbond_w25q64() -> Self {
        Self {
            capacity_mb: 8,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 25,
        }
    }

    pub const fn winbond_w25q256() -> Self {
        Self {
            capacity_mb: 32,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 50,
        }
    }
}

/// Per-controller configuration
#[derive(Clone, Copy, Debug)]
pub struct SmcConfig {
    pub controller_id: SmcController,
    pub cs0: Option<FlashConfig>,
    pub cs1: Option<FlashConfig>,
    pub dma_enabled: bool,
    pub enable_interrupts: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcController {
    Fmc = 0,
    Spi1 = 1,
    Spi2 = 2,
}

impl SmcController {
    pub fn base_address(&self) -> uintptr_t {
        match self {
            Self::Fmc => 0x7E620000,
            Self::Spi1 => 0x7E630000,
            Self::Spi2 => 0x7E640000,
        }
    }

    pub fn flash_window_address(&self) -> uintptr_t {
        match self {
            Self::Fmc => 0x80000000,
            Self::Spi1 => 0x90000000,
            Self::Spi2 => 0xB0000000,
        }
    }

    pub fn irq_number(&self) -> u32 {
        match self {
            Self::Fmc => 39,
            Self::Spi1 => 65,
            Self::Spi2 => 66,
        }
    }
}
```

---

### 2.3 `controller.rs` — Generic SMC Controller

**One controller per hardware instance (FMC, SPI1, SPI2).**

```rust
use crate::smc::registers::SmcRegisters;
use crate::smc::types::*;

/// Generic SMC controller state
pub struct Smc {
    regs: SmcRegisters,
    controller_id: SmcController,
    config: SmcConfig,
    state: SmcState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmcState {
    Uninitialized,
    Ready,
    DmaInFlight,
    Error,
}

impl Smc {
    /// # Safety
    /// Caller must ensure:
    /// - No other SMC instance exists for this hardware controller
    /// - Pointer points to valid SMC register block
    pub unsafe fn new(
        controller_id: SmcController,
        config: SmcConfig,
    ) -> Result<Self, SmcError> {
        // Validate configuration
        if config.cs0.is_none() && config.cs1.is_none() {
            return Err(SmcError::InvalidCapacity);
        }

        let base = controller_id.base_address() as *const _;
        let regs = SmcRegisters::new(base);

        Ok(Self {
            regs,
            controller_id,
            config,
            state: SmcState::Uninitialized,
        })
    }

    /// Initialize hardware. Must be called before any I/O operations.
    pub fn init(&mut self) -> Result<(), SmcError> {
        // 1. Configure flash types and write-enable
        let mut conf = 0u32;
        if self.config.cs0.is_some() {
            conf |= 1 << 16; // CONF_ENABLE_W0
            conf |= 0x2 << 0; // FLASH_TYPE_SPI
        }
        if self.config.cs1.is_some() {
            conf |= 1 << 17; // CONF_ENABLE_W1
            conf |= 0x2 << 2; // FLASH_TYPE_SPI
        }
        self.regs.write_conf(conf);

        // 2. Configure timing for each CS
        if let Some(cs_cfg) = self.config.cs0 {
            self.configure_timing(0, &cs_cfg)?;
        }
        if let Some(cs_cfg) = self.config.cs1 {
            self.configure_timing(1, &cs_cfg)?;
        }

        // 3. Set up segment addresses (memory map)
        self.setup_segments()?;

        // 4. Enable interrupts if requested
        if self.config.enable_interrupts {
            let mut intr = self.regs.read_intr_ctrl();
            intr |= 1 << 3; // INTR_CTRL_DMA_EN
            self.regs.write_intr_ctrl(intr);
        }

        self.state = SmcState::Ready;
        Ok(())
    }

    fn configure_timing(&self, cs: usize, config: &FlashConfig) -> Result<(), SmcError> {
        // Calculate clock divisor based on desired SPI frequency
        // SYSCLK = 200 MHz
        let ideal_clk_div = Self::calculate_clock_divisor(200, config.spi_clock_mhz)?;

        // Set timing: [31:28]=SETUP, [27:24]=HOLD, [23:20]=CLK_DIV
        let timings = (1u32 << 28) | (1u32 << 24) | (ideal_clk_div << 20);
        self.regs.write_timings(timings);

        Ok(())
    }

    fn setup_segments(&self) -> Result<(), SmcError> {
        let window_base = self.controller_id.flash_window_address();
        let cs0_size = self.config.cs0.map(|c| c.capacity_mb as usize * 1024 * 1024).unwrap_or(0);
        let cs1_size = self.config.cs1.map(|c| c.capacity_mb as usize * 1024 * 1024).unwrap_or(0);

        // Validate no overlap
        if cs0_size + cs1_size > 256 * 1024 * 1024 {
            return Err(SmcError::InvalidCapacity);
        }

        // Write segment registers (4 KB units)
        if cs0_size > 0 {
            let seg = Self::encode_segment(0, cs0_size)?;
            self.regs.write_segment(0, seg);
        }

        if cs1_size > 0 {
            let seg = Self::encode_segment(cs0_size, cs0_size + cs1_size)?;
            self.regs.write_segment(1, seg);
        }

        Ok(())
    }

    fn encode_segment(start: usize, end: usize) -> Result<u32, SmcError> {
        // Convert byte addresses to 4 KB units
        let start_4k = (start >> 12) as u32;
        let end_4k = (end >> 12) as u32;

        if start_4k > 0xFFFF || end_4k > 0xFFFF {
            return Err(SmcError::InvalidCapacity);
        }

        Ok((end_4k << 16) | start_4k)
    }

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

    /// Read from flash via memory window (PIO mode)
    pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        if self.state != SmcState::Ready {
            return Err(SmcError::HardwareError);
        }

        let window = self.controller_id.flash_window_address() as *const u32;
        let flash_ptr = unsafe { (window as *const u8).add(offset as usize) };

        // Copy from flash window (hardware automatically handles SPI)
        unsafe {
            core::ptr::copy_nonoverlapping(flash_ptr, buf.as_mut_ptr(), buf.len());
        }

        Ok(buf.len())
    }

    /// Initiate DMA read (non-blocking)
    pub fn dma_read(
        &mut self,
        flash_offset: u32,
        dram_addr: uintptr_t,
        len: u32,
    ) -> Result<(), SmcError> {
        if self.state != SmcState::Ready && self.state != SmcState::DmaInFlight {
            return Err(SmcError::HardwareError);
        }

        // Set up DMA registers
        self.regs.write_dma_addr(dram_addr as u32 & 0x000BFFFC);
        self.regs.write_dma_len(len - 1);

        // Set segment for flash address (simplified; always CS0)
        let seg = Self::encode_segment(
            flash_offset as usize,
            (flash_offset + len) as usize,
        )?;
        self.regs.write_segment(0, seg);

        // Trigger DMA
        self.regs.write_dma_ctrl(0x1); // DMA_CTRL_REQUEST

        self.state = SmcState::DmaInFlight;
        Ok(())
    }

    /// Check DMA status
    pub fn dma_status(&self) -> SmcState {
        if self.state != SmcState::DmaInFlight {
            return self.state;
        }

        let intr_ctrl = self.regs.read_intr_ctrl();
        if (intr_ctrl & (1 << 11)) != 0 {
            // DMA_CTRL_DMA_STATUS set
            return SmcState::Ready;
        }

        SmcState::DmaInFlight
    }

    /// Clear DMA completion flag (for next transfer)
    pub fn dma_clear(&mut self) {
        let mut intr_ctrl = self.regs.read_intr_ctrl();
        intr_ctrl &= !(1 << 11); // Clear DMA_STATUS
        self.regs.write_intr_ctrl(intr_ctrl);
        
        if self.state == SmcState::DmaInFlight {
            self.state = SmcState::Ready;
        }
    }

    pub fn is_ready(&self) -> bool {
        self.state == SmcState::Ready
    }

    pub fn controller_id(&self) -> SmcController {
        self.controller_id
    }
}
```

---

### 2.4 `fmc.rs` & `spi.rs` — Specialized Controllers

**FMC adds:** WDT-based boot switching, redundancy management
**SPI1/SPI2:** Simpler configuration, shared register layout

```rust
// fmc.rs
use crate::smc::controller::Smc;
use crate::smc::types::*;

pub struct FmcController {
    inner: Smc,
}

impl FmcController {
    pub unsafe fn new(config: SmcConfig) -> Result<Self, SmcError> {
        let inner = Smc::new(SmcController::Fmc, config)?;
        Ok(Self { inner })
    }

    pub fn init(&mut self) -> Result<(), SmcError> {
        self.inner.init()
    }

    /// Switch boot image: CS0 <-> CS1
    /// Atomically sets WDT control and resets CPU
    pub fn switch_boot_image(&self, use_alternate: bool) -> ! {
        let mut wdt_ctrl = self.inner.regs.read_wdt2_ctrl();
        if use_alternate {
            wdt_ctrl |= 1 << 4; // BOOT_SOURCE = CS1
        } else {
            wdt_ctrl &= !(1 << 4); // BOOT_SOURCE = CS0
        }
        self.inner.regs.write_wdt2_ctrl(wdt_ctrl);

        // Trigger watchdog reset
        // (assumes watchdog HAL available)
        loop {
            cortex_m::asm::nop();
        }
    }

    // Delegated methods from Smc:
    pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        self.inner.read(offset, buf)
    }

    pub fn dma_read(&mut self, offset: u32, dram_addr: uintptr_t, len: u32) -> Result<(), SmcError> {
        self.inner.dma_read(offset, dram_addr, len)
    }
}

// spi.rs
pub struct SpiController {
    inner: Smc,
}

impl SpiController {
    pub unsafe fn new(
        id: SmcController,
        config: SmcConfig,
    ) -> Result<Self, SmcError> {
        if id == SmcController::Fmc {
            return Err(SmcError::HardwareError);
        }
        let inner = Smc::new(id, config)?;
        Ok(Self { inner })
    }

    pub fn init(&mut self) -> Result<(), SmcError> {
        self.inner.init()
    }

    // Delegated to Smc:
    pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        self.inner.read(offset, buf)
    }

    pub fn dma_read(&mut self, offset: u32, dram_addr: uintptr_t, len: u32) -> Result<(), SmcError> {
        self.inner.dma_read(offset, dram_addr, len)
    }
}
```

---

### 2.5 `device/flash.rs` — Flash Device Operations

**Higher-level abstractions: erase, program, verify.**

#### Phase 3A Assessment (Implemented)

Current Phase 3A scope is implemented as a read-only facade over wrapper layers:
- Added `smc/device/mod.rs` and `smc/device/flash.rs`
- Added minimal `FlashDevice` API:
    - `read(offset, buf) -> Result<usize, SmcError>`
    - `capacity_bytes() -> Result<usize, SmcError>`
- Added wrapper-aware constructors:
    - `SpiNorFlash::from_fmc(&mut FmcReady, cfg) -> Result<_, SmcError>`
    - `SpiNorFlash::from_spi(&mut SpiReady, cfg) -> Result<_, SmcError>`
- Reads are delegated to wrapper/controller paths; no duplicate register logic
- Build wiring and QEMU/portable smoke tests are in place

Review findings for this unit:
1. Capacity source mismatch risk (medium):
     - `capacity_bytes()` is currently derived from facade-local `FlashConfig`, not
         from controller-owned capacity state.
     - If facade config diverges from controller config, reported capacity may not
         match read-time bounds validation.
2. Coverage gap (low):
     - Current tests exercise `from_fmc` path only; `from_spi` constructor path is
         not yet directly tested.

Recommended follow-ups before Phase 3B:
- Validate facade `cfg` against wrapper/controller capacity at construction.
- Add SPI-path smoke coverage (`from_spi`) for construction + read + bounds.
- Keep write/erase out of scope until command transport is finalized.

Recommendation on `FlashConfig` ownership:
- Keep a **single source of truth** for capacity in Phase 3A (controller/wrapper
    state). The read-only facade should not introduce an independent capacity
    authority.
- `FlashConfig` in the device layer is still justified for Phase 3B+ because
    write/erase paths require page/sector/block geometry and device policy data.
- If `FlashConfig` is carried by the facade before Phase 3B, treat it as
    validated metadata only: check once at construction against controller state,
    then fail fast on mismatch.

```rust
use crate::smc::controller::Smc;
use crate::smc::types::*;

/// SPI NOR Flash standard operations
pub trait FlashDevice {
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError>;
    fn erase_sector(&mut self, sector_addr: u32) -> Result<(), SmcError>;
    fn program_page(&mut self, offset: u32, data: &[u8]) -> Result<usize, SmcError>;
    fn verify(&self, offset: u32, expected: &[u8]) -> Result<bool, SmcError>;
    fn get_status(&self) -> u8;
}

/// Standard SPI NOR flash commands
pub mod commands {
    pub const READ: u8 = 0x03;
    pub const FAST_READ: u8 = 0x0B;
    pub const WRITE_ENABLE: u8 = 0x06;
    pub const WRITE_DISABLE: u8 = 0x04;
    pub const ERASE_SECTOR: u8 = 0x20;      // 4 KB
    pub const ERASE_BLOCK: u8 = 0xD8;       // 64 KB
    pub const PAGE_PROGRAM: u8 = 0x02;
    pub const READ_STATUS: u8 = 0x05;
    pub const READ_JEDEC_ID: u8 = 0x9F;
}

/// Wrapper around Smc for flash-specific operations
pub struct SpiNorFlash<'a> {
    smc: &'a mut Smc,
    config: FlashConfig,
}

impl<'a> SpiNorFlash<'a> {
    pub fn new(smc: &'a mut Smc, config: FlashConfig) -> Self {
        Self { smc, config }
    }

    /// Erase a 4 KB sector
    pub fn erase_sector(&mut self, addr: u32) -> Result<(), SmcError> {
        if addr % self.config.sector_size != 0 {
            return Err(SmcError::HardwareError);
        }

        // Send WREN command (simplified; assumes bit-bang available)
        self.send_command(&[commands::WRITE_ENABLE], &mut [])?;

        // Send ERASE_SECTOR + address
        let cmd_buf = [
            commands::ERASE_SECTOR,
            ((addr >> 16) & 0xFF) as u8,
            ((addr >> 8) & 0xFF) as u8,
            (addr & 0xFF) as u8,
        ];
        self.send_command(&cmd_buf, &mut [])?;

        // Poll WIP bit until clear
        self.wait_write_complete()?;

        Ok(())
    }

    /// Program a page (up to 256 bytes)
    pub fn program_page(&mut self, addr: u32, data: &[u8]) -> Result<(), SmcError> {
        if data.len() > self.config.page_size as usize {
            return Err(SmcError::InvalidCapacity);
        }

        // Send WREN
        self.send_command(&[commands::WRITE_ENABLE], &mut [])?;

        // Send PP + address + data
        let mut cmd_buf = [0u8; 4];
        cmd_buf[0] = commands::PAGE_PROGRAM;
        cmd_buf[1] = ((addr >> 16) & 0xFF) as u8;
        cmd_buf[2] = ((addr >> 8) & 0xFF) as u8;
        cmd_buf[3] = (addr & 0xFF) as u8;
        self.send_command(&cmd_buf, data)?;

        // Poll WIP
        self.wait_write_complete()?;

        Ok(())
    }

    /// Verify written data
    pub fn verify(&self, addr: u32, expected: &[u8]) -> Result<bool, SmcError> {
        let mut readback = [0u8; 256];
        if expected.len() > readback.len() {
            return Err(SmcError::InvalidCapacity);
        }

        self.smc.read(addr, &mut readback[..expected.len()])?;
        Ok(&readback[..expected.len()] == expected)
    }

    fn send_command(&mut self, cmd: &[u8], data: &[u8]) -> Result<(), SmcError> {
        // This is a placeholder; actual implementation depends on having
        // a bit-bang SPI interface or command queue registers
        // For now, assumes this is handled by a separate UART-like command interface
        todo!("Implement based on actual flash command interface")
    }

    fn wait_write_complete(&self) -> Result<(), SmcError> {
        let mut timeout = 10000;
        while timeout > 0 {
            let status = self.read_status()?;
            if (status & 0x01) == 0 {
                return Ok(()); // WIP clear
            }
            timeout -= 1;
        }
        Err(SmcError::Timeout)
    }

    fn read_status(&self) -> Result<u8, SmcError> {
        // Placeholder: requires command interface
        todo!("Read status register")
    }
}

impl<'a> FlashDevice for SpiNorFlash<'a> {
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        self.smc.read(offset, buf)
    }

    fn erase_sector(&mut self, sector_addr: u32) -> Result<(), SmcError> {
        self.erase_sector(sector_addr)
    }

    fn program_page(&mut self, offset: u32, data: &[u8]) -> Result<usize, SmcError> {
        self.program_page(offset, data)?;
        Ok(data.len())
    }

    fn verify(&self, offset: u32, expected: &[u8]) -> Result<bool, SmcError> {
        self.verify(offset, expected)
    }

    fn get_status(&self) -> u8 {
        self.read_status().unwrap_or(0)
    }
}
```

---

### 2.6 `interrupts.rs` — Interrupt Handling

**Safe wrappers for ISR context**, following AST1060 PAC guide:

```rust
use crate::smc::types::SmcController;

#[derive(Clone, Copy, Debug)]
pub enum SmcInterrupt {
    DmaComplete,
    DmaError,
    CommandAbort,
    WriteProtected,
    Unknown,
}

pub struct SmcInterruptDecoder;

impl SmcInterruptDecoder {
    pub fn decode(intr_ctrl: u32) -> SmcInterrupt {
        // Bit fields from hardware
        if (intr_ctrl & (1 << 11)) != 0 {
            SmcInterrupt::DmaComplete
        } else if (intr_ctrl & (1 << 10)) != 0 {
            SmcInterrupt::CommandAbort
        } else if (intr_ctrl & (1 << 9)) != 0 {
            SmcInterrupt::WriteProtected
        } else {
            SmcInterrupt::Unknown
        }
    }
}

/// Safe ISR context: only allows non-blocking operations
pub struct SmcIsrContext {
    pub controller_id: SmcController,
    pub interrupt: SmcInterrupt,
}

// User provides ISR handler:
/*
fn fmc_dma_isr() {
    let ctx = SmcIsrContext {
        controller_id: SmcController::Fmc,
        interrupt: SmcInterruptDecoder::decode(fmc_intr_ctrl),
    };
    
    // Safe to call from ISR
    app_handle_smc_interrupt(ctx);
}
*/
```

---

## 3. Trait Implementations

### 3.1 Storage Error Trait (Recommended)

**Use `embedded_storage::Error`, not `embedded_hal::spi::Error`.**

SMC is a **flash storage controller**, not a raw SPI bus. Using the correct trait improves semantic clarity and downstream compatibility:

```rust
// ✓ CORRECT: Storage-oriented error trait
use embedded_storage;

impl embedded_storage::Error for SmcError {
    fn kind(&self) -> embedded_storage::ErrorKind {
        embedded_storage::ErrorKind::Other
        // Could be more specific:
        // - Read → SmcError::HardwareError
        // - Write → SmcError::WriteProtected, WriteInProgress
        // - Erase → SmcError::Timeout, DmaAborted
    }
}
```

**Why `embedded_storage` over `embedded_hal::spi`:**

| Trait | Purpose | Error Kinds |
|-------|---------|-----------|
| `embedded_hal::spi::Error` | SPI bus controllers | Protocol errors (clock, MOSI, MISO, CS) |
| `embedded_storage::Error` | Flash/storage devices | Storage operations (read, write, erase, verify) |

SMC provides **memory access abstraction**, not protocol-level SPI. Errors are storage-domain (write-protected, timeout, DMA failure), not bus-domain.

**Example: Downstream code clarity:**

```rust
// With embedded_storage trait
fn flash_write<E: embedded_storage::Error>(data: &[u8]) -> Result<(), E> {
    // Clearly for storage operations
    // Caller knows errors are write/erase/verify related
}

// With embedded_hal::spi trait (misleading)
fn flash_write<E: embedded_hal::spi::Error>(data: &[u8]) -> Result<(), E> {
    // Ambiguous: is this SPI protocol or storage operations?
    // Caller confused about error semantics
}
```

### 3.2 Optional: Flash Device Trait

For future flash operation support, define a custom trait aligned with storage semantics:

```rust
use embedded_storage::Error;

/// Flash-aware operations beyond raw memory access
pub trait FlashOps: Error {
    fn erase_sector(&mut self, offset: u32) -> Result<(), Self>;
    fn program_page(&mut self, offset: u32, data: &[u8]) -> Result<(), Self>;
    fn verify(&self, offset: u32, expected: &[u8]) -> Result<bool, Self>;
    fn get_status(&self) -> u8;
}
```

### 3.3 Core Traits (NOT Recommended)

```rust
// ✗ DO NOT implement SpiDevice/SpiBus
// (SMC is not a general-purpose SPI bus; it's flash-specific)
// impl embedded_hal::spi::SpiDevice for Smc { }

// ✗ DO NOT implement embedded_hal::spi::Error directly
// (Use embedded_storage::Error instead for semantic correctness)
// impl embedded_hal::spi::Error for SmcError { }
```

### 3.2 Custom Traits for FMC/SPI

```rust
pub trait SmcRead {
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError>;
    fn read_u32(&self, offset: u32) -> Result<u32, SmcError> {
        let mut buf = [0u8; 4];
        self.read(offset, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }
}

pub trait SmcDma {
    fn dma_read(&mut self, offset: u32, dram: uintptr_t, len: u32) -> Result<(), SmcError>;
    fn dma_status(&self) -> bool; // true = in-flight
}

impl SmcRead for Smc { ... }
impl SmcDma for Smc { ... }
```

---

## 4. Error Handling Philosophy

**Three error categories:**

| Category | Examples | Handling |
|----------|----------|----------|
| **Configuration** | Invalid CS, out-of-range capacity | Return `Err(SmcError::...)` at init |
| **Operational** (terminal) | Timeout, hardware error | Return `Err(SmcError::...)` on operation |
| **Operational** (retryable) | DMA in-flight, not ready | Return `Err(nb::Error::WouldBlock)` for non-blocking API |

```rust
/// Non-blocking API uses nb::Result
pub fn dma_read_nonblocking(
    &mut self,
    offset: u32,
    dram: uintptr_t,
    len: u32,
) -> nb::Result<(), SmcError> {
    if !self.is_ready() {
        return Err(nb::Error::WouldBlock); // Retryable
    }
    
    self.dma_read(offset, dram, len)
        .map_err(|e| nb::Error::Other(e))
}

/// Blocking API polls internally
pub fn dma_read_blocking(
    &mut self,
    offset: u32,
    dram: uintptr_t,
    len: u32,
) -> Result<(), SmcError> {
    loop {
        match self.dma_read_nonblocking(offset, dram, len) {
            Ok(()) => return Ok(()),
            Err(nb::Error::WouldBlock) => {
                core::hint::spin_loop(); // Retry
            }
            Err(nb::Error::Other(e)) => return Err(e),
        }
    }
}
```

---

## 5. Initialization Sequence (Safe & Enforced)

**Key principle: Constructor is unsafe; init() is safe; hardware configured at construction.**

```rust
// In application:

fn main() -> ! {
    // Assume SCU clock already initialized to 200 MHz
    
    // Step 1: Create FMC with config (UNSAFE)
    let mut fmc = unsafe {
        FmcController::new(SmcConfig {
            controller_id: SmcController::Fmc,
            cs0: Some(FlashConfig::winbond_w25q256()),
            cs1: Some(FlashConfig::winbond_w25q64()),
            dma_enabled: true,
            enable_interrupts: true,
        })?
    };
    
    // Step 2: Initialize hardware (SAFE)
    fmc.init()?; // Compiler enforces this is called
    
    // Step 3: Use
    let mut buf = [0u8; 256];
    fmc.read(0x1000, &mut buf)?;
}
```

**Guarantees:**
- Constructor's `unsafe` forces caller to acknowledge ownership risk
- `init()` is typed as `fn init(&mut self)`, ensuring it's called before use
- Type system prevents using uninitialized controller

---

## 6. Testing Strategy

### 6.1 Unit Tests (No Hardware)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct MockRegisters {
        conf: u32,
        ce_ctrl: u32,
        intr_ctrl: u32,
        dma_ctrl: u32,
        dma_addr: u32,
        dma_len: u32,
        segments: [u32; 2],
        timings: u32,
    }

    impl MockRegisters {
        fn new() -> Self {
            Self {
                conf: 0,
                ce_ctrl: 0,
                intr_ctrl: 0,
                dma_ctrl: 0,
                dma_addr: 0,
                dma_len: 0,
                segments: [0; 2],
                timings: 0,
            }
        }
    }

    #[test]
    fn test_encode_segment() {
        let seg = Smc::encode_segment(0, 16 * 1024 * 1024).unwrap();
        let start_4k = seg & 0xFFFF;
        let end_4k = (seg >> 16) & 0xFFFF;
        assert_eq!(start_4k, 0);
        assert_eq!(end_4k, 4096); // 16 MB / 4 KB
    }

    #[test]
    fn test_clock_divisor_calculation() {
        let div = Smc::calculate_clock_divisor(200, 25).unwrap();
        assert_eq!(div, 3); // 200 / 2^3 = 25
    }

    #[test]
    fn test_config_validation() {
        let bad_config = SmcConfig {
            controller_id: SmcController::Fmc,
            cs0: None,
            cs1: None,
            dma_enabled: true,
            enable_interrupts: false,
        };

        let result = unsafe { Smc::new(SmcController::Fmc, bad_config) };
        assert!(result.is_err());
    }
}
```

### 6.2 Integration Tests (With QEMU)

```bash
# Run on QEMU AST1030
.cargo/bin/test_smc_integration \
    --fmc-flash /path/to/fmc_test_image.bin \
    --spi1-flash /path/to/spi1_test_image.bin

# Expected output:
# FMC: Read 256 bytes @ 0x00000000 ... OK
# SPI1: Read 256 bytes @ 0x00000000 ... OK
# FMC: DMA read 4 KB to SRAM ... OK
# Boot switch test (triggers reset) ... OK
```

---

## 7. Incremental Implementation Plan

### Phase 1: Core Register Access (Week 1–2)

- [ ] Implement `registers.rs` with safe wrappers
- [ ] Define `types.rs` (SmcError, FlashConfig, SmcController)
- [ ] Unit test register encoding/validation

**Deliverable:** Single register access layer, testable in isolation

### Phase 2: Generic Controller (Week 2–3)

- [ ] Implement `controller.rs` (Smc struct)
- [ ] PIO read operation
- [ ] Segment and timing configuration
- [ ] Integration test with QEMU

**Deliverable:** Can read flash via memory window on all three controllers

### Phase 3: DMA Support (Week 3–4)

- [ ] Implement DMA initiation and status polling
- [ ] Interrupt handling in `interrupts.rs`
- [ ] DMA completion callback
- [ ] Integration test with DMA transfers

**Deliverable:** Non-blocking reads for high throughput

### Phase 4: Flash Operations (Week 4–5)

- [ ] Implement `device/flash.rs` (erase, program, verify)
- [ ] Command queue or bit-bang interface
- [ ] Write-in-progress polling
- [ ] Integration test: erase + program + verify cycle

**Deliverable:** Full read/write support for single device

### Phase 5: Multi-Flash & Redundancy (Week 5–6)

- [ ] Dual-image support (CS0 = active, CS1 = backup)
- [ ] Boot switching (FMC WDT control)
- [ ] Atomic failover logic
- [ ] Integration test: Cold boot from alternate image

**Deliverable:** Redundant firmware update capability

### Phase 6: Documentation & Examples (Week 6–7)

- [ ] API documentation (rustdoc)
- [ ] Example applications
- [ ] Board support package (BSP) integration
- [ ] Production readiness review

**Deliverable:** Ready for production integration

---

## 8. Key Design Decisions Explained

| Decision | Rationale |
|----------|-----------|
| **Unsafe constructor** | Matches AST1060 PAC guide; forces caller to acknowledge ownership |
| **Single `regs()` method** | Consolidated audit surface; easy to mock for testing |
| **Per-controller types** (Fmc, Spi) | Specialization for FMC boot-switching; clarity for users |
| **Generic Smc base class** | Code reuse; no DRY violation |
| **Separate FlashDevice trait** | Flash operations deferred until needed; driver is minimal |
| **nb::Result for DMA** | Allows non-blocking patterns; familiar to embedded-hal users |
| **SmcInterruptDecoder** | Safe ISR integration (no unsafe in handler) |
| **Four-tier stack** | Registers → Controller → Flash → Application |

---

## 9. Comparison with Current Implementation

| Aspect | Current SPI | Recommended | Improvement |
|--------|------------|-------------|------------|
| Unsafe perimeter | Scattered macros (20+) | Single `regs()` | ✓ Auditability |
| Constructor safety | Not unsafe | Unsafe + PhantomData | ✓ Ownership enforced |
| Error handling | All → Other | Granular + retryable | ✓ Better debugging |
| Initialization | Deferred `init()` | In constructor | ✓ Can't forget |
| Register access | Macro-based | Method-based | ✓ Testability |
| DMA support | Present | Non-blocking API | ✓ Interrupt-safe |
| Interrupt handling | Unsafe functions | Safe enum decoder | ✓ ISR safety |
| Documentation | Minimal | Comprehensive | ✓ Maintainability |
| Testing | Limited | Unit + integration | ✓ Reliability |

---

## 10. Summary: The Three-Layer Cake

```
┌─────────────────────────────────────────────────┐
│         Application Layer                       │
│  (Firmware updates, boot logic, etc.)           │
└──────────────┬──────────────────────────────────┘
               │
┌──────────────▼──────────────────────────────────┐
│         Device Layer (device/flash.rs)          │
│  - Erase, Program, Verify                       │
│  - Flash-specific commands                      │
│  - Status polling                               │
└──────────────┬──────────────────────────────────┘
               │
┌──────────────▼──────────────────────────────────┐
│    Controller Layer (Smc, Fmc, Spi)             │
│  - PIO reads, DMA transfers                     │
│  - Segment mapping, timing config               │
│  - State management                             │
└──────────────┬──────────────────────────────────┘
               │
┌──────────────▼──────────────────────────────────┐
│  Register Layer (registers.rs)                  │
│  - Safe register wrappers                       │
│  - Single unsafe perimeter (regs() method)      │
│  - Hardware state synchronization               │
└──────────────┬──────────────────────────────────┘
               │
        Hardware (FMC, SPI1, SPI2)
        ├─ Register blocks @ 0x7E620000, etc.
        ├─ DMA engines
        ├─ SPI bus interface
        └─ Interrupt lines (IRQ 39, 65, 66)
```

Each layer:
- Has clear responsibility
- Can be tested independently
- Hides complexity below
- Provides safe, typed interface above

---

## Conclusion

**Start with layers 1–2 (registers + controllers)** and test thoroughly with QEMU. This gives you:
- A proven register safety model
- Non-blocking DMA support
- Interrupt-safe operations
- ~70% of the HAL value with 20% of the complexity

**Add layer 3 (flash operations)** only when you need write support, using the proven patterns established below.

This architecture prioritizes:
1. **Safety** (unsafe in one place)
2. **Testability** (mock-friendly)
3. **Simplicity** (start minimal)
4. **Extensibility** (add features as needed)
