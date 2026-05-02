## SMC Controller Test Plan

Reverse-engineered from `aspeed-rust/src/spi/spitest.rs` and aligned with the refactored Pure Helpers architecture.

---

## 1. Test Phases

### Phase 1: Pure Helper Logic (Host-based)

**Target:** `Bazel test //target/ast10x0/peripherals:smc_helpers_test`

Run tests locally on host without hardware. Coverage:

| Helper | Test Cases | Status |
|--------|-----------|--------|
| `encode_segment` | 16 MB boundary, 256 MB overflow, 4 KB alignment | ✓ Implemented |
| `calculate_clock_divisor` | 25 MHz, 50 MHz, 0 MHz error, max divisor (7) | ✓ Implemented |
| `flash_capacity_bytes` | Capacity overflow (capacity_mb * 1024 * 1024) | ✓ Implemented |
| `total_capacity_bytes` | Multi-CS overflow, 256 MB window limit | ✓ Implemented |
| `validate_mapped_range` | Exact fit, arithmetic overflow, out-of-bounds | ✓ Implemented |
| `validate_dma_read` | Valid request, zero length, misaligned DRAM, masked bits, flash range overflow | ✓ Implemented |

**Execution:** Pre-build, no hardware needed. Fast feedback loop.

---

### Phase 2: Integration (QEMU AST1030)

**Target:** Emulated hardware behavior without real flash.

#### 2a. Controller Initialization

| Test | Expected | Verification |
|------|----------|--------------|
| FMC init with single CS | Ready state | State transitions to Ready |
| SPI0 init with single CS | Ready state | State transitions to Ready |
| SPI1 init with dual CS | Ready state (CS0+CS1 configured) | Segment registers written |
| Init failure: no CS configured | Error::InvalidCapacity | Rejected at constructor |
| Timing config applied | SYSCLK=200MHz → divisor computed | Timing register has divisor |
| Segment mapping (FMC) | CS0 @ 0x80000000 (16 MB), CS1 @ 0x80100000 (16 MB) | Segment regs: 0x00004000, 0x00004004 |
| Segment mapping (SPI0) | CS0 @ 0x90000000 (64 MB) | Segment reg: 0x00010000 |
| Segment mapping (SPI1) | CS0 @ 0xB0000000, CS1 @ 0xB4000000 | Both segment regs written |

#### 2b. PIO Read

| Test | Expected | Verification |
|------|----------|--------------|
| Read 256 bytes @ 0x0000 | Data matches memory window | Read buffer matches MMIO source |
| Read 256 bytes @ 0x1000 | Data at offset stored correctly | Address translation works |
| Read 4096 bytes (page boundary) | All data consistent | Larger transfers work |
| Read when not Ready | Error::HardwareError | State check enforced |
| Read with overflow (u32::MAX offset + len) | Error::InvalidCapacity | Bounds validation caught |
| Unaligned read (e.g., 3 bytes) | OK (no alignment requirement) | Byte-level access works |

#### 2c. DMA Read (Non-blocking)

| Test | Expected | Verification |
|------|----------|--------------|
| DMA read 512 bytes | State → DmaInFlight | Status register bit set |
| DMA DRAM alignment check (4-byte) | Misaligned rejected | Error::InvalidCapacity |
| DMA DRAM mask check (0x000BFFFC) | Out-of-range rejected | Error::InvalidCapacity |
| DMA zero-length | Rejected | Error::InvalidCapacity |
| DMA flash range overflow | Rejected | Error::InvalidCapacity |
| DMA state check (not Ready) | Error::HardwareError | State machine enforced |

#### 2d. Hardware Register State Observations

| Scenario | Register | Expected | How to verify |
|----------|----------|----------|----------------|
| After init | FMC000 (CONF) | 0x0203 (2 CS, SPI type) | Read via astdebug or bare metal |
| Segment setup | FMC030 (CS0 SEG) | Depends on capacity | Visual inspection |
| DMA triggered | FMC080 (DMA CTRL) | Bit [0]=1 (REQUEST) | ISR triggered (if enabled) |
| Timing written | FMC010 (CS0 CTRL) | Divisor bits set | Clock scaling applied in next transfer |

---

### Phase 3: Device Layer (Target AST1030/AST1060)

**Target:** Real flash devices with read/write cycles.

#### 3a. JEDEC ID Detection

| Test | Device | Expected | Verification |
|------|--------|----------|--------------|
| FMC CS0 read ID | Winbond W25Q256 (32 MB) | ID = 0x19...60 | Manufacturer code present |
| FMC CS1 read ID | Winbond W25Q64 (8 MB) | ID = 0x17...40 | Different device detected |
| SPI0 CS0 read ID | Macronix MX25L80006E (8 MB) | ID format valid | Command handling validated |
| SPI1 dual CS read ID | Both devices detected | Per-CS ID distinct | Multi-CS routing verified |

#### 3b. Read/Write Cycle (with verification)

| Test | Controller | Address | Size | Write? | Expected | Verification |
|------|-----------|---------|------|--------|----------|------|
| Simple read | FMC | 0x0000 | 256 B | No | Factory data or erase-state | Memory window works |
| Erase → Write → Read | FMC | 0x1000 | 256 B | Yes | Pattern matches written | Full cycle works |
| Multiple offsets | SPI0 | 0x0000, 0x100, 0x200, 0x1000, 0x2000 | 256 B ea | Yes | All offsets verify | Addressing spans window |
| Small transfer (64 B) | SPI1 | 0x0000 | 64 B | Yes | Read matches written | Sub-page sizes work |
| Page boundary (4096 B) | FMC | 0x1000 | 4096 B | Yes | Full sector matches | Large transfers align |
| DMA read (128+ bytes) | All | Various | 512 B | No | DMA path used instead of PIO | Register state shows DMA_CTRL set |

#### 3c. Error Conditions

| Test | Scenario | Expected | Verification |
|------|----------|----------|--------------|
| Write-protected region | Attempt erase @ 0x0FFF0000 | Error from device | Programming fails (Device::WriteProtected) |
| Timeout (firmware bug) | Send stalled command | Error::Timeout after N ms | Watchdog or timeout counter triggered |
| DMA collision | Multiple DMA ops queued | Error::Busy or Error::DmaInFlight | Second request rejected |
| Invalid CS selection | Attempt CS=2 on CS1-only device | Error::InvalidChipSelect | Range checking enforced |

---

### Phase 4: System Integration

**Target:** Full firmware update scenario.

#### 4a. Multi-Controller Scenario

| Test | Scenario | Expected |
|------|----------|----------|
| FMC active boot + SPI1 secondary | Boot from FMC, program backup on SPI1 | Both controllers initialized independently |
| Dual-image failover | Erase FMC, restore from SPI1 | Atomic boot image switch |
| DMA high-throughput | Copy 1 MB FMC → DRAM via DMA | Non-blocking transfers complete |

#### 4b. Block Device Abstraction

| Test | Operation | Input | Expected | Verification |
|------|-----------|-------|----------|------|
| Init from JEDEC | Read and detect | Winbond ID | Block device ready | Geometry correct |
| Block erase (4 KB) | Erase range [0, 2) | Success | Sector cleared (0xFF pattern) | Readback all 0xFF |
| Block program | Program 2 blocks (8 KB) | Test pattern | Blocks written | Verify reads match |
| Block read | Read 2 blocks (8 KB) | Start=0 | Data returned | Pattern consistent |

---

## 2. Test Data & Configuration

### Controller Configurations

```rust
// From spitest.rs, adapted for SMC helpers:

FMC_CONFIG {
    mmap_base: 0x8000_0000,
    max_cs: 2,
    frequency: 50_000_000,  // 50 MHz SPI bus
    capacity_mb: [32, 8],   // CS0: 32 MB (W25Q256), CS1: 8 MB (W25Q64)
}

SPI0_CONFIG {
    mmap_base: 0x9000_0000,
    max_cs: 1,
    frequency: 50_000_000,
    capacity_mb: [64],      // CS0: 64 MB
}

SPI1_CONFIG {
    mmap_base: 0xb000_0000,
    max_cs: 2,
    frequency: 50_000_000,
    capacity_mb: [64, 64],  // CS0 + CS1: 64 MB each
}
```

### Test Data Patterns

| Pattern | Size | Purpose | Usage |
|---------|------|---------|-------|
| Sequential (0x00..0xFF..0x00) | 256 B | Page-sized | Write/read cycle |
| Random seed (0x179a_4e87) | 4 KB | Sector-sized | DMA stress test |
| All zeros (0x00000000) | Variable | Erase verify | Post-erase check |
| All ones (0xFFFFFFFF) | Variable | Factory state | Blank device check |

### Timing Constants

```rust
const ERASE_DELAY_NS: u32 = 2_000_000;    // 2 ms post-erase
const PROGRAM_DELAY_NS: u32 = 8_000_000;  // 8 ms post-program
const DMA_MIN_LENGTH: usize = 128;        // Threshold for DMA vs PIO
```

---

## 3. Test Execution (by Phase)

### Phase 1 (Quick, No Hardware)

```bash
# Host-based pure logic
bazelisk test //target/ast10x0/peripherals:smc_helpers_test
```

**Expected:** All host tests pass in < 1 second.

### Phase 2 (Emulation, ~30 mins)

```bash
# QEMU AST1030 boot
bazelisk build --config=virt_ast10x0 //tests/smc_qemu_integration:smc_integration_test
timeout 600 qemu-system-arm \
  -M ast1030-evb \
  -kernel dist/smc_integration_test.elf \
  -nographic \
  -serial stdio
```

**Expected:** Initialization + state transitions pass; register reads validated.

### Phase 3 (Hardware, ~2 hours)

```bash
# On real AST1030/AST1060 board with flash devices connected
cargo test --release --target=...
```

**Expected:** Full read/write verification passes on all controllers/CS combinations.

### Phase 4 (System, ~1 hour)

```bash
# Integration with firmware update service
./scripts/firmware_update_test.sh
```

**Expected:** Multi-controller scenario succeeds; failover works.

---

## 4. Success Criteria

| Phase | Criterion | Metric |
|-------|-----------|--------|
| 1 | All pure helper tests pass | 13/13 tests pass |
| 2 | Controller init, PIO, DMA all work | 4/4 controller types ready |
| 3 | Device layer read/write without errors | 9/9 test scenarios pass |
| 4 | Multi-controller system stable | No register corruption, no deadlock |

---

## 5. Known Limitations & Future Work

- **No async interrupt testing** (mirrors aspeed-rust comment: "irq is not being handled in this test")
- **No write operations on production data** (Phase 3 limited to test regions)
- **No multi-threaded contention tests** (assumes single-threaded firmware)
- **Future:** Add stress tests (1000 read/write cycles), timing calibration validation

---

## 6. Test Artifacts & Logging

All tests should output to UART:

```
################ DMA TEST starts. base: 0x7e620000 addr: 0x0000, len: 0x00001000 ######################
write pointer 0x60000000
##start sector erase
##start page_programing
##start read
read buffer:
  [0x00, 0x01, 0x02, ..., 0xFE, 0xFF]
Mmap buffer: 0x80001000
read write test passed!
################# FMC test done! ###############
```

---

## References

- Source: `aspeed-rust/src/spi/spitest.rs` (lines 1–600+)
- Architecture: [FMC_SPI_DRIVER_ARCHITECTURE.md](FMC_SPI_DRIVER_ARCHITECTURE.md)
- Helpers module: [helpers.rs](helpers.rs)
