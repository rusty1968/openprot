# AST10x0 Peripheral Ownership Pattern Analysis

## Executive Summary

The AST10x0 reference implementation uses a **type-state pattern** with **immutable borrowed references** for peripheral ownership. This approach enforces hardware lifecycle stages at compile time while avoiding shared ownership mechanisms (Rc/Arc). Peripherals are created with `unsafe` constructors, initialized through type transitions, and passed around as references with lifetime bounds.

---

## 1. Peripheral Struct Definition

### 1.1 SMC (Static Memory Controller) Pattern

**Files:**
- [ast10x0-pac-guide.md](ast10x0-pac-guide.md) - High-level architecture
- [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) - Main controller
- [peripherals/smc/types.rs](target/ast10x0/peripherals/smc/types.rs) - Type definitions

**Pattern: Generic Type-State with PhantomData**

```rust
// From /target/ast10x0/peripherals/smc/controller.rs (lines 45-65)
pub struct Smc<B: SmcRegisterBackend, Mode> {
    regs: B,                              // Hardware backend (FMC or SPI)
    controller_id: SmcController,         // Which SMC instance
    config: SmcConfig,                    // Runtime configuration
    state: SmcState,                      // Internal state machine
    normal_read_ctrl: [u32; 2],          // Per-CS configuration snapshots
    flash_window_base: [usize; 2],       // AHB flash window addresses
    _mode: PhantomData<fn() -> Mode>,    // Type-state marker (zero-cost)
}

// Type aliases for ergonomic access
pub type UninitSmc = Smc<FmcRegisterBackend, Uninitialized>;
pub type ReadySmc = Smc<FmcRegisterBackend, Ready>;
```

**Key characteristics:**
- **Generic over backend `B`:** Allows FMC or SPI register implementations
- **Generic over mode `Mode`:** `Uninitialized`, `Ready` — enforces init ordering
- **PhantomData marker:** Zero-cost type state (no runtime overhead)
- **No allocation:** All data is stack-allocated

### 1.2 Type-State Markers

**Files:**
- [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) (lines 50-57)
- [peripherals/spimonitor/controller.rs](target/ast10x0/peripherals/spimonitor/controller.rs) (lines 19-26)

```rust
// SMC type states (lines 50-57)
pub struct Uninitialized;  // Controller created but hw not initialized
pub struct Ready;          // Hardware initialized, ready for operations

// SPI Monitor type states (lines 19-26)
pub struct Uninitialized;
pub struct Configured;     // Policy tables programmed
pub struct Locked;         // Policy write-protected (irreversible)
```

### 1.3 UART Peripheral

**Files:**
- [peripherals/uart/mod.rs](target/ast10x0/peripherals/uart/mod.rs) (lines 99-107)

```rust
// Simpler peripheral without type state
pub struct Usart {
    usart: *const device::uart::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>,  // Prevent Sync (thread-safe reads only)
}
```

**Key characteristics:**
- Raw pointer to register block
- `PhantomData<UnsafeCell<()>>` makes type `!Sync` while allowing `Send`
- Used for simple I/O (UART) without complex state transitions

### 1.4 SPI Monitor Peripheral

**Files:**
- [peripherals/spimonitor/controller.rs](target/ast10x0/peripherals/spimonitor/controller.rs) (lines 26-35)

```rust
pub struct SpiMonitor<Mode> {
    regs: SpiMonitorRegisters,
    controller: SpiMonitorController,
    scu: ScuRegisters,                      // SCU for mux control
    _mode: PhantomData<fn() -> Mode>,
}

pub type UninitSpiMonitor = SpiMonitor<Uninitialized>;
pub type ConfiguredSpiMonitor = SpiMonitor<Configured>;
pub type LockedSpiMonitor = SpiMonitor<Locked>;
```

---

## 2. How Peripherals Are Passed Around

### 2.1 Owned Values at Init Time Only

**Pattern:** Create with `unsafe`, consume into next stage

```rust
// From target/ast10x0/tests/smc/target.rs (lines 31-49)
let config = SmcConfig {
    controller_id: SmcController::Fmc,
    cs0: Some(FlashConfig { capacity_mb: 1, /* ... */ }),
    cs1: None,
    dma_enabled: false,
    enable_interrupts: false,
    topology: SmcTopology::BootSpi { master_idx: 0 },
};

// STEP 1: Create uninitialized controller (owned value)
let controller = unsafe { UninitSmc::new(config)? };

// STEP 2: Consume into initialized controller (move, not copy)
let mut controller = controller.init()?;

// STEP 3: Use as mutable reference
controller.read(0, &mut buf)?;
```

**Key points:**
- Initialization consumes the `Uninitialized` variant, moves to `Ready`
- Prevents using uninitialized peripherals at compile time
- Only one ownership chain: uninit → init → use

### 2.2 Immutable Borrowed References for Operations

**Files:**
- [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) (lines 195-217)

```rust
// All read operations use immutable self
impl<B: SmcRegisterBackend> Smc<B, Ready> {
    pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        // ... validation ...
        let window = self.controller_id.flash_window_address() as *const u8;
        unsafe {
            // SAFETY: Read from fixed MMIO window
            core::ptr::copy_nonoverlapping(flash_ptr, buf.as_mut_ptr(), buf.len());
        }
        Ok(buf.len())
    }

    pub fn is_ready(&self) -> bool {
        self.state == SmcState::Ready
    }
}
```

**Pattern:** Immutable reference (`&self`) for read-only hardware access via MMIO

### 2.3 Mutable Borrows for State-Changing Operations

**Files:**
- [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) (lines 218-270)

```rust
impl<B: SmcRegisterBackend> Smc<B, Ready> {
    /// Initiate DMA read (mutable because it changes internal state)
    pub fn dma_read(&mut self, flash_offset: u32, dram_addr: usize, len: u32) 
        -> Result<(), SmcError> 
    {
        if self.state != SmcState::Ready {
            return Err(SmcError::ControllerNotReady);
        }
        // ... validation ...
        self.regs.write_dma_ctrl(0x1);
        self.state = SmcState::DmaInFlight;  // ← State change
        Ok(())
    }

    /// Handle DMA completion from IRQ (mutable)
    pub fn handle_dma_irq(&mut self) -> Result<SmcInterrupt, SmcError> {
        // ... decode status ...
        self.state = SmcState::Ready;  // ← State change
        Ok(decoded)
    }
}
```

**Key pattern:**
- `&mut self` required when internal `state` field changes
- Compiler prevents concurrent mutable access
- Borrow checker ensures exclusive ownership during mutation

### 2.4 Lifetime-Bounded References for Facades

**Files:**
- [peripherals/smc/device/flash.rs](target/ast10x0/peripherals/smc/device/flash.rs) (lines 175-225)

```rust
// Device facade holds a borrowed reference to the controller
pub struct SpiNorFlash<'a> {
    backend: FlashBackend<'a>,  // ← Lifetime-bounded reference
    cfg: FlashConfig,
    cs: ChipSelect,
    cmd_mode: TransferMode,
    addressing_policy: FlashAddressingPolicy,
    command_profile: FlashCommandProfile,
}

impl<'a> SpiNorFlash<'a> {
    /// Build from an initialized FMC controller
    pub fn from_fmc(fmc: &'a mut FmcReady, cfg: FlashConfig) 
        -> Result<Self, SmcError> 
    {
        Ok(Self {
            backend: FlashBackend::Fmc(fmc),  // Borrow fmc for lifetime 'a
            cfg,
            cs: ChipSelect::Cs0,
            // ...
        })
    }

    pub fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError> {
        match self.backend {
            FlashBackend::Fmc(fmc) => fmc.read(offset, buf),
            FlashBackend::Spi(spi) => spi.read(offset, buf),
        }
    }
}
```

**Pattern:**
- Lifetime parameter `'a` ties facade lifetime to controller lifetime
- Controller cannot be dropped while facade exists
- Compiler prevents use-after-free

---

## 3. Test Initialization and Access Patterns

### 3.1 Simple Smoke Test (SMC)

**Files:**
- [tests/smc/target.rs](target/ast10x0/tests/smc/target.rs)

```rust
fn run_smc_smoke_test() -> Result<(), SmcError> {
    // Create configuration
    let config = SmcConfig {
        controller_id: SmcController::Fmc,
        cs0: Some(FlashConfig {
            capacity_mb: 1,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 25,
        }),
        cs1: None,
        dma_enabled: false,
        enable_interrupts: false,
        topology: SmcTopology::BootSpi { master_idx: 0 },
    };

    // Initialize controller (type state: Uninitialized → Ready)
    let controller = unsafe { UninitSmc::new(config)? };
    let mut controller = controller.init()?;

    // Use as mutable reference
    if !controller.is_ready() {
        return Err(SmcError::HardwareError);
    }

    // PIO read
    let mut buf = [0u8; 8];
    let n = controller.read(0, &mut buf)?;
    if n != 8 {
        return Err(SmcError::HardwareError);
    }

    // Bounds checking
    let mut overflow_buf = [0u8; 8];
    match controller.read(0x000F_FFFF, &mut overflow_buf) {
        Err(SmcError::InvalidCapacity) => {},
        _ => return Err(SmcError::HardwareError),
    }

    Ok(())
}
```

### 3.2 Device Facade Test

**Files:**
- [tests/smc/target_device.rs](target/ast10x0/tests/smc/target_device.rs)

```rust
fn run_device_smoke_test() -> Result<(), SmcError> {
    let config = SmcConfig { /* ... */ };
    
    // Initialize FMC (type state transition)
    let uninit = unsafe { FmcUninit::new(config)? };
    let mut fmc = uninit.init()?;

    // Build device facade with borrowed reference
    let flash = SpiNorFlash::from_fmc(&mut fmc, FLASH_CFG)?;

    // Facade operations use borrowed reference
    let cap = flash.capacity_bytes()?;
    if cap != 1 * 1024 * 1024 {
        return Err(SmcError::HardwareError);
    }

    let _ = flash.status()?;
    
    let mut buf = [0u8; 8];
    let n = flash.read(0, &mut buf)?;
    if n != 8 {
        return Err(SmcError::HardwareError);
    }

    Ok(())
}
```

**Key pattern:** Controller and facade lifetimes are correctly bound

### 3.3 SPI Monitor Boot Test

**Files:**
- [tests/spim/test_boot_uc/target.rs](target/ast10x0/tests/spim/test_boot_uc/target.rs)

```rust
fn run_boot_uc_test() -> Result<(), &'static str> {
    // Create board (owns all peripherals)
    let mut board = unsafe { Ast1060Board::init() };
    let mut monitor = board.monitor();
    
    // Phase 1: Hold
    phase_1_hold(&mut monitor, MonitorInstance::Spim0)?;
    
    // Phase 2: Configure policy
    phase_2_configure_policy(&mut monitor, MonitorInstance::Spim0)?;
    
    // Phase 3: Release
    phase_3_release(&mut monitor, MonitorInstance::Spim0)?;
    
    // Phase 4: Verify
    phase_4_runtime_verification(&mut monitor, MonitorInstance::Spim0)?;
    
    Ok(())
}

// Phase functions take mutable reference (state changes during lock)
fn phase_1_hold(monitor: &mut dyn Monitor, instance: MonitorInstance) 
    -> Result<(), &'static str> 
{
    monitor.set_mux(instance, MuxSelect::RotControl)
        .map_err(|_| "Failed to set mux to ROT")?;
    monitor.soft_reset(instance)
        .map_err(|_| "Soft reset failed")?;
    Ok(())
}
```

**Key pattern:** Board owns peripherals, passes mutable references to functions

---

## 4. Shared Ownership Patterns

### 4.1 NO Rc/Arc Usage Found

**Search result:** Only `UnsafeCell` found in marking type `!Sync`

```rust
// From /target/ast10x0/peripherals/smc/fmc_backend.rs (lines 11, 22)
use core::cell::UnsafeCell;

pub struct FmcRegisterBackend {
    base: *const device::fmc::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>,  // ← Only use of UnsafeCell
}
```

### 4.2 Why No Shared Ownership?

1. **Single-threaded embedded context:** Only one core/task at a time
2. **Type-state enforcement:** Init order prevents multiple access patterns
3. **Lifetime bounds:** References prevent aliasing
4. **Exclusive ownership model:** One owner, one path through init/use/drop

### 4.3 PhantomData for Marker Traits

**Files:**
- [peripherals/uart/mod.rs](target/ast10x0/peripherals/uart/mod.rs) (line 105)
- [peripherals/smc/fmc_backend.rs](target/ast10x0/peripherals/smc/fmc_backend.rs) (line 22)

```rust
// Prevent Sync (single-threaded only), allow Send (can move between contexts)
_not_sync: PhantomData<UnsafeCell<()>>,
```

**Effect:**
- `Usart: !Sync` (cannot share `&Usart` between threads)
- `Usart: Send` (can move `Usart` to another context)

---

## 5. Unsafe Code and Borrow Checker Bypass Patterns

### 5.1 Raw Pointer Access to MMIO Windows

**Files:**
- [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) (lines 207-217)

```rust
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
```

**Safety contract:**
1. `flash_ptr` points to valid MMIO window (fixed by hardware design)
2. `buf` is writable and non-overlapping with MMIO
3. Length validated before access
4. `copy_nonoverlapping` ensures no internal overlap

### 5.2 Constructor Safety: Single Ownership of Hardware

**Files:**
- [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) (lines 72-92)

```rust
impl<B: SmcRegisterBackend> Smc<B, Uninitialized> {
    /// Create a new SMC controller instance from a pre-built register backend.
    ///
    /// # Safety
    /// Caller must ensure:
    /// - `backend` wraps a valid hardware register block
    /// - Only one `Smc` instance exists for this hardware controller
    pub unsafe fn new_with_backend(backend: B, config: SmcConfig) 
        -> Result<Self, SmcError> 
    {
        if config.cs0.is_none() && config.cs1.is_none() {
            return Err(SmcError::InvalidCapacity);
        }

        Ok(Self {
            regs: backend,
            controller_id: config.controller_id,
            config,
            state: SmcState::Ready,
            normal_read_ctrl: [0; 2],
            flash_window_base: [0; 2],
            _mode: PhantomData,
        })
    }
}

impl Smc<FmcRegisterBackend, Uninitialized> {
    /// Create a new FMC SMC controller instance.
    ///
    /// # Safety
    /// Caller must ensure:
    /// - No other Smc instance exists for this hardware controller
    /// - The controller's base address points to valid hardware
    pub unsafe fn new(config: SmcConfig) -> Result<Self, SmcError> {
        let base = config.controller_id.base_address() as *const _;
        // SAFETY: Caller ensures base address is valid and no other instance exists.
        let regs = unsafe { FmcRegisterBackend::new(base) };
        // SAFETY: Delegating ownership requirements to caller.
        unsafe { Self::new_with_backend(regs, config) }
    }
}
```

**Safety relies on:**
1. Single instantiation (caller ensures no duplicate instances)
2. Valid base address (caller ensures memory mapping)
3. Exclusive ownership (no concurrent access)

### 5.3 Register Backend Unsafe Perimeter

**Files:**
- [peripherals/smc/fmc_backend.rs](target/ast10x0/peripherals/smc/fmc_backend.rs) (lines 26-46)

```rust
/// Safe wrapper around SMC hardware registers
///
/// This struct consolidates all unsafe hardware access. All register operations
/// go through this single point, making it easy to audit safety invariants.
pub struct FmcRegisterBackend {
    base: *const device::fmc::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>,
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
        unsafe { &*self.base }
    }
}
```

**Pattern:** Single `regs()` method concentrates all unsafe dereferences

### 5.4 User-Mode SPI Transfer with Mode Control

**Files:**
- [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) (lines 325-398)

```rust
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

    let cs_idx = cs as usize;
    let user_base = (self.normal_read_ctrl[cs_idx] & SPI_CTRL_FREQ_MASK) | ASPEED_SPI_USER;
    let window = self.flash_window_base[cs_idx] as *mut u32;

    // Assert CS
    self.regs.write_cs_ctrl(cs, user_base | ASPEED_SPI_USER_INACTIVE);
    self.regs.write_cs_ctrl(cs, user_base);

    // SAFETY: user mode is active; the flash aperture is the hardware-defined
    // byte-stream port for SPI command traffic while user mode is held.
    unsafe {
        // Command phase — always single-wire
        let cmd_ctrl = (user_base & SPI_CTRL_IO_MODE_MASK) | mode.cmd_io_bits();
        self.regs.write_cs_ctrl(cs, cmd_ctrl);
        spi_write_data(window, cmd);

        // Address / TX payload phase
        let addr_ctrl = (user_base & SPI_CTRL_IO_MODE_MASK) | mode.addr_io_bits();
        self.regs.write_cs_ctrl(cs, addr_ctrl);
        spi_write_data(window, tx_payload);

        // RX data phase
        let data_ctrl = (user_base & SPI_CTRL_IO_MODE_MASK) | mode.data_io_bits();
        self.regs.write_cs_ctrl(cs, data_ctrl);
        spi_read_data(window as *const u32, rx);
    }

    // Deassert CS and restore normal-read configuration
    self.regs.write_cs_ctrl(cs, user_base | ASPEED_SPI_USER_INACTIVE);
    self.regs.write_cs_ctrl(cs, self.normal_read_ctrl[cs_idx]);
    Ok(())
}

// Helper functions for volatile access
unsafe fn spi_read_data(ahb_addr: *const u32, read_arr: &mut [u8]) {
    for index in 0..read_arr.len() {
        if index % 4 == 0 {
            let word = unsafe { core::ptr::read_volatile(ahb_addr.add(index / 4)) };
            read_arr[index] = (word & 0xFF) as u8;
        } else {
            read_arr[index] = unsafe { core::ptr::read_volatile(ahb_addr.cast::<u8>().add(index)) };
        }
    }
}

unsafe fn spi_write_data(ahb_addr: *mut u32, write_arr: &[u8]) {
    for index in 0..write_arr.len() {
        if index % 4 == 0 && index + 4 <= write_arr.len() {
            let word = u32::from_le_bytes([
                write_arr[index], write_arr[index+1], 
                write_arr[index+2], write_arr[index+3]
            ]);
            unsafe { core::ptr::write_volatile(ahb_addr.add(index / 4), word) };
        } else {
            unsafe { core::ptr::write_volatile(ahb_addr.cast::<u8>().add(index), write_arr[index]) };
        }
    }
}
```

**Safety guarantees:**
1. Hardware-enforced CSing prevents overlapping transfers
2. Mode control registers manage bus contention
3. Register restoration ensures known state after operation

### 5.5 Volatile Register Access for SPI Monitor Logs

**Files:**
- [peripherals/spimonitor/registers.rs](target/ast10x0/peripherals/spimonitor/registers.rs) (lines 160-190)

```rust
/// Current violation log write index (number of entries written so far).
pub fn read_log_idx_reg(&self) -> u32 {
    // SAFETY: raw offset read within the known SPIPF register block page.
    unsafe {
        let ptr = (self.base as *const u8).add(0x080) as *const u32;
        core::ptr::read_volatile(ptr)  // ← Volatile to avoid caching
    }
}

/// Base address of the violation log RAM region.
pub fn log_ram_base_addr(&self) -> usize {
    unsafe {
        let ptr = (self.base as *const u8).add(0x088) as *const u32;
        core::ptr::read_volatile(ptr) as usize
    }
}
```

**Pattern:** `read_volatile` for hardware state that changes outside of Rust control

---

## 6. Summary Table: Ownership Mechanisms

| Peripheral | Struct | Type-State | Ownership Model | Sharing | Unsafe Used |
|-----------|--------|-----------|-----------------|---------|------------|
| SMC (FMC) | `Smc<FmcRegisterBackend, Mode>` | Uninit → Ready | Type-state + exclusive | None | Constructor, read ops |
| SMC (SPI) | `Smc<SpiRegisterBackend, Mode>` | Uninit → Ready | Type-state + exclusive | None | Constructor, read ops |
| SPIM Monitor | `SpiMonitor<Mode>` | Uninit → Config → Locked | Type-state + exclusive | None | Constructor |
| UART | `Usart` | None | Direct ownership | None | Register access |
| SPI NOR Facade | `SpiNorFlash<'a>` | None | Lifetime-bounded ref | No (borrow) | None |

---

## 7. Key Design Principles

### 7.1 Type-State Prevents Misuse

```rust
// This will NOT compile:
let uninit = unsafe { UninitSmc::new(config)? };
uninit.read(0, &mut buf)?;  // ← ERROR: method not available on Uninitialized

// Must transition first:
let ready = uninit.init()?;
ready.read(0, &mut buf)?;   // ← OK
```

### 7.2 Lifetime Bounds Prevent Use-After-Free

```rust
let mut fmc = unsafe { FmcUninit::new(config)? }.init()?;

{
    let flash = SpiNorFlash::from_fmc(&mut fmc, cfg)?;
    // fmc is borrowed here; cannot drop it
    flash.read(0, &mut buf)?;
} // flash dropped; fmc borrow released

// fmc can now be dropped or reused
drop(fmc);  // OK
```

### 7.3 Single Ownership Model

- One active peripheral instance per hardware block
- Type-state prevents concurrent access patterns
- Compiler enforces exclusive access via `&mut self`
- No need for Arc/Mutex in single-threaded context

### 7.4 Unsafe Consolidation

- Register backends (`FmcRegisterBackend`, `SpiMonitorRegisters`) centralize unsafe
- Single `regs()` method is the audit point
- Constructors have explicit safety contracts
- Operations on initialized peripherals are safe

---

## 8. Testing Infrastructure

### 8.1 Test Initialization Pattern

**Files:** All in `/target/ast10x0/tests/`

```rust
// Common test pattern:
1. Create config (compile-time or computed)
2. unsafe { Peripheral::new(config)? }
3. peripheral.init()?
4. Run test operations
5. Return Result<(), ErrorType>
```

### 8.2 Exit Codes

Tests use cortex_m_semihosting for exit:
```rust
use cortex_m_semihosting::debug::{EXIT_FAILURE, EXIT_SUCCESS, exit};

fn main() -> ! {
    let status = match run_test() {
        Ok(()) => EXIT_SUCCESS,
        Err(_) => EXIT_FAILURE,
    };
    exit(status);
}
```

---

## 9. File Location Quick Reference

| Purpose | File | Key Lines |
|---------|------|-----------|
| SMC Type Definition | [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) | 50-65 |
| Register Backend | [peripherals/smc/fmc_backend.rs](target/ast10x0/peripherals/smc/fmc_backend.rs) | 11-46 |
| Type-State Markers | [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) | 50-57 |
| UART Simple Pattern | [peripherals/uart/mod.rs](target/ast10x0/peripherals/uart/mod.rs) | 99-107 |
| SPIM Monitor | [peripherals/spimonitor/controller.rs](target/ast10x0/peripherals/spimonitor/controller.rs) | 19-26 |
| Device Facade | [peripherals/smc/device/flash.rs](target/ast10x0/peripherals/smc/device/flash.rs) | 175-225 |
| Constructor Safety | [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) | 72-92 |
| MMIO Read | [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) | 207-217 |
| User-Mode SPI | [peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs) | 325-398 |
| Test Example | [tests/smc/target.rs](target/ast10x0/tests/smc/target.rs) | 31-49 |

---

## 10. Conclusion

The AST10x0 peripheral ownership model is a **minimal, safe-by-construction design** that:

1. **Enforces hardware state at compile time** via type-state markers
2. **Prevents misuse** by making invalid API sequences impossible to call
3. **Avoids shared ownership** because the single-threaded embedded context doesn't require it
4. **Concentrates unsafe code** in register backends and MMIO access points
5. **Uses lifetime bounds** to prevent use-after-free
6. **Achieves zero-cost abstractions** through PhantomData and inlining

This approach is suitable for embedded systems where:
- Single-threaded execution is guaranteed
- Hardware must be accessed in specific orders
- Compile-time guarantees are valuable for safety-critical code
- Runtime overhead must be minimized
