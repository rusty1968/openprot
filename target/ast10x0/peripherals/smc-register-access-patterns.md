# SMC/FMC Register Access Patterns

A practical guide to safely and idiomatically accessing Static Memory Controller (SMC) / Flash Memory Controller (FMC) registers using the `ast1060-pac` crate.

Based on real-world patterns from [aspeed-rust SPI driver](https://github.com/OpenPRoT/aspeed-rust/tree/main/src/spi).

## Overview

The AST1060 PAC provides type-safe register access through the `RegisterBlock` struct. Registers are accessed via methods named by their **hexadecimal offset** (e.g., `fmc000()` = offset 0x00, `fmc080()` = offset 0x80).

Three fundamental patterns enable safe manipulation:
- **Read**: Query current register state
- **Write**: Set register to a computed value
- **Modify**: Read-modify-write atomic operations

## Register Naming Convention

FMC registers are organized by their memory offset:

| Offset | Method | Purpose |
|--------|--------|---------|
| 0x000  | `fmc000()` | Configuration register |
| 0x004  | `fmc004()` | 4-byte address mode / address width |
| 0x008  | `fmc008()` | DMA status |
| 0x010  | `fmc010()` | CS0 control register |
| 0x014  | `fmc014()` | CS1 control register |
| 0x030  | `fmc030()` | CS0 memory segment mapping |
| 0x034  | `fmc034()` | CS1 memory segment mapping |
| 0x06C  | `fmc06c()` | SPI I/O mode selection |
| 0x080  | `fmc080()` | DMA control register |
| 0x084  | `fmc084()` | DMA flash/DRAM address |
| 0x088  | `fmc088()` | DMA size (flash window / DRAM) |
| 0x090  | `fmc090()` | DMA checksum (CRC) |
| 0x094  | `fmc094()` | CS0 timing calibration status |
| 0x098  | `fmc098()` | CS1 timing calibration status |

## Core Patterns

### Pattern 1: Simple Read

Query the current value of a register.

```rust
let value = self.regs.fmc080().read().bits();
if value & DMA_REQUEST != 0 {
    // DMA request is pending
}
```

**Advantages:**
- Non-blocking snapshot of hardware state
- Can be called multiple times safely
- No side effects

### Pattern 2: Simple Write

Set a register to a computed value, replacing all bits.

```rust
// Set DMA control to trigger a transfer
self.regs.fmc080().write(|w| unsafe { w.bits(0x1) });

// Clear all segment mappings
self.regs.fmc030().write(|w| unsafe { w.bits(0) });
```

**Safety note:** The `unsafe` block is required because the PAC requires explicit acknowledgment that the caller is providing a valid bit pattern. Compute the value carefully before writing.

### Pattern 3: Modify (Read-Modify-Write)

Atomically read current bits, apply a function, and write back.

```rust
// Enable CS0's 4-byte address mode (bit 0)
self.regs.fmc004().modify(|r, w| unsafe {
    let current = r.bits();
    w.bits(current | (1 << 0))
});
```

**Advantages:**
- Preserves other bits in the register
- Hardware typically ensures atomicity
- Clearer intent than manual read+write

**Critical pattern:** Always use `modify()` when you only want to change specific bits:

```rust
// ✓ GOOD: Sets only the CS0 4-byte-mode bit, preserves others
self.regs.fmc004().modify(|r, w| unsafe {
    w.bits(r.bits() | (SPI_CTRL_CEX_4BYTE_MODE_SET << 0))
});

// ✗ WRONG: Would clear all other bits!
self.regs.fmc004().write(|w| unsafe {
    w.bits(SPI_CTRL_CEX_4BYTE_MODE_SET << 0)
});
```

### Pattern 4: Bit Field Extraction

Extract multi-bit fields using masks and shifts.

```rust
// Extract bits [23:20] as clock divisor (4 bits)
let clk_div = (register_value >> 20) & 0xF;

// Extract bits [31:28] as setup time (4 bits)
let setup_time = (register_value >> 28) & 0xF;
```

## CS-Specific Register Pairs

Many registers have paired variants for CS0 and CS1. Use conditional logic or macros to handle both:

### Manual Dispatch Example

```rust
pub fn write_cs_ctrl(&self, cs: usize, value: u32) {
    match cs {
        0 => self.regs.fmc010().write(|w| unsafe { w.bits(value) }),
        1 => self.regs.fmc014().write(|w| unsafe { w.bits(value) }),
        _ => {} // Invalid CS
    }
}

pub fn read_cs_ctrl(&self, cs: usize) -> u32 {
    match cs {
        0 => self.regs.fmc010().read().bits(),
        1 => self.regs.fmc014().read().bits(),
        _ => 0, // Invalid CS
    }
}
```

### Macro-Based Dispatch

For performance-critical code, use macros to zero cost abstractions:

```rust
macro_rules! cs_ctrl_write {
    ($this:expr, $cs:expr, $value:expr) => {{
        match $cs {
            0 => $this.regs.fmc010().write(|w| unsafe { w.bits($value) }),
            1 => $this.regs.fmc014().write(|w| unsafe { w.bits($value) }),
            _ => {}
        }
    }};
}

// Usage:
cs_ctrl_write!(self, 0, new_control_value);
```

## Common Workflow: Configure & Enable

Typical initialization sequence:

```rust
pub fn init(&mut self) -> Result<(), Error> {
    // Step 1: Read current configuration
    let mut config = self.regs.fmc000().read().bits();
    
    // Step 2: Compute new configuration
    config |= 1 << 16; // Enable CS0 write
    config |= 0x2 << 0; // Set flash type to SPI
    
    // Step 3: Write computed value
    self.regs.fmc000().write(|w| unsafe { w.bits(config) });
    
    Ok(())
}
```

Or more idiomatically using `modify()`:

```rust
pub fn init(&mut self) -> Result<(), Error> {
    // Modify current config in one atomic operation
    self.regs.fmc000().modify(|r, w| unsafe {
        let current = r.bits();
        w.bits(
            current
                | (1 << 16)  // Enable CS0 write
                | (0x2 << 0) // Set flash type to SPI
        )
    });
    
    Ok(())
}
```

## DMA Operations

Complex multi-register sequence for DMA transfers:

```rust
pub fn dma_read(&mut self, flash_addr: u32, dram_addr: u32, len: u32) {
    // 1. Request DMA grant
    if self.regs.fmc080().read().bits() & DMA_REQUEST != 0 {
        while self.regs.fmc080().read().bits() & DMA_GRANT == 0 {}
    }
    
    // 2. Set up source address (flash)
    self.regs.fmc084().write(|w| unsafe { w.bits(flash_addr) });
    
    // 3. Set up length (note: hardware uses length-1 encoding)
    self.regs.fmc088().write(|w| unsafe { w.bits(len - 1) });
    
    // 4. Trigger DMA
    self.regs.fmc080().write(|w| unsafe { w.bits(DMA_REQUEST) });
    
    // 5. Wait for completion
    while self.regs.fmc008().read().bits() & DMA_STATUS == 0 {}
    
    // 6. Read checksum (optional)
    let checksum = self.regs.fmc090().read().bits();
}
```

**Key points:**
- Some registers use special encodings (e.g., length - 1)
- Order of operations matters (source before length, control last)
- Account for hardware-specific delays or synchronization needs

## Timing & Calibration

Register access pattern for timing sweep (read-intensive):

```rust
pub fn run_timing_sweep(&mut self, cs: usize) -> bool {
    for hcycle in 0..=5 {
        for delay_ns in 0..=0xf {
            // Compute new timing value
            let timing = compute_timing(hcycle, delay_ns);
            
            // Write timing to calibration register
            match cs {
                0 => self.regs.fmc094().write(|w| unsafe { w.bits(timing) }),
                1 => self.regs.fmc098().write(|w| unsafe { w.bits(timing) }),
                _ => continue,
            }
            
            // Read back result (e.g., checksum)
            let result = self.regs.fmc090().read().bits();
            
            if validate_result(result) {
                return true;
            }
        }
    }
    false
}
```

## Safety Considerations

### The `unsafe` Block

The PAC requires `unsafe { w.bits(...) }` because it cannot verify at compile time that a computed u32 is a valid register value. The safety burden transfers to the caller:

```rust
// ✓ Safe: Caller computes bits from logical constraints
self.regs.fmc080().write(|w| unsafe {
    w.bits(DMA_REQUEST | DMA_MODE_READ)  // Hand-verified bit constants
});

// ⚠️ Risky: Arbitrary u32 could cause undefined behavior
let user_value = read_from_user_input();
self.regs.fmc080().write(|w| unsafe {
    w.bits(user_value)  // WRONG: untrusted value
});
```

### Multi-Register Consistency

Some operations require multiple registers to be consistent. Minimize the window between reads and writes:

```rust
// ✗ WRONG: Long gap between read and write; hardware could change
let current = self.regs.fmc080().read().bits();
do_something_else();  // Work that could interfere
self.regs.fmc080().write(|w| unsafe { w.bits(current | NEW_FLAG) });

// ✓ BETTER: Use modify() for atomic read-modify-write
self.regs.fmc080().modify(|r, w| unsafe {
    w.bits(r.bits() | NEW_FLAG)
});
```

### Ownership & Concurrency

Store a reference to the `RegisterBlock`:

```rust
pub struct SmcController {
    regs: &'static ast1060_pac::fmc::RegisterBlock,
    // ... other fields
}
```

The `&'static` lifetime indicates:
- Single, non-exclusive access to hardware
- No need for `Mutex` or `RefCell`
- Caller responsible for single-threaded access (or proper synchronization in multi-threaded contexts)

To prevent accidental concurrent access, add `PhantomData<UnsafeCell<()>>`:

```rust
pub struct SmcController {
    regs: &'static ast1060_pac::fmc::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>,  // Prevent Sync
}
```

This ensures the type is `Send` but not `Sync`, flagging shared access as a compile error without additional synchronization.

## Helper Abstractions

Build safe wrappers around common patterns:

```rust
// Segment manipulation helper
pub fn encode_segment(start: u32, end: u32) -> u32 {
    // Convert MB addresses to 4KB units
    let start_4k = start >> 12;
    let end_4k = end >> 12;
    (end_4k << 16) | start_4k
}

// CS-agnostic control register set
pub fn set_cs_mode(&self, cs: usize, mode: u32) {
    let mask = mode & 0xFF;
    match cs {
        0 => self.regs.fmc010().write(|w| unsafe { w.bits(mask) }),
        1 => self.regs.fmc014().write(|w| unsafe { w.bits(mask) }),
        _ => {}
    }
}
```

These helpers:
- Centralize register bit calculations
- Improve readability and reduce errors
- Enable re-use across multiple operations

## Debugging

Use read-only snapshots to inspect hardware state without side effects:

```rust
// Safe to call in debug output without affecting hardware
let config = self.regs.fmc000().read().bits();
let dma_status = self.regs.fmc008().read().bits();
let dma_ctrl = self.regs.fmc080().read().bits();

println!("Config: {:#010x}", config);
println!("DMA Status: {:#010x}", dma_status);
println!("DMA Control: {:#010x}", dma_ctrl);
```

## Summary

| Operation | Pattern | Use Case |
|-----------|---------|----------|
| Query state | `reg.read().bits()` | Polling, status checks |
| Set value | `reg.write(\|w\| unsafe { w.bits(...) })` | One-time initialization, complete overwrites |
| Update bits | `reg.modify(\|r, w\| unsafe { w.bits(r.bits() \| FLAG) })` | Enabling features, setting flags, preserving other bits |
| CS dispatch | `match cs { 0 => fmc010(), 1 => fmc014(), _ => {} }` | Multi-CS operations |

## References

- **SPI Driver Reference**: [`aspeed-rust/src/spi`](https://github.com/OpenPRoT/aspeed-rust/tree/main/src/spi)
- **AST1060 PAC**: [`ast1060-pac`](https://github.com/OpenPRoT/ast1060-pac)
- **Embedded HAL Patterns**: [`embedded-hal` (GitHub)](https://github.com/rust-embedded/embedded-hal)
