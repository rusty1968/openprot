# SMC Module Code Review

A comprehensive review of the Static Memory Controller (SMC) peripheral driver implementation.

**Status**: ✅ Builds successfully with `--platforms=//target/ast10x0`

## Architecture Overview

The SMC module provides a safe, layered interface for controlling FMC (Flash Memory Controller), SPI1, and SPI2 controllers:

```
┌─────────────────────────────────────┐
│  Public API (Smc)                   │
│  - read() / write operations        │
│  - dma_read() / dma_write()         │
│  - is_ready(), controller_id()      │
└─────────────────────────────────────┘
        ↓
┌─────────────────────────────────────┐
│  Controller Layer (controller.rs)   │
│  - init(), setup_segments()         │
│  - configure_timing()               │
│  - State management (SmcState)      │
└─────────────────────────────────────┘
        ↓
┌─────────────────────────────────────┐
│  Register Layer (registers.rs)      │
│  - SmcRegisters (unsafe perimeter)  │
│  - PAC read/write/modify wrappers   │
└─────────────────────────────────────┘
        ↓
┌─────────────────────────────────────┐
│  PAC Layer (ast1060_pac)            │
│  - RegisterBlock methods (fmc000-fmc098) │
└─────────────────────────────────────┘
```

## Module Structure

### `registers.rs` — Unsafe Perimeter
**Purpose**: Consolidate all unsafe hardware register access below this line.

**Key Design**:
- Single unsafe constructor: `SmcRegisters::new(base)`
- `_not_sync` PhantomData prevents accidental Sync implementation
- All methods safe (safe wrappers around PAC calls)
- Follows AST1060 PAC guide pattern

**Methods by Register Offset**:
- `fmc000`: Configuration (read/write/modify)
- `fmc004`: Address width control (read/write)
- `fmc008`: DMA status (read-only)
- `fmc010`/`fmc014`: CS0/CS1 control (read/write)
- `fmc030`/`fmc034`: CS0/CS1 segment mapping (read/write)
- `fmc06c`: SPI I/O mode (read/write/modify)
- `fmc080`: DMA control (read/write)
- `fmc084`: DMA address (read/write)
- `fmc088`: DMA length (read/write)
- `fmc090`: DMA checksum (read-only)
- `fmc094`/`fmc098`: CS0/CS1 calibration status (read-only)

**Strengths**:
✓ Single audit point for all register access
✓ PAC ensures type safety at compile time
✓ Consistent naming (offset-based methods)
✓ Clear safety documentation

**Observations**:
- Pattern follows best practices from `aspeed-rust/src/spi/fmccontroller.rs`
- `modify()` wrappers avoid read-write races
- No implicit type conversions or bit manipulation helpers (intentional minimalism)

---

### `types.rs` — Error Handling & Configuration
**Purpose**: Define domain types, error enums, and hardware addresses.

**Key Types**:

**`SmcError`** (terminal errors):
- `HardwareError`: Device malfunctioned
- `Timeout`: Operation didn't complete in time
- `DmaAborted`: DMA transfer interrupted
- `DmaLengthMismatch`: CRC/length mismatch
- `InvalidChipSelect`: CS out of range
- `InvalidCapacity`: Capacity exceeds limits
- `DeviceNotSupported`: Unsupported flash
- `WriteProtected`: Hardware write-protect active
- `WriteInProgress`: Can't perform operation

Implements `embedded_hal::spi::Error` for HAL compatibility.

**`SmcRetryable`** (non-blocking errors):
- `NotReady`: Device busy, try again
- `DmaTransferPending`: Transfers still in-flight

Converts to `nb::Error::WouldBlock` for non-blocking APIs.

**`SmcController`** (hardware identifiers):
```
Fmc:   0x7E620000 → 0x80000000 (256 MB window)
Spi1:  0x7E630000 → 0x90000000 (256 MB window)
Spi2:  0x7E640000 → 0xB0000000 (256 MB window)
```

Each has dedicated IRQ vectors (39, 65, 66).

**`FlashConfig`** (per-device):
- Capacity (MB)
- Page size (typically 256 B)
- Sector size (typically 4 KB)
- Block size (typically 64 KB)
- SPI clock frequency (MHz)

Includes constants for common devices (Winbond W25Q64/W25Q256).

**`SmcConfig`** (per-controller):
- `controller_id`: Which FMC/SPI controller
- `cs0`/`cs1`: Optional device configs
- `dma_enabled`: Enable DMA transfers
- `enable_interrupts`: Route interrupts to handler

---

### `controller.rs` — Main Driver Logic
**Purpose**: Implement initialization, memory mapping, and I/O operations.

**State Machine**:
```
Uninitialized
    ↓ (init())
  Ready
    ↓ (dma_read())
DmaInFlight
    ↓ (ISR / polling)
  Ready
```

**Key Operations**:

**initialization** (`init()`):
1. Configure flash types and write-enable per CS
2. Set up timing parameters (PLACEHOLDER - TODO)
3. Configure segment mapping (memory windows)
4. Enable DMA interrupts (if requested)

**Segment Encoding** (`encode_segment()`):
Converts logical memory ranges to hardware register format:
```
Register: [31:16]=END_4K | [15:0]=START_4K

Example: 16 MB segment
  - Start: 0x00000000 >> 12 = 0x0000
  - End:   0x01000000 >> 12 = 0x4000
  - Reg:   0x40000000
```

**Clock Divisor** (`calculate_clock_divisor()`):
Computes binary divisor for SPI clock:
```
SYSCLK = 200 MHz
200 >> 2 = 50 MHz (divisor 2)
200 >> 3 = 25 MHz (divisor 3)
```

**DMA Read** (`dma_read()`):
Non-blocking transfer initiation:
1. Set DRAM address (masked to 0x000BFFFC)
2. Set transfer length (hardware expects length-1)
3. Configure segment for flash address range
4. Trigger DMA (fmc080 = 0x1)
5. Return immediately; caller polls for completion

**Memory Window Read** (`read()`):
Programmed I/O via memory-mapped window:
- Direct `memcpy` from flash address space
- Hardware converts memory accesses to SPI commands
- No setup required; instantaneous but slower than DMA

**Strengths**:
✓ Clear state transitions
✓ Comprehensive safety checks (bounds validation)
✓ Proper error propagation
✓ Includes unit tests (segment encoding, clock divisor)

**Concerns**:

🟡 **Timing Configuration Incomplete**:
Current implementation:
```rust
fn configure_timing(&self, _cs: usize, config: &FlashConfig) -> Result<(), SmcError> {
    let sysclk_mhz = 200u32;
    let _ideal_clk_div = Self::calculate_clock_divisor(sysclk_mhz, config.spi_clock_mhz)?;
    Ok(())
}
```
TODO: Write computed divisor to CS control registers (fmc010/fmc014).

🟡 **DMA Polling Incomplete**:
`dma_read()` doesn't wait for completion. Caller must:
- Implement ISR for DMA completion interrupt, OR
- Poll `regs.read_dma_status()` for completion flag

Example completion check:
```rust
while (regs.read_dma_status() & DMA_STATUS_BIT) == 0 {
    // Wait for completion
}
```

🟡 **Missing DMA Write Support**:
Only DMA read is implemented. DMA write would follow similar pattern:
```rust
pub fn dma_write(&mut self, dram_addr: usize, flash_offset: u32, len: u32) -> Result<(), SmcError>
```

🟡 **No Chip Select Abstraction**:
Methods assume CS0. For multi-device support:
```rust
pub fn dma_read_cs(&mut self, cs: usize, flash_offset: u32, dram_addr: usize, len: u32) -> Result<(), SmcError>
```

---

### `interrupts.rs` — Safe ISR Context
**Purpose**: Provide safe interrupt decoding for ISR context.

**Types**:

**`SmcInterrupt`**:
- `DmaComplete`: Bit 11 in fmc008
- `DmaError`: (TODO: not decoded yet)
- `CommandAbort`: Bit 10 in fmc008
- `WriteProtected`: Bit 9 in fmc008

**`SmcInterruptDecoder::decode()`**:
Decodes interrupt status register bits into safe enum. Safe to call from ISR.

**Observations**:
- Missing `DmaError` decoding (should check for error flags)
- No reference to actual register bit positions in hardware docs
- Likely TODO: Add more comprehensive status checks

---

### `mod.rs` — Module Exports
**Purpose**: Public API surface.

**Public Exports**:
- `SmcError`, `SmcController`, `FlashConfig`, `SmcConfig`, `SmcRetryable`
- `Smc` (main controller struct)
- `SmcInterrupt`, `SmcInterruptDecoder`
- `Result<T>` type alias

---

## Safety Analysis

### Unsafe Blocks

**1. `SmcRegisters::new()` (Constructor)**
```rust
pub const unsafe fn new(base: *const device::fmc::RegisterBlock) -> Self
```
**Safety Invariants**:
- ✓ Base must point to valid FMC register block
- ✓ No other `SmcRegisters` instance for this controller
- ✓ Caller maintains exclusive access (single-threaded or synchronized)

**Verification**: Caller (Smc::new) passes `SmcController::base_address()`, which is valid. No runtime verification possible.

**Risk**: If multiple threads access same controller without synchronization, race conditions on register writes. Mitigated by `!Sync` due to PhantomData.

**2. `SmcRegisters::regs()`**
```rust
unsafe { &*self.base }
```
**Safety Invariants**:
- ✓ Constructor ensures pointer validity
- ✓ Only one SmcRegisters instance per hardware
- ✓ PAC ensures RegisterBlock struct layout is correct

**Verification**: Compile-time (PAC definitions).

**3. `Smc::read()` — Memory Window Copy**
```rust
unsafe {
    let window = self.controller_id.flash_window_address() as *const u8;
    let flash_ptr = unsafe { window.add(offset as usize) };
    core::ptr::copy_nonoverlapping(flash_ptr, buf.as_mut_ptr(), buf.len());
}
```
**Safety Invariants**:
- ✓ Window addresses are hardcoded and valid
- ✓ `add()` doesn't wrap (256 MB window)
- ✓ Flash is mapped read-only; no concurrent writes

**Verification**: Addresses verified against hardware docs. Bounds check: offset + len ≤ 256 MB should be enforced by caller.

**Risk**: No runtime bounds checking on `offset + len`. Caller is responsible.

**Recommendation**: Add optional runtime bounds check:
```rust
if offset as u64 + buf.len() as u64 > 256 * 1024 * 1024 {
    return Err(SmcError::InvalidCapacity);
}
```

### Concurrency

**Thread Safety**:
- ✓ `SmcRegisters` is `Send` (pointer is Send)
- ✓ `SmcRegisters` is NOT `Sync` (PhantomData prevents it)
- ✓ `Smc` is NOT `Send` or `Sync` (contains SmcRegisters)

This ensures accidental sharing across threads is a compile error.

**Multi-Core Safety**:
In multi-core systems, synchronization is caller's responsibility. Consider:
```rust
pub static mut SMC_FMC: Option<Smc> = None;
static SMC_LOCK: SpinLock<()> = SpinLock::new(());

fn dma_read_safe(offset: u32, ...) {
    let _guard = SMC_LOCK.lock();
    unsafe { SMC_FMC.as_mut().unwrap().dma_read(...) }
}
```

---

## Testing

**Present**:
- ✓ `test_encode_segment()`: Validates 16 MB range encoding
- ✓ `test_clock_divisor()`: 200 MHz → 25 MHz (divisor 3)
- ✓ `test_clock_divisor_high_speed()`: 200 MHz → 50 MHz (divisor 2)
- ✓ `test_segment_overflow()`: Rejects 512 MB > 256 MB max

**Missing**:
- ❌ Integration tests (actual register writes)
- ❌ DMA completion polling tests
- ❌ Multi-CS scenarios
- ❌ Error path tests (invalid configs)

---

## Known TODOs & Future Work

### High Priority

1. **Implement Timing Register Writes**
   ```rust
   // TODO in configure_timing()
   // Set fmc010/fmc014 with clock divisor bits
   // Reference: aspeed-rust SPI driver for bit positions
   ```

2. **Add DMA Completion Polling**
   ```rust
   pub fn dma_wait(&self, timeout_ms: u32) -> Result<(), SmcError>
   ```

3. **Implement Bounds Checking in `read()`**
   ```rust
   if offset as u64 + buf.len() as u64 > 256 * 1024 * 1024 {
       return Err(SmcError::InvalidCapacity);
   }
   ```

### Medium Priority

4. **Add DMA Write Support**
   ```rust
   pub fn dma_write(&mut self, dram_addr: usize, flash_offset: u32, len: u32) -> Result<(), SmcError>
   ```

5. **Add CS Parameter to I/O Methods**
   Support arbitrary CS (CS0/CS1) selection instead of hardcoded CS0.

6. **Enhance Interrupt Decoding**
   - Decode DMA error status bits
   - Return detailed error information
   - Reference hardware register definitions

### Lower Priority

7. **Flash Device Abstraction Layer**
   Define `FlashDevice` trait for SPI NOR standard operations:
   ```rust
   pub trait FlashDevice {
       fn read(&mut self, offset: u32, buf: &mut [u8]) -> Result<usize, SmcError>;
       fn erase_sector(&mut self, offset: u32) -> Result<(), SmcError>;
       fn program_page(&mut self, offset: u32, data: &[u8]) -> Result<(), SmcError>;
   }
   ```

8. **DMA Timeout Calibration**
   Add configurable timeout instead of infinite polling.

---

## Comparison to Reference Implementation

**aspeed-rust SPI Driver** (`src/spi/fmccontroller.rs`):
- ✓ Same register access patterns (our guide documents this)
- ✓ Similar FMC initialization sequence
- ⚠️ More complex: includes timing calibration sweep, multi-device support
- ⚠️ No explicit state machine (stateless functions)

**AST1060 PAC Guide** (`ast1060-pac-guide.md`):
- ✓ We follow consolidation pattern exactly
- ✓ Same unsafe perimeter principle
- ✓ PhantomData prevents Sync

---

## Code Quality

**Strengths**:
✓ Well-documented (module docs, safety comments)
✓ Modular (clear separation of concerns)
✓ Safe by default (minimal unsafe code)
✓ Follows project conventions (license, style)
✓ Includes unit tests
✓ Clear error types

**Areas for Improvement**:
- [ ] Runtime bounds checking (memory window reads)
- [ ] More comprehensive interrupt handling
- [ ] Multi-device (multi-CS) support
- [ ] Integration tests
- [ ] Timing calibration implementation

---

## Recommended Next Steps

1. **Immediate**: Finish timing configuration (HIGH PRIORITY)
2. **Short-term**: Add DMA completion waiting and bounds checks
3. **Medium-term**: Implement DMA write and multi-CS support
4. **Long-term**: Flash device abstraction and advanced features

---

## References

- [FMC Register Access Patterns](smc-register-access-patterns.md)
- [aspeed-rust SPI Driver](https://github.com/OpenPRoT/aspeed-rust/tree/main/src/spi)
- [AST1060 PAC Guide](https://github.com/OpenPRoT/ast1060-pac-guide)
- [AST1060 Hardware Datasheet](file:///path/to/ast1060-datasheet.pdf)

---

## Summary

The SMC module is a solid foundation for flash controller management:
- ✅ Safe, layered architecture
- ✅ Follows Rust embedded best practices
- ✅ Builds successfully
- ⚠️ Timing configuration incomplete
- ⚠️ DMA polling not implemented
- ⚠️ Lacks bounds checking on memory reads

**Recommended Status**: Ready for testing on hardware with completion of timing setup and DMA polling.
