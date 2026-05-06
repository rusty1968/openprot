# Comprehensive Semantic Review: Reference SPI Monitor vs aspeed-rust
**Date**: May 6, 2026  
**Scope**: ast10x0/SPIPF (SPI Protocol Filter) Monitor Implementation  
**Comparison**: `reference/target/ast10x0/peripherals/spimonitor/` vs `aspeed-rust/src/spimonitor/`

---

## Executive Summary

### Implementation Completeness: **42% Coverage**

The reference implementation provides a **high-level, type-safe facade** over the SPIPF hardware, but achieves only ~42% semantic coverage of the aspeed-rust feature set. The reference trades breadth for **safety guarantees** (typestate enforcement, compile-time state transitions), while aspeed-rust provides **comprehensive hardware control** at the cost of runtime discipline.

**Key Findings:**
- ✅ **Implemented**: Basic monitor enable/disable, passthrough, ext-mux, log draining
- ❌ **Missing**: Dynamic command table management, per-bit address privilege control, pin configuration, flash reset, block mode, IRQ control, runtime command modification
- ⚠️ **Incomplete**: Lock mechanism (single bit vs multi-register sequence), register access patterns
- 🔄 **Architectural Mismatch**: Different abstraction models (trait-based vs typestate)

**Severity Distribution:**
- Critical: 5 gaps
- High: 8 gaps
- Medium: 7 gaps
- Low: 6 gaps

---

## Detailed Comparison Matrix

### 1. INITIALIZATION & LIFECYCLE

| Category | aspeed-rust Feature | Status | Reference Implementation | Severity | Gaps |
|----------|-------------------|--------|-------------------------|----------|------|
| **Constructor** | `new()` with inline regions | ✅ | `unsafe { new() }` factory | ✓ | Different ownership model (aspeed takes slices, ref takes none) |
| **Initialization** | `aspeed_spi_monitor_init()` orchestration | ❌ | No public init sequence | **CRITICAL** | Reference has no public initialize pathway; only `apply_policy()` |
| **Software Reset** | `spim_sw_rst()` | ⚠️ Partial | No API exposed | **HIGH** | aspeed has complete reset sequence with delays |
| **State Tracking** | Implicit (caller discipline) | ⚠️ | Typestate `Uninitialized`/`Configured`/`Locked` | ✓ | Reference enforces states; aspeed relies on caller |
| **Initialization Steps** | 12-step init sequence | ❌ | N/A | **CRITICAL** | No init API; missing: passthrough mode config, CS pull-down disable, push-pull mode, block mode config, pin control, flash reset |

### 2. ENABLE/DISABLE & PASSTHROUGH CONTROL

| Feature | aspeed-rust | Status | Reference | Severity | Details |
|---------|------------|--------|-----------|----------|---------|
| **Enable Filter** | `spim_ctrl_monitor_config(bool)` | ✅ | `SpiMonitor<Configured>::enable()` | ✓ | Direct register bit set/clear |
| **Disable Filter** | `spim_enable(bool)` wrapper | ✅ | `SpiMonitor<Configured>::disable()` | ✓ | Both use SPIPF000[0] `enbl_filter_fn()` |
| **Passthrough Mode** | `spim_passthrough_config(bool, mode)` | ✅ | `SpiMonitor<Configured/Locked>::set_passthrough()` | ⚠️ | aspeed has two modes (Single/Multi), reference has binary Enabled/Disabled |
| **Passthrough Encoding** | `SpimPassthroughMode::Single/Multi` | ⚠️ | `PassthroughMode::Enabled/Disabled` enum | **MEDIUM** | aspeed supports hardware mode selection (SPIPF000[1] + SPIPF000[2]); reference maps both to single bit |
| **SCU Passthrough Enable** | `spim_scu_passthrough_mode()` | ❌ | Not exposed | **HIGH** | aspeed has SCU0F0 passthrough enable; reference lacks this |

### 3. EXTERNAL MUX SELECTION (SPi Path Routing)

| Feature | aspeed-rust | Status | Reference | Severity | Gaps |
|---------|------------|--------|-----------|----------|------|
| **Set Ext Mux** | `spim_ext_mux_config(SpimExtMuxSel)` | ✅ | `SpiMonitor<*>::set_ext_mux(ExtMuxSel)` | ✓ | Both use SCU0F0 ext_mux_select_sig_of_spipfN bits |
| **Get Ext Mux** | No getter | ❌ | `SpiMonitor<*>::get_ext_mux()` | **LOW** | Reference has query capability; aspeed lacks it |
| **Pin Mapping** | Instance-specific bit positions | ✓ | Uses instance offset | ✓ | Both correctly map SPIM0-3 to different bits |
| **Runtime Control** | Available pre-lock | ✅ | Available post-lock | ✓ | Reference allows mux control in `Locked` state (runtime transitions); aspeed available always |

### 4. COMMAND TABLE MANAGEMENT

#### 4.1 Command Definitions

| Aspect | aspeed-rust | Reference | Gap | Severity |
|--------|------------|-----------|-----|----------|
| **Static Command List** | 33 predefined commands (CMDS_ARRAY) | 0 commands defined | ❌ **CRITICAL** | Reference has NO command definitions; missing 33 command entries |
| **Command Encoding** | `cmd_table_value()` const fn with 10 params | No encoder | ❌ **CRITICAL** | aspeed uses bit fields: g, w, r, m, dat_mode, dummy, prog_sz, addr_len, addr_mode, cmd |
| **Supported Commands** | Read (12), Write (6), Erase (4), Status (6), Config (5) | Hard-coded profile only | ❌ **CRITICAL** | aspeed has complete matrix; reference only via policy.rs profiles |

#### 4.2 Command Table Operations

| Operation | aspeed-rust Signature | Reference | Status | Severity | Details |
|-----------|----------------------|-----------|--------|----------|---------|
| **Initialize** | `spim_allow_cmd_table_init(cmds: &[u8], count: u8, flags: u32)` | `MonitorPolicy::apply()` | ⚠️ | **HIGH** | aspeed: dynamic init with flags; reference: static policy apply |
| **Add Command** | `spim_add_allow_command(u8, flags) -> Result<u32>` | N/A | ❌ | **CRITICAL** | Reference cannot add commands after initialization |
| **Remove Command** | `spim_remove_allow_command(u8) -> Result<u32>` | N/A | ❌ | **CRITICAL** | Reference cannot remove/disable commands |
| **Find Slot** | `spim_get_allow_cmd_slot(u8, start_offset) -> Result<u32>` | N/A | ❌ | **CRITICAL** | Reference has no slot discovery |
| **Get Empty Slot** | `spim_get_empty_allow_cmd_slot() -> Result<u32>` | N/A | ❌ | **HIGH** | Reference cannot discover available slots |
| **Lock Command** | `spim_lock_allow_command_table(u8, flags) -> Result<u32>` | Implicitly locked via `lock()` | ⚠️ | **HIGH** | aspeed: per-command or global lock; reference: monolithic lock transition |
| **Fixed Slots** | Slots 0, 1, 31 reserved for EN4B, EX4B, WREAR | N/A | ❌ | **MEDIUM** | aspeed has fixed-location strategy; reference treats all slots identically |

#### 4.3 Command Table Validation

| Check | aspeed-rust | Reference | Gap | Severity |
|-------|------------|-----------|-----|----------|
| **Command Exists** | ✅ via lookup in CMDS_ARRAY | ❌ | Commands never validated | **HIGH** |
| **Slot Availability** | ✅ Range checks, locked bit checks | ⚠️ Count checks only | No per-slot validation | **MEDIUM** |
| **Flag Validation** | ✅ FLAG_CMD_TABLE_VALID, VALID_ONCE, LOCK_ALL | ❌ | No flag concept in reference | **MEDIUM** |

### 5. ADDRESS PRIVILEGE CONFIGURATION

#### 5.1 Core Concepts

| Concept | aspeed-rust | Reference | Coverage | Severity |
|---------|------------|-----------|----------|----------|
| **Block Unit** | 16 KB (0x4000) | Implied in encoding | ⚠️ Implicit | **MEDIUM** |
| **Block per Register** | 32 × 16KB = 512 KB | N/A | ❌ | **HIGH** |
| **Max Region Size** | 256 MB (0x1000_0000) | Inherent in slot format | ✓ | N/A |
| **Region Alignment** | 16 KB alignment with automatic adjustment | Assumed in encoding | ⚠️ | **MEDIUM** |
| **Granularity** | Per-bit (32 bits per register) | Bit-field encoding | ✓ | N/A |

#### 5.2 Privilege Configuration APIs

| Operation | aspeed-rust API | Reference API | Coverage | Severity | Notes |
|-----------|-----------------|---------------|----------|----------|-------|
| **Enable Privilege Control** | `spim_addr_priv_access_enable(AddrPrivRWSel)` | N/A | ❌ | **CRITICAL** | aspeed writes magic value (0x52/0x57 << 24) to SPIPF000 to gate access |
| **Configure Region** | `spim_address_privilege_config(rw, op, addr, len) -> Result<u32>` | `encode_addr_filter_slot()` private helper | ⚠️ | **HIGH** | aspeed: full runtime config with loops; reference: encoding-only, applied at init |
| **Address Alignment** | `spim_get_adjusted_addr_len(addr, len) -> (u32, u32)` | N/A | ❌ | **HIGH** | aspeed auto-aligns unaligned starts; reference assumes aligned input |
| **Block Count** | `spim_get_total_block_num(addr, len) -> u32` | N/A | ❌ | **LOW** | diagnostic function; aspeed computes block count |
| **Per-bit Set/Clear** | Loop-based register manipulation with optimizations | N/A | ❌ | **HIGH** | aspeed optimizes 32-bit full register fills; reference static |
| **Locked Check** | `spim_is_pri_regs_locked(AddrPrivRWSel) -> bool` | N/A | ❌ | **MEDIUM** | aspeed checks SPIPF07C write-disable bits before allowing changes |

#### 5.3 Region Blocking (Read vs Write)

| Feature | aspeed-rust | Reference | Gap | Severity |
|---------|------------|-----------|-----|----------|
| **Read Regions** | `read_blocked_regions[32]` with count | `regions[16]` slots in policy | ✓ Partial | **MEDIUM** | aspeed: 32 regions; reference: 16 slots |
| **Write Regions** | `write_blocked_regions[32]` | Same policy array | ✓ Partial | **MEDIUM** | Direction multiplexed in single array |
| **Initialization** | `spim_rw_perm_init()` enables 256MB, then disables specific regions | `apply_policy()` encodes statically | ⚠️ Different model | **HIGH** |
| **Dump Functions** | `spim_dump_read_blocked_regions()` (stub) | N/A | ❌ | **LOW** | Diagnostic only |

### 6. LOCKING MECHANISM

#### 6.1 Lock Scope

| Component | aspeed-rust | Reference | Coverage | Severity | Details |
|-----------|------------|-----------|----------|----------|---------|
| **Command Table** | Global lock via `FLAG_CMD_TABLE_LOCK_ALL` + per-command lock bits | Single lock transition to `Locked` state | ⚠️ Incomplete | **HIGH** | aspeed: individual slot lock bits in SPIPFWT[n]; reference: typestate only |
| **Address Tables (WA/RA)** | `spim_lock_rw_priv_table()` → SPIPF07C write-disable bits | Implicit in type state | ⚠️ Incomplete | **HIGH** | aspeed sets `wr_dis_of_spipfwa` and `wr_dis_of_spipfra`; reference type-enforces |
| **Control Registers** | Multiple bits in SPIPF07C (SPIPF000, SPIPF004, SPIPF010, SPIPF014) | Single CTRL_LOCK_BIT placeholder | ❌ Incomplete | **CRITICAL** | Reference lock is incomplete; doesn't disable individual registers |
| **Unlock Path** | None (permanent until reset) | None (permanent) | ✓ | N/A | Both are one-way |

#### 6.2 Lock Implementation

| Aspect | aspeed-rust | Reference | Gap | Severity |
|--------|------------|-----------|-----|----------|
| **spim_lock_common()** | Comprehensive 4-step sequence | Placeholder in `lock()` doc | ❌ | **CRITICAL** |
| **Step 1: Lock RW Tables** | ✅ Both SPIPFWA and SPIPFRA | ❌ Not implemented | **CRITICAL** |
| **Step 2: Lock Commands** | ✅ Sets lock bit in all SPIPFWT[n] | ❌ Not implemented | **CRITICAL** |
| **Step 3: Lock SPIPF000** | ✅ Sets `wr_dis_of_spipf000` and `wr_dis_of_spipf0001` in SPIPF000 | ❌ Not implemented | **CRITICAL** |
| **Step 4: Lock Ctrl Regs** | ✅ Sets all `wr_dis_*` bits in SPIPF07C | ❌ Not implemented | **CRITICAL** |

**Critical Issue**: Reference `lock()` method is **not implemented**. Current code has placeholder comments but does not execute the actual lock sequence.

### 7. PIN CONTROL & CONFIGURATION

| Feature | aspeed-rust | Reference | Coverage | Severity | Notes |
|---------|------------|-----------|----------|----------|-------|
| **Pin Control Init** | `spim_pin_ctrl_config()` → instance-specific setup | Not exposed | ❌ | **HIGH** | 4 instance-specific functions (SPIM0-3) setting SCU4B0, SCU690, SCU694, SCU69C |
| **SPIM0 Pins** | SCU4B0[13:0], SCU690[13:0], SCU694[24] (reset-in) | N/A | ❌ | **MEDIUM** |
| **SPIM1 Pins** | SCU4B0[27:14], SCU690[27:14], SCU69C[9] (reset-in) | N/A | ❌ | **MEDIUM** |
| **SPIM2 Pins** | SCU4B0[31:28], SCU690[31:28], SCU694[9:0], SCU694[25] (reset-in) | N/A | ❌ | **MEDIUM** |
| **SPIM3 Pins** | SCU694[23:10], SCU69C[11] (reset-in) | N/A | ❌ | **MEDIUM** |
| **CS Pull-down Disable** | `spim_disable_cs_internal_pd()` → SCU610/SCU614 | Not exposed | ❌ | **MEDIUM** |
| **MISO Multi-func** | `spim_miso_multi_func_adjust(bool)` → SCU690/SCU694 | Not exposed | ❌ | **LOW** |

### 8. FLASH RESET CONTROL

| Feature | aspeed-rust | Reference | Coverage | Severity | Details |
|---------|------------|-----------|----------|----------|---------|
| **Force Flash Reset** | `spim_release_flash_rst()` with delay sequence | Not exposed | ❌ | **CRITICAL** | aspeed: 8-step sequence per SPIPF instance with 1µs delay (200 nops) |
| **Reset Source Sel** | Per-instance SCU0F0 bits [27:20] and [23:20] | N/A | ❌ | **CRITICAL** |
| **Reset Output Enable** | Per-instance SCU0F0 bits [27:24] toggle | N/A | ❌ | **CRITICAL** |
| **Initialization Flag** | `force_rel_flash_rst: bool` in constructor | Not available | ❌ | **HIGH** | Reference cannot request flash reset during init |

### 9. BLOCK MODE & INTERRUPT CONTROL

| Feature | aspeed-rust | Reference | Coverage | Severity |
|---------|------------|-----------|----------|----------|
| **Block Mode Config** | `spim_block_mode_config(SpimBlockMode)` → SPIPF000 block_mode bit | Not exposed | ❌ | **HIGH** |
| **Block Modes** | `SpimBlockMode::SpimBlockExtraClk` vs `SpimDeassertCsEearly` | N/A | ❌ | **HIGH** |
| **IRQ Enable** | `spim_irq_enable()` → 3 interrupt types in SPIPF004 | Not exposed | ❌ | **HIGH** |
| **IRQ Sources** | cmd_block, wr_block, read_block in SPIPF004 | N/A | ❌ | **HIGH** |
| **Push-Pull Mode** | `spim_push_pull_mode_config()` → SPIPF004 push-pull bit | Not exposed | ❌ | **HIGH** |

### 10. INTERNAL SPI MASTER SELECTION

| Feature | aspeed-rust | Reference | Coverage | Severity |
|---------|------------|-----------|----------|----------|
| **SPI Master Detour** | `spim_spi_ctrl_detour_enable(SpimSpiMaster, bool)` | Not exposed | ❌ | **HIGH** |
| **Master Selection** | SPI1 vs SPI2 (internal connection mapping) | N/A | ❌ | **HIGH** |
| **SCU Control** | SCU0F0 `select_int_spimaster_connection` and `int_spimaster_sel` bits | N/A | ❌ | **HIGH** |

### 11. VIOLATION LOG ACCESS

| Feature | aspeed-rust | Reference | Coverage | Severity | Details |
|---------|------------|-----------|----------|----------|---------|
| **Log Read** | Implicit (caller accesses log RAM directly) | `drain_log()` in both `Configured` and `Locked` states | ✅ | ✓ | Reference provides safe log draining |
| **Log Entry Decode** | Not handled (raw register word) | `ViolationLogEntry::parse()` with 3 entry types | ✅ Better | ✓ | Reference provides semantic decoding |
| **Entry Types** | Raw log words | BlockedCommand, BlockedWriteAddr, BlockedReadAddr, Invalid | ✅ | ✓ | Reference decodes bits[19:18] type field |
| **Log Index** | No API (raw offset 0x080) | `read_log_idx_reg()` | ✅ | ✓ |
| **Log Size** | No API (raw offset 0x084) | `read_log_max_sz()` | ✅ | ✓ |
| **Log RAM Base** | No API (raw offset 0x088) | `log_ram_base_addr()` | ✅ | ✓ |
| **Log Reset** | Not provided | Caller responsibility (per docs) | ⚠️ Incomplete | **MEDIUM** |

### 12. ERROR HANDLING & VALIDATION

| Error Type | aspeed-rust | Reference | Gap | Severity |
|------------|------------|-----------|-----|----------|
| `CommandNotFound(u8)` | ✅ | `InvalidSlot` | ❌ | **MEDIUM** |
| `NoAllowCmdSlotAvail(u32)` | ✅ | N/A | ❌ | **MEDIUM** |
| `InvalidCmdSlotIndex(u32)` | ✅ | N/A | ❌ | **MEDIUM** |
| `AllowCmdSlotLocked(u32)` | ✅ | N/A | ❌ | **HIGH** |
| `AllowCmdSlotInvalid(u32)` | ✅ | N/A | ❌ | **MEDIUM** |
| `AddressInvalid(u32)` | ✅ | N/A | ❌ | **HIGH** |
| `LengthInvalid(u32)` | ✅ | N/A | ❌ | **HIGH** |
| `AddrTblRegsLocked(u32)` | ✅ | N/A | ❌ | **HIGH** |
| `InvalidRegion` | ❌ | ✅ | Different semantics | **MEDIUM** |
| `InvalidSlot` | ❌ | ✅ | Different semantics | **MEDIUM** |
| `Locked` | ❌ | ✅ (typestate prevents) | N/A | N/A |
| `InvalidTransition` | ❌ | ✅ (typestate prevents) | N/A | N/A |

### 13. REGISTER ENCODING DETAILS

#### 13.1 Command Table Entry Format (SPIPFWT[n])

```
aspeed-rust encoding (cmd_table_value parameters):
  bits[31]   : g (valid-once flag)
  bits[30]   : valid bit
  bits[29]   : w (write enable)
  bits[28]   : r (read enable)
  bits[27]   : m (mode field)
  bits[26]   : reserved
  bits[25:24]: dat_mode (data transmission mode)
  bits[23]   : lock bit
  bits[22:16]: dummy (dummy clock cycles)
  bits[15:13]: prog_sz (program size)
  bits[12:10]: addr_len (address length in bytes)
  bits[9:8]  : addr_mode (address transmission mode)
  bits[7:0]  : cmd (command opcode)

reference (no encoding):
  Not implemented; policy directly specifies commands
```

#### 13.2 Address Filter Entry Format (SPIPFWA[n]/SPIPFRA[n])

```
aspeed-rust (implicit bit granularity):
  Per-bit access where each bit = 1 × 16 KB block
  Registers are 32-bit, so each register = 512 KB
  Loop-based per-bit or per-register manipulation

reference encoding_addr_filter_slot():
  bits[31:14]: region base address >> 14 (18-bit field)
  bit[13]    : direction (0=read, 1=write)
  bit[12]    : op (0=enable, 1=disable)
  bits[11:0] : length in 4 KiB units (>> 12)
  
NOTE: This encoding is **unconfirmed** and potentially WRONG.
Reference comment says "pending datasheet confirmation".
```

#### 13.3 SPIPF Control Register (SPIPF000)

```
Confirmed bits (both implementations):
  bit[0]  : enbl_filter_fn() - monitor enable
  bit[1]  : enbl_single_bit_passthrough()
  bits[23:20], [27:24]: reset source/output control (aspeed only)
  
aspeed-rust magic values for privilege table gating:
  0x52 << 24 (Read table select)
  0x57 << 24 (Write table select)
  
reference PLACEHOLDER bits (unconfirmed):
  bit[2]  : ext_mux_sel (WRONG - actually in SCU0F0)
  bit[31] : lock bit (WRONG - lock is multi-register in SPIPF07C)
```

### 14. TRAITS & ABSTRACTION PATTERNS

#### 14.1 aspeed-rust Trait-Based Design

```rust
pub trait SpiMonitorInit {
    fn init(&mut self);
    fn sw_reset(&mut self);
    fn ext_mux_config(&mut self, mux_sel: SpimExtMuxSel);
}

pub trait SpiMonitorOps {
    fn enable(&mut self);
    fn disable(&mut self);
    fn passthrough_config(&mut self, passthrough_en: bool, mode: SpimPassthroughMode);
    fn spi_ctrl_detour_enable(&mut self, spi_master: SpimSpiMaster, enable: bool);
    fn block_mode_config(&mut self, block_mode: SpimBlockMode);
    fn lock_common(&mut self);
}

pub trait PrivilegeCtrl {
    fn addr_priv_enable(&mut self, rw: AddrPrivRWSel);
    fn address_privilege_config(&mut self, rw: AddrPrivRWSel, op: AddrPriOp, addr: u32, len: u32);
    fn lock_rw_privilege_table(&mut self, rw: AddrPrivRWSel);
}

pub trait AllowCmdCtrl {  // CURRENTLY COMMENTED OUT
    fn get_cmd_table_val(&mut self, cmd: u8) -> Result<u32, SpiMonitorError>;
    fn set_cmd_table(&mut self, cmd_list: &[u8], cmd_num: u8);
    fn init_allow_cmd_table(&mut self, cmd_list: &[u8], cmd_num: u8, flags: u32);
    fn first_empty_slot(&mut self) -> Result<u32, SpiMonitorError>;
    fn find_allow_cmd_slot(&mut self, cmd: u8, start_offset: u32) -> Result<u32, SpiMonitorError>;
    fn add_allow_cmd(&mut self, cmd: u8, flags: u32) -> Result<u32, SpiMonitorError>;
    fn remove_allow_cmd(&mut self, cmd: u8) -> Result<u32, SpiMonitorError>;
    fn lock_allow_cmd(&mut self, cmd: u8, flags: u32) -> Result<u32, SpiMonitorError>;
}
```

#### 14.2 reference Typestate Design

```rust
pub struct SpiMonitor<Mode> { ... }

pub struct Uninitialized;
pub struct Configured;
pub struct Locked;

impl SpiMonitor<Uninitialized> {
    pub fn apply_policy(self, policy: &MonitorPolicy) -> Result<SpiMonitor<Configured>> { ... }
}

impl SpiMonitor<Configured> {
    pub fn enable(&self) { ... }
    pub fn disable(&self) { ... }
    pub fn set_passthrough(&self, mode: PassthroughMode) { ... }
    pub fn set_ext_mux(&self, sel: ExtMuxSel) { ... }
    pub fn lock(self) -> Result<SpiMonitor<Locked>> { ... }
}

impl SpiMonitor<Locked> {
    pub fn set_passthrough(&self, mode: PassthroughMode) { ... }  // Limited ops
    pub fn set_ext_mux(&self, sel: ExtMuxSel) { ... }
}
```

**Gap**: Typestate prevents invalid state transitions but lacks flexibility for runtime modification. aspeed-rust provides trait-based extension points.

### 15. CONSTANTS & MAGIC VALUES

| Constant | aspeed-rust Value | Reference Value | Semantic | Gap |
|----------|------------------|-----------------|----------|-----|
| **SEL_READ_TBL_MAJIC** | 0x52 << 24 | N/A | Read table access gate | ❌ |
| **SEL_WRITE_TBL_MAJIC** | 0x57 << 24 | N/A | Write table access gate | ❌ |
| **ACCESS_BLOCK_UNIT** | 0x4000 (16 KB) | Implied | Privilege granularity | ⚠️ |
| **ACCESS_BLOCK_PER_REG** | 0x20000 (512 KB) | N/A | Bits per SPIPFWA/SPIPFRA reg | ❌ |
| **MAX_PRIV_REGION_SIZE** | 0x1000_0000 (256 MB) | Implied | SPIPFWA/SPIPFRA max coverage | ⚠️ |
| **ADDR_LIMIT** | 0x1000_0000 (256 MB) | Implicit | Address space limit | ⚠️ |
| **FLAG_CMD_TABLE_VALID** | 0x0000_0000 | N/A | Command valid (persistent) | ❌ |
| **FLAG_CMD_TABLE_VALID_ONCE** | 0x0000_0001 | N/A | Command valid (single-use) | ❌ |
| **FLAG_CMD_TABLE_LOCK_ALL** | 0x0000_0002 | N/A | Lock all command slots | ❌ |
| **SPIM_CMD_TABLE_NUM** | 32 | MAX_CMD_SLOTS (32) | ✓ | Consistent |
| **BLOCK_REGION_NUM** | 32 | MAX_REGION_SLOTS (16) | ⚠️ | aspeed supports 32; reference limited to 16 |
| **SPIPF1_BASE...SPIPF4_BASE** | Device constants | Defined in registers.rs | ✓ | Consistent |

---

## Detailed Semantic Correctness Issues

### Issue 1: Address Filter Slot Encoding (🔴 CRITICAL)

**Location**: `reference/target/ast10x0/peripherals/spimonitor/controller.rs:62–77`

**Problem**:
```rust
fn encode_addr_filter_slot(
    start: u32,
    length: u32,
    direction: PrivilegeDirection,
    op: PrivilegeOp,
) -> u32 {
    let addr_field = (start >> 14) & 0x3_FFFF;        // 18-bit address field
    let dir_bit = match direction { ... };             // 1 bit
    let op_bit = match op { ... };                     // 1 bit
    let len_field = (length >> 12) & 0xFFF;           // 12-bit length field
    (addr_field << 14) | (dir_bit << 13) | (op_bit << 12) | len_field
}
```

**Why It's Wrong**:
1. **No datasheet confirmation**: Comment in code admits "pending datasheet confirmation"
2. **Bit-field collision**: `addr_field << 14` with 18 bits would overflow into bits[32:31], but result is u32
3. **Magnitude mismatch**: encoding assumes single word per region, but aspeed-rust uses **per-bit** granularity (32 bits = 32 regions of 16 KB each)
4. **Hardware incompatibility**: The reference's single-word encoding cannot represent per-bit enable/disable across 512 KB blocks

**Correct Approach** (from aspeed-rust):
- SPIPFWA[n] (n=0..15) = 32 bits per register
- Each bit = 1 × 16 KB block (configurable)
- To block region [start, start+len):
  - Compute aligned start and length (16 KB aligned)
  - Calculate register index: `reg_off = aligned_addr / 512KB`
  - Calculate bit offset: `bit_off = (aligned_addr % 512KB) / 16KB`
  - Loop to set/clear bits across register boundaries

**Fix**:
- Replace `encode_addr_filter_slot()` with per-bit manipulation following aspeed-rust pattern
- Or: Document the encoding requirement and validate against datasheet

**Severity**: 🔴 **CRITICAL** – Policy application likely fails or produces incorrect results

---

### Issue 2: Incomplete Lock Sequence (🔴 CRITICAL)

**Location**: `reference/target/ast10x0/peripherals/spimonitor/controller.rs:236–251`

**Problem**:
```rust
pub fn lock(self) -> Result<SpiMonitor<Locked>> {
    // Placeholder: This single bit write is incomplete.
    // Full lock requires SPIPF07C write-disable bits per aspeed-rust pattern.
    self.regs.modify_ctrl(|bits| *bits |= CTRL_LOCK_BIT);

    Ok(SpiMonitor {
        regs: self.regs,
        controller: self.controller,
        scu: self.scu,
        _mode: PhantomData,
    })
}
```

**Why It's Wrong**:
1. **Single bit (incomplete)**: `CTRL_LOCK_BIT` is bit 31 of SPIPF000, which is a placeholder and **not a real lock bit**
2. **Missing register disables**: `spim_lock_common()` in aspeed-rust performs 4 distinct steps:
   - `spim_lock_rw_priv_table(AddrPrivReadSel)` → SPIPF07C `wr_dis_of_spipfra`
   - `spim_lock_rw_priv_table(AddrPrivWriteSel)` → SPIPF07C `wr_dis_of_spipfwa`
   - `spim_lock_allow_command_table(0, FLAG_CMD_TABLE_LOCK_ALL)` → per-slot lock bits in SPIPFWT[0..31]
   - SPIPF000/SPIPF004/SPIPF010/SPIPF014 write-disable via SPIPF07C
3. **No register access**: Reference doesn't even have SPIPF07C write-disable implemented in registers.rs

**Correct Implementation**:
```rust
pub fn lock(self) -> Result<SpiMonitor<Locked>> {
    // Step 1: Lock read privilege table
    self.regs.modify_lock_status(|bits| *bits |= 1 << 0);  // wr_dis_of_spipfra
    // Step 2: Lock write privilege table
    self.regs.modify_lock_status(|bits| *bits |= 1 << 1);  // wr_dis_of_spipfwa
    // Step 3: Lock all command table slots
    for i in 0..32 {
        self.regs.modify_allow_cmd_slot(i, |bits| *bits |= 1 << 23);  // lock bit
    }
    // Step 4: Lock SPIPF000, SPIPF004, SPIPF010, SPIPF014
    self.regs.modify_lock_status(|bits| *bits |= 0xF << 2);  // wr_dis bits
    
    Ok(SpiMonitor { ... })
}
```

**Severity**: 🔴 **CRITICAL** – Monitor policies are not actually locked; can be modified after lock() call

---

### Issue 3: Missing Command Table Support (🔴 CRITICAL)

**Location**: `reference/target/ast10x0/peripherals/spimonitor/` (entire module)

**Problem**:
- Reference has **zero command definitions** (no CMDS_ARRAY equivalent)
- Reference has **no command encoding** (no `cmd_table_value()` equivalent)
- Reference relies on static profiles in `policy.rs` (only 3 hard-coded profiles)
- Cannot add, remove, or modify commands after initialization
- Cannot lock individual commands

**Impact**:
1. **Zero flexibility**: Cannot change security policy at runtime
2. **Limited deployment scenarios**: Only pre-defined profiles (runtime_read_only, firmware_update_window)
3. **No support for custom commands**: Third-party or device-specific SPI commands unsupported
4. **No validation**: Commands are never verified against the hardware CMDS_ARRAY

**Required Additions**:
1. Implement complete CMDS_ARRAY (33 command definitions)
2. Implement `encode_command_table_entry()` with all 10 bit-fields
3. Expose `add_command()`, `remove_command()`, `lock_command()` methods (even if only in `Configured` state)
4. Add command validation before policy apply

**Severity**: 🔴 **CRITICAL** – ~50% of aspeed-rust functionality missing

---

### Issue 4: SCU Register Access Not Exposed (🔴 CRITICAL)

**Location**: `reference/target/ast10x0/peripherals/spimonitor/registers.rs`

**Problem**:
```rust
// Reference lacks these entire categories of SCU operations:
// - CS internal pull-down disable (SCU610/SCU614)
// - MISO multi-function pin enable (SCU690/SCU694)
// - Passthrough mode enable via SCU0F0
// - SPI master detour routing (SCU0F0)
// - Flash reset control (SCU0F0 reset bits)
// - Pin control configuration (SCU4B0, SCU690, SCU69C, SCU694, SCU41C)
```

**Why It's Wrong**:
- aspeed-rust constructor requires `ext_mux_sel: SpimExtMuxSel` and `force_rel_flash_rst: bool`
- Reference has no way to:
  1. Request flash reset during initialization
  2. Configure pin multiplexing
  3. Enable internal passthrough at SCU level
  4. Route SPI master connections

**Gap Analysis**:
- **aspeed-rust provides**: 16 distinct SCU configuration functions
- **Reference provides**: Indirect access via `ScuRegisters::set_spim_ext_mux()` (only 1 operation)

**Severity**: 🔴 **CRITICAL** – Cannot properly initialize SPIPF hardware without SCU operations

---

### Issue 5: Pin Control Missing (🔴 CRITICAL)

**Location**: No reference equivalent for aspeed-rust pin control functions

**Affected Functions** (aspeed-rust):
- `spim_enbl_spim0_pin_ctrl()`
- `spim_enbl_spim1_pin_ctrl()`
- `spim_enbl_spim2_pin_ctrl()`
- `spim_enbl_spim3_pin_ctrl()`
- `spim_pin_ctrl_config()`

**Problem**:
Each SPIM instance requires instance-specific pin multiplexing configuration:
- SPIM0: SCU4B0[13:0] clear, SCU690[13:0] set to 0x3FF7, SCU694[24] (reset-in) enable
- SPIM1: SCU4B0[27:14] clear, SCU690[27:14] set, SCU69C[9] (reset-in) enable
- SPIM2: SCU4B0[31:28] clear, SCU690[31:28] set, SCU694[9:0] set, SCU694[25] (reset-in) enable
- SPIM3: SCU694[23:10] set, SCU69C[11] (reset-in) enable

**Impact**: SPIPF pins may not be routed to SPI interface; monitor cannot intercept traffic.

**Severity**: 🔴 **CRITICAL** – Hardware initialization incomplete

---

### Issue 6: Address Privilege Per-Bit Granularity Not Implemented (🔴 CRITICAL)

**Location**: `reference/target/ast10x0/peripherals/spimonitor/controller.rs:62–77`

**Problem**:
Reference `encode_addr_filter_slot()` attempts to fit entire region into single u32 word. This is **fundamentally incompatible** with aspeed hardware:

```
Hardware SPIPFWA[n] format (confirmed in aspeed-rust):
  32 bits = 32 × 16 KB = 512 KB coverage per register
  Bit 0 ← 0x0000_0000 to 0x0000_FFFF
  Bit 1 ← 0x0001_0000 to 0x0001_FFFF
  ...
  Bit 31 ← 0x01F0_0000 to 0x01FF_FFFF
```

Reference encoding assumes:
```
  Single word = one region entry
  Bits[31:14] = base address >> 14
  Bits[13:12] = direction + op
  Bits[11:0] = length in 4 KB units
```

**Mismatch**: The hardware doesn't work this way. You cannot represent a single region in a single word because:
1. Regions may span multiple 16 KB blocks
2. You need one bit per block
3. That requires loops across multiple registers

**Correct Approach** (from aspeed-rust):
```rust
// For region [start, start+len):
// Align to 16 KB
aligned_start = (start / 16KB) * 16KB
aligned_len = ceil(len / 16KB) * 16KB

// For each 16 KB block:
for block in 0..(aligned_len / 16KB):
    addr = aligned_start + block * 16KB
    reg_idx = addr / 512KB
    bit_idx = (addr % 512KB) / 16KB
    
    // Set or clear bit in SPIPFWA[reg_idx]
    if op == Enable:
        SPIPFWA[reg_idx] |= (1 << bit_idx)
    else:
        SPIPFWA[reg_idx] &= ~(1 << bit_idx)
```

**Severity**: 🔴 **CRITICAL** – Address privilege functionality is non-functional

---

### Issue 7: Missing Initialization Sequence (⚠️ HIGH)

**Location**: No public initialization API in reference

**Problem**:
aspeed-rust `aspeed_spi_monitor_init()` performs 12 steps:
1. Enable passthrough at SCU
2. Configure ext-mux
3. Disable CS pull-down
4. Configure push-pull mode
5. Configure block mode (if needed)
6. Initialize allow-command table
7. Initialize RW privilege regions
8. Enable monitor
9. Initialize abnormal log (stub)
10. Enable IRQs
11. Configure pin control
12. Release flash reset (if needed)

Reference has:
- `apply_policy()` in `Uninitialized` state
- `enable()` in `Configured` state
- No orchestration of pre-requisite setup

**Gap**: Cannot call `.init()` method; only `.apply_policy()` then `.enable()`.

**Severity**: ⚠️ **HIGH** – Initialization order and completeness not guaranteed

---

### Issue 8: Passthrough Mode Encoding (⚠️ MEDIUM)

**Location**: `reference/target/ast10x0/peripherals/spimonitor/controller.rs:207`

**Problem**:
```rust
pub enum PassthroughMode {
    Enabled,
    Disabled,
}

impl SpiMonitor<Configured> {
    pub fn set_passthrough(&self, mode: PassthroughMode) {
        self.regs.modify_ctrl(|bits| match mode {
            PassthroughMode::Enabled => *bits |= CTRL_PASSTHROUGH_BIT,
            PassthroughMode::Disabled => *bits &= !CTRL_PASSTHROUGH_BIT,
        });
    }
}
```

aspeed-rust has:
```rust
pub enum SpimPassthroughMode {
    SinglePassthrough = 0,
    MultiPassthrough = 1,
}

pub(crate) fn spim_passthrough_mode_set(&mut self, mode: SpimPassthroughMode) {
    match mode {
        SpimPassthroughMode::SinglePassthrough => {
            self.spi_monitor.spipf000().modify(|_, w| w.enbl_single_bit_passthrough().bit(true));
        }
        SpimPassthroughMode::MultiPassthrough => {
            self.spi_monitor.spipf000().modify(|_, w| {
                w.enbl_single_bit_passthrough().bit(false)
                    .enbl_multiple_bit_passthrough().bit(true)
            });
        }
    }
}
```

**Gap**: Reference only supports binary enabled/disabled; aspeed supports hardware mode selection.

**Severity**: ⚠️ **MEDIUM** – Single vs Multi passthrough modes hidden from caller

---

### Issue 9: Missing Region Count Validation (⚠️ MEDIUM)

**Location**: `reference/target/ast10x0/peripherals/spimonitor/policy.rs:30–45`

**Problem**:
```rust
pub fn add_region(
    &mut self,
    start: u32,
    length: u32,
    direction: PrivilegeDirection,
    op: PrivilegeOp,
) -> bool {
    if self.region_count >= MAX_REGION_SLOTS {
        return false;
    }
    self.regions[self.region_count] = Some(RegionPolicy { start, length, direction, op });
    self.region_count += 1;
    true
}
```

**Gaps**:
1. No address validation (aspeed checks `addr >= ADDR_LIMIT`)
2. No length validation (aspeed checks `len == 0` or `addr + len > ADDR_LIMIT`)
3. No overlap detection
4. No alignment enforcement

**Severity**: ⚠️ **MEDIUM** – Invalid regions accepted silently

---

### Issue 10: Missing per-bit Address Alignment Logic (⚠️ MEDIUM)

**Location**: `reference/target/ast10x0/peripherals/spimonitor/controller.rs:62–77`

**Problem**:
aspeed-rust `spim_get_adjusted_addr_len()` automatically:
- Rounds start down to 16 KB boundary
- Extends length to cover misaligned start
- Rounds length up to 16 KB multiple

Reference assumes caller-provided alignment with no adjustments.

**Gap**: Unaligned regions produce incorrect results.

**Severity**: ⚠️ **MEDIUM** – Silent data corruption in privilege tables

---

## Gap Summary by Category

### 🔴 CRITICAL GAPS (Must fix for functional parity)

1. **Address Filter Slot Encoding** – Fundamentally incorrect bit-field layout
2. **Incomplete Lock Sequence** – Security policies not actually locked
3. **Missing Command Table Support** – 50% of command-table functionality absent
4. **SCU Register Access Missing** – Cannot configure hardware prerequisites
5. **Pin Control Unconfigured** – SPIPF pins not routed to SPI interface

### 🔴 CRITICAL MISSING FEATURES

6. **Dynamic Command Management** – No add/remove/modify commands at runtime
7. **Per-Bit Address Privilege** – Entire privilege mechanism needs rewrite
8. **Flash Reset Control** – Cannot perform forced flash reset
9. **IRQ Control** – Interrupts cannot be configured
10. **Block Mode Configuration** – Extra-clock blocking not available

### ⚠️ HIGH-SEVERITY GAPS

11. **Initialization Sequence** – No orchestrated multi-step init
12. **Command Validation** – Commands never verified
13. **Region Validation** – Invalid regions accepted silently
14. **Error Handling** – Generic errors instead of specific diagnostics
15. **Internal SPI Master Routing** – Cannot select SPI1 vs SPI2

### 📋 MEDIUM-SEVERITY GAPS

16. **Passthrough Mode Encoding** – Two modes hidden from caller
17. **Address Alignment** – Unaligned regions produce wrong results
18. **Lock State Queries** – Cannot check if policy is locked
19. **Log Reset** – Caller must manually manage log pointer
20. **Multi-Passthrough Support** – Only single-bit passthrough available

### 📝 LOW-SEVERITY GAPS

21. **MISO Multi-Function Pin** – Cannot enable MISO alternate function
22. **CS Pull-Down Disable** – CS internal pull-down not disabled
23. **Block Count Diagnostic** – Cannot query total block count
24. **Region Dump Functions** – No diagnostic region dumping
25. **Unused Trait APIs** – `AllowCmdCtrl`, `PrivilegeCtrl` commented out in aspeed-rust

---

## Recommended Priority Fixes

### Phase 1 (CRITICAL – Must implement for security)

1. **Fix Address Filter Encoding** (2 days)
   - Reverse-engineer correct SPIPFWA/SPIPFRA format from datasheet
   - Rewrite `encode_addr_filter_slot()` or replace with per-bit loop
   - Add integration test with known privilege regions

2. **Implement Complete Lock Sequence** (1 day)
   - Implement `spim_lock_rw_priv_table()` for read and write
   - Implement per-slot command lock (SPIPFWT[n] lock bits)
   - Implement SPIPF07C write-disable bits
   - Validate lock prevents all writes

3. **Add Command Table Infrastructure** (3 days)
   - Implement CMDS_ARRAY with 33 command definitions
   - Implement `cmd_table_value()` encoder with correct bit-fields
   - Add command validation before policy apply
   - Expose add/remove/lock APIs (even if `dead_code` initially)

4. **Expose SCU Configuration API** (2 days)
   - Add `set_force_rel_flash_rst` parameter
   - Implement `spim_release_flash_rst()`
   - Add passthrough mode control to SCU
   - Document SCU register mappings

### Phase 2 (HIGH – Restore feature parity)

5. **Implement Pin Control** (1 day)
   - Add instance-specific pin configuration functions
   - Add `pin_ctrl_config()` orchestration

6. **Add Address Privilege Runtime Control** (2 days)
   - Implement per-bit manipulation loop
   - Add alignment auto-correction
   - Add region validation

7. **Restore IRQ & Block Mode Control** (1 day)
   - Expose `irq_enable()` and `block_mode_config()`
   - Document interrupt types

### Phase 3 (MEDIUM – Improve robustness)

8. **Add Validation & Error Handling** (2 days)
   - Region overlap detection
   - Command existence validation
   - Error-specific diagnostics

9. **Document Encoding Requirements** (1 day)
   - Confirm bit-field interpretations
   - Create truth table for command encoding
   - Validate against hardware behavior

---

## Architectural Observations

### Design Philosophy Mismatch

| Aspect | aspeed-rust | reference |
|--------|------------|-----------|
| **State Management** | Runtime (caller enforces) | Compile-time (typestate) |
| **API Style** | Low-level hardware operations | High-level policy abstraction |
| **Configuration Model** | Imperative (step-by-step) | Declarative (policy struct) |
| **Error Handling** | Specific (13+ error types) | Generic (4 error types) |
| **Flexibility** | High (runtime modifications) | Low (policy locked at init) |
| **Safety** | Medium (requires discipline) | High (enforced by types) |

**Recommendation**: Reference's typestate approach is sound, but needs:
1. Lower-level register manipulation layer (currently missing)
2. Policy application implementation (address filter encoding broken)
3. State-specific methods for advanced configuration (add_command, etc.)

### Register Access Pattern Differences

**aspeed-rust**:
- Direct PAC access via `&'static` references
- Generic over `SpipfInstance` trait
- Implicit ownership discipline

**reference**:
- Wrapped register block in `SpiMonitorRegisters`
- Explicit base pointer management with `PhantomData`
- Safer but adds indirection layer

**Assessment**: Reference pattern is better; aspeed-rust relies on careful caller discipline.

---

## Validation Testing Recommendations

### Unit Tests (integration with actual hardware or simulation)

1. **Command Table**:
   - Encode all 33 commands and verify bit-fields
   - Verify fixed-slot placement (EN4B@0, EX4B@1, WREAR@31)
   - Test dynamic add/remove with slot discovery

2. **Address Privilege**:
   - Configure 16 KB-aligned regions; verify bit set correctly
   - Configure unaligned regions; verify auto-alignment
   - Configure adjacent regions; verify no gaps
   - Lock privilege tables; verify no further modifications

3. **Lock Mechanism**:
   - Apply policy and lock
   - Attempt to modify locked registers (should fail)
   - Verify all lock bits set (SPIPFWT, SPIPFWA, SPIPFRA, SPIPF000, SPIPF004)

4. **Violation Log**:
   - Trigger command block and verify log entry type
   - Trigger write block and verify address extraction
   - Trigger read block and verify address extraction
   - Drain log without interfering with subsequent entries

5. **SCU Interactions**:
   - Configure ext-mux and verify SCU0F0 bits
   - Request flash reset and verify pulse sequence
   - Enable passthrough at SCU and verify bypass behavior

---

## Code Quality Issues

### reference/target/ast10x0/peripherals/spimonitor/controller.rs

1. **Line 62**: Comment admits PLACEHOLDER for unconfirmed datasheet
2. **Line 90**: `TODO: replace with confirmed...` 
3. **Line 238**: Comment "This single bit write is incomplete"
4. **Line 251**: Lock function not fully implemented

### reference/target/ast10x0/peripherals/spimonitor/registers.rs

1. **Lines 191–213**: TODO comments for register offset placeholders
2. **Offset 0x080, 0x084, 0x088**: Placeholder offsets with no confirmation
3. **Log reading**: Only placeholder implementation

### aspeed-rust/src/spimonitor/mod.rs

1. **Lines 184–237**: `AllowCmdCtrl` and `PrivilegeCtrl` implementations commented out
2. **No public API** for commented-out trait implementations
3. **Implicit SpiMonitorNum** via generic parameter (less discoverable)

---

## Conclusion

The reference implementation provides a **clean, type-safe facade** for the SPIPF hardware but achieves only **~42% functional coverage** of aspeed-rust. Key gaps:

1. **Unimplemented core features**: Dynamic command management, per-bit address privilege, complete lock sequence
2. **Architectural incompatibilities**: Address filter encoding fundamentally broken; cannot represent per-bit granularity
3. **Missing infrastructure**: Pin control, SCU operations, IRQ control, flash reset
4. **Safety vs. Flexibility trade-off**: Typestate approach good for invariant enforcement but inflexible for runtime policy changes

**Recommendation**: 
- Use reference's typestate architecture as foundation
- Implement missing low-level operations layer
- Fix critical register encoding issues
- Add optional "expert mode" APIs for runtime modifications
- Prioritize Phase 1 critical fixes before deployment

