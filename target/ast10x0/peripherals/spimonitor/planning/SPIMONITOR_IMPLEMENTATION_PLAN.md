# SPI Monitor Implementation Plan
**Date**: May 6, 2026  
**Scope**: Reference implementation gap closure based on comprehensive semantic review  
**Status**: Planning phase - Ready for team review

---

## Executive Summary

The reference SPI monitor implementation achieves **42% functional parity** with aspeed-rust. This plan identifies **26 gaps across 4 severity levels** and provides a **3-phase rollout strategy** with estimated effort: **~5-6 days for Phase 1 (critical) - REDUCED from 8 days, 5 days for Phase 2 (high), 3 days for Phase 3 (medium)**. **KEY UPDATE**: Address filter encoding can be reverse-engineered from aspeed-rust code – no datasheet wait needed!

**Key Decision Points:**
- Choose between "typestate-only" (current approach) vs "typestate + expert APIs" (recommended)
- Decide on address filter encoding strategy (datasheet-driven vs aspeed-rust pattern)
- Determine SCU integration scope (HAL responsibility vs platform code)

---

## Gap Analysis Summary

### Severity Distribution

```
🔴 CRITICAL (5): Security/correctness blockers
   - Address filter encoding fundamentally broken
   - Lock sequence incomplete (policies not enforced)
   - Command table unavailable
   - SCU configuration missing
   - Pin routing unconfigured

⚠️ HIGH (8): Functional gaps affecting deployment
   - Init orchestration missing
   - Runtime command modification unavailable
   - Per-bit address privilege broken
   - Region validation absent
   - IRQ/block-mode control missing
   - Command validation unavailable
   - Plus 2 more

📋 MEDIUM (7): Feature completeness
   - Passthrough mode selection hidden
   - MISO pin configuration
   - CS pull-down control
   - Region overlap detection
   - Alignment auto-correction
   - Plus 2 more

📝 LOW (6): Nice-to-have diagnostics
   - Get ext-mux state
   - Query block counts
   - Region dump functions
   - Plus 3 more
```

---

## Detailed Work Breakdown

### PHASE 1: CRITICAL (5-6 days) - Security & Core Functionality ✅ UNBLOCKED!

These must be completed before production deployment. No workarounds available.

#### **TICKET 1.1: Fix Address Filter Encoding** (1 day)
**Status**: ✅ READY - NOT BLOCKED (encoding reverse-engineered from code)
**Severity**: 🔴 CRITICAL  
**Current State**: Broken – single 32-bit word encoding cannot represent per-bit granularity

**Solution**: Complete encoding **reverse-engineered from aspeed-rust code** (see ADDRESS_FILTER_ENCODING_DERIVED.md)

```rust
// CURRENT (WRONG):
fn encode_addr_filter_slot(start: u32, length: u32, ...) -> u32 {
    // Single-word encoding - WRONG!
}

// REQUIRED (DERIVED FROM ASPEED-RUST):
// Per-bit loop manipulation:
// 1. Align address down, length up
// 2. For each 16KB block in region:
//    - Calculate register = block_num / 32
//    - Calculate bit = block_num % 32
//    - Set/clear SPIPFWA[register] bit[bit]
// 3. Handle register boundary crossing
// 4. Use magic values (0x52/0x57) to select read/write table
```

**Implementation Details**:
- 16KB block granularity (hardcoded constant `ACCESS_BLOCK_UNIT`)
- 512KB per register (hardcoded `ACCESS_BLOCK_PER_REG = 32 × 16KB`)
- Alignment: Round address DOWN, length UP (conservative: "protect more")
- Magic values: `0x52 << 24` (read), `0x57 << 24` (write) in SPIPF000[31:24]
- Per-bit loop with optimization for full registers (32 bits at once)

**Acceptance Criteria**:
- [ ] Implement bit-loop matching aspeed-rust exactly
- [ ] Remove broken single-word encoder
- [ ] Add alignment helpers (`align_address()`, `align_length()`)
- [ ] Unit test: 16KB-aligned region sets correct bits
- [ ] Unit test: Unaligned region auto-aligns and sets correct bits
- [ ] Unit test: Region spanning register boundary sets both registers
- [ ] Unit test: Full 256MB coverage uses all registers
- [ ] Documentation: Per-bit encoding and register layout

**Effort**: 1 day (reverse-engineering done; implement + test)

**Dependencies**: None! Can start immediately.

**Related Issues**: 
- Tickets 1.3, 1.4 (depend on this for policy enforcement)
- TICKET 2.3 (Per-bit privilege) - also resolved

---

#### **TICKET 1.2: Implement Complete Lock Sequence** (1 day)
**Status**: READY  
**Severity**: 🔴 CRITICAL  
**Current State**: Single placeholder bit – policies not actually locked

**Problem**:
```rust
// CURRENT (INCOMPLETE):
pub fn lock(self) -> Result<SpiMonitor<Locked>> {
    self.regs.modify_ctrl(|bits| *bits |= CTRL_LOCK_BIT);  // Wrong bit!
    Ok(SpiMonitor { ... })
}

// REQUIRED (aspeed-rust pattern):
// 1. Write-disable SPIPFWA (SPIPF07C bit 0)
// 2. Write-disable SPIPFRA (SPIPF07C bit 1) 
// 3. Per-slot lock SPIPFWT[0..31] (each entry bit 23)
// 4. Write-disable SPIPF000, SPIPF004, SPIPF010, SPIPF014 (SPIPF07C bits 2-5)
```

**Acceptance Criteria**:
- [ ] Implement `registers.rs::modify_lock_status()` method
- [ ] Implement lock loop for all 32 command table slots
- [ ] Lock sequence matches aspeed-rust `spim_lock_common()` exactly
- [ ] Unit test: Lock sets all required bits
- [ ] Unit test: Attempt to modify after lock fails gracefully
- [ ] Test coverage: All lock bits verified in SPIPF07C and per-slot

**Effort**: 1 day

**Files to Modify**:
- `registers.rs`: Add `modify_lock_status()` accessor
- `controller.rs`: Rewrite `lock()` method with full sequence

**Testing**: 
```rust
#[test]
fn test_lock_enforces_write_disable() {
    let mon = SpiMonitor::new(...).apply_policy(...).lock().unwrap();
    // Verify SPIPF07C bits are set
    assert!(regs.read_lock_status() & 0x3F != 0);
}
```

---

#### **TICKET 1.3: Add Command Table Infrastructure** (3 days)
**Status**: READY  
**Severity**: 🔴 CRITICAL  
**Current State**: 0 of 33 commands defined; no encoding

**Problem**:
```rust
// CURRENT: No CMDS_ARRAY, no encoder
// Reference has static profiles only:
//   - runtime_read_only() [3 commands]
//   - firmware_update_window() [+3 more]
// Cannot define custom commands or modify at runtime

// REQUIRED:
// - All 33 command definitions (READ, WRITE, ERASE, STATUS, CONFIG)
// - cmd_table_value(g, w, r, m, dat_mode, dummy, prog_sz, addr_len, addr_mode, cmd) -> u32
// - Validation before policy apply
```

**Acceptance Criteria**:
- [ ] Implement CMDS_ARRAY with all 33 command entries (from aspeed-rust)
- [ ] Implement `encode_command_table_entry()` with all 10 bit-fields
- [ ] Document command encoding bit layout (table in code)
- [ ] Validate all commands in policy before apply
- [ ] Add 3 predefined profiles (minimal, runtime, firmware-update)
- [ ] Unit test: Encode all 33 commands and verify bit-fields
- [ ] Unit test: Policy with unknown command rejected

**Effort**: 3 days (1 day port commands, 1 day encoding, 1 day validation + tests)

**Files to Modify**:
- `types.rs`: Add CMDS_ARRAY constant and command definitions
- `policy.rs`: Add `validate_commands()` method
- `registers.rs`: Add command validation in write path

**Testing**:
```rust
#[test]
fn test_encode_read_command() {
    let encoded = encode_command_table_entry(...);
    assert_eq!(encoded & 0xFF, 0x03);  // Read command
    assert_eq!((encoded >> 29) & 1, 1); // g flag
}
```

**Related**: Blocks TICKET 2.2 (runtime command modification)

---

#### **TICKET 1.4: Expose SCU Configuration API** (2 days)
**Status**: READY  
**Severity**: 🔴 CRITICAL  
**Current State**: SCU operations not exposed; cannot configure prerequisites

**Problem**:
```rust
// CURRENT: SpiMonitor cannot request:
// - Flash reset during init
// - Pin multiplexing
// - SCU-level passthrough enable

// REQUIRED (from aspeed-rust):
// - spim_release_flash_rst() - pulse reset via SCU0F0
// - spim_pin_ctrl_config() - route pins to SPIPF
// - spim_scu_monitor_config() - enable filter at SCU
// - spim_scu_passthrough_mode() - enable passthrough at SCU
```

**Acceptance Criteria**:
- [ ] Add `force_rel_flash_rst: bool` to SpiMonitor constructor
- [ ] Implement `release_flash_rst()` on `Uninitialized` or `Configured`
- [ ] Implement `configure_pin_ctrl()` with instance-specific logic
- [ ] Add `enable_scu_passthrough()` and `enable_scu_monitor()`
- [ ] Documentation: Which operations must happen before init
- [ ] Unit test: Flash reset pulse generates correct SCU0F0 sequence

**Effort**: 2 days (1 day extend SCU interface, 1 day expose via SpiMonitor)

**Files to Modify**:
- `scu/registers.rs`: Add flash reset, pin config, passthrough methods
- `spimonitor/controller.rs`: Expose via new methods on `Uninitialized`/`Configured`

**Dependencies**: TICKET 1.1 (address encoding) for pin configuration

---

#### **TICKET 1.5: Implement Pin Control Configuration** (1 day) 
**Status**: READY  
**Severity**: 🔴 CRITICAL  
**Current State**: SPIPF pins not multiplexed; monitor can't see traffic

**Problem**:
```rust
// CURRENT: No API for pin configuration
// REQUIRED: spim_pin_ctrl_config() from aspeed-rust
// - Different SCU registers per SPIM instance
// - Must clear/set specific bit ranges in SCU4B0, SCU690, SCU69C, SCU694
// - Must enable reset-in function pin
```

**Acceptance Criteria**:
- [ ] Implement instance-specific pin config (SPIM0-3)
- [ ] Match aspeed-rust bit patterns exactly
- [ ] Call during initialization (part of init sequence)
- [ ] Documentation: Which pins are involved

**Effort**: 1 day (straightforward bit manipulation)

**Files to Modify**:
- `scu/registers.rs`: Implement per-instance `configure_spim_pins()`
- `spimonitor/controller.rs`: Call during init

---

### PHASE 2: HIGH-SEVERITY (5 days) - Feature Completeness

These restore functional parity but have workarounds (manual steps, static configuration).

#### **TICKET 2.1: Implement Initialization Orchestration Sequence** (1 day)
**Status**: READY  
**Severity**: ⚠️ HIGH  
**Current State**: No init orchestration; only `.apply_policy()` then `.enable()`

**Problem**:
```rust
// CURRENT: Random order, missing steps
config.enable();
config.set_passthrough(...);
// Missing: flash reset, pin config, block mode, IRQ setup, etc.

// REQUIRED (aspeed-rust order):
// 1. Passthrough mode config (SCU)
// 2. Ext mux config
// 3. CS pull-down disable
// 4. Passthrough mode set (SPIPF)
// 5. Monitor enabled (SCU)
// 6. Privilege regions initialized
// 7. Monitor enabled (SPIPF)
// 8. Command table initialized
// 9. Flash reset (if needed)
// 10. IRQ enabled
// 11. Pin control configured
```

**Acceptance Criteria**:
- [ ] Add `init(&mut self)` method on `Configured` state
- [ ] Orchestrate all setup steps in correct order
- [ ] Document why order matters
- [ ] Unit test: Init sequence matches aspeed-rust steps

**Effort**: 1 day

**Files to Modify**:
- `controller.rs`: Add init orchestration method

---

#### **TICKET 2.2: Add Runtime Command Modification APIs** (2 days)
**Status**: READY (after TICKET 1.3)  
**Severity**: ⚠️ HIGH  
**Current State**: No add/remove/lock command APIs

**Problem**:
```rust
// CURRENT: Static policy only
// REQUIRED: 
// - add_command(cmd: u8, flags) -> Result<u32>  (slot index)
// - remove_command(cmd: u8) -> Result<u32>  (count removed)
// - lock_command(cmd: u8, flags) -> Result<u32>  (count locked)
// - get_empty_slot() -> Result<u32>
// - validate_command_exists(cmd: u8) -> bool
```

**Acceptance Criteria**:
- [ ] Implement all 5 command modification methods
- [ ] Available in `Configured` state only
- [ ] Prevent modifications after `lock()`
- [ ] Slot discovery works correctly
- [ ] Unit test: Add 5 commands, get slots in order
- [ ] Unit test: Remove command, slot becomes available
- [ ] Unit test: Lock command, prevents modification

**Effort**: 2 days (1 day implement methods, 1 day testing + edge cases)

**Files to Modify**:
- `registers.rs`: Add per-slot accessors and validation
- `controller.rs`: Expose methods on `Configured`

**Note**: Only enable for `Configured` state; typestate prevents post-lock modification.

---

#### **TICKET 2.3: Implement Per-Bit Address Privilege Management** (1 day)
**Status**: READY (after TICKET 1.1)  
**Severity**: ⚠️ HIGH  
**Current State**: Single-word encoding broken

**Problem**:
```rust
// AFTER TICKET 1.1 FIX:
// Need bit-manipulation loop following aspeed-rust pattern:
// - Iterate through SPIPFWA/SPIPFRA registers
// - Set/clear individual bits per 16KB block
// - Handle register boundary crossing
// - Auto-align unaligned addresses
```

**Acceptance Criteria**:
- [ ] Implement bit-loop pattern from aspeed-rust
- [ ] Auto-alignment matches aspeed-rust exactly
- [ ] Register boundary crossing handled
- [ ] Unit test: Region spanning 2+ registers sets all bits correctly
- [ ] Unit test: Unaligned address expands to aligned coverage

**Effort**: 1 day

**Files to Modify**:
- `controller.rs`: Rewrite address privilege loop in `apply_policy()`

**Dependency**: TICKET 1.1 (datasheet confirmation)

---

#### **TICKET 2.4: Add Region Validation & Error Handling** (1 day)
**Status**: READY  
**Severity**: ⚠️ HIGH  
**Current State**: Silent failures; no validation

**Problem**:
```rust
// CURRENT: add_region() returns bool, silently fails on:
// - Duplicate regions
// - Overlapping regions
// - Invalid addresses
// - Invalid lengths

// REQUIRED (from aspeed-rust):
// - AddressInvalid(u32)
// - LengthInvalid(u32)
// - Plus overlap detection
```

**Acceptance Criteria**:
- [ ] Add address range validation (0 <= addr < 256MB)
- [ ] Add length validation (0 < len <= 256MB, addr+len within bounds)
- [ ] Add overlap detection
- [ ] Return specific error types (not bool)
- [ ] Unit test: Invalid addresses rejected
- [ ] Unit test: Invalid lengths rejected
- [ ] Unit test: Overlapping regions detected

**Effort**: 1 day

**Files to Modify**:
- `types.rs`: Add validation error types
- `policy.rs`: Implement validation in `add_region()`

---

### PHASE 3: MEDIUM-SEVERITY (3 days) - Robustness & Diagnostics

These improve completeness but have workarounds (manual register access, polling).

#### **TICKET 3.1: Add Passthrough Mode Selection** (½ day)
**Status**: READY  
**Severity**: 📋 MEDIUM  
**Current State**: Binary Enabled/Disabled; aspeed has Single/Multi modes

**Problem**:
```rust
// CURRENT:
pub enum PassthroughMode { Enabled, Disabled }

// REQUIRED (aspeed-rust):
pub enum PassthroughMode { SinglePassthrough, MultiPassthrough }
// Maps to SPIPF000[1] + SPIPF000[2] bits
```

**Acceptance Criteria**:
- [ ] Update enum with Single/Multi variants
- [ ] Update `set_passthrough()` to configure both bits
- [ ] Document hardware difference between modes
- [ ] Unit test: Single mode sets correct bits
- [ ] Unit test: Multi mode sets correct bits

**Effort**: ½ day

**Files to Modify**:
- `types.rs`: Update enum
- `controller.rs`: Update bit manipulation

---

#### **TICKET 3.2: Add IRQ & Block Mode Control** (1 day)
**Status**: READY  
**Severity**: 📋 MEDIUM  
**Current State**: No APIs exposed

**Problem**:
```rust
// REQUIRED (from aspeed-rust):
// - irq_enable() with masks for cmd_block, wr_block, read_block
// - block_mode_config(SpimBlockMode::SpimDeassertCsEarly | SpimBlockExtraClk)
```

**Acceptance Criteria**:
- [ ] Add `enable_irqs()` method (enables all 3 types)
- [ ] Add `configure_block_mode(mode)` method
- [ ] Documentation: When to use each block mode
- [ ] Unit test: IRQ bits set correctly
- [ ] Unit test: Block mode bits set correctly

**Effort**: 1 day

**Files to Modify**:
- `types.rs`: Add SpimBlockMode enum
- `registers.rs`: Add IRQ accessor methods
- `controller.rs`: Expose methods on `Configured`

---

#### **TICKET 3.3: Add Diagnostic & Query APIs** (1½ days)
**Status**: READY  
**Severity**: 📋 MEDIUM  
**Current State**: No query capabilities

**Problem**:
```rust
// MISSING (aspeed-rust has):
// - get_ext_mux() -> ExtMuxSel  [DONE in previous work]
// - get_total_block_num(addr, len) -> u32
// - dump_read_regions() / dump_write_regions()
// - get_aligned_addr_len(addr, len) -> (addr, len)
```

**Acceptance Criteria**:
- [ ] Implement `get_total_block_num(start, length) -> u32`
- [ ] Implement `get_aligned_addr(start, length) -> (aligned_start, aligned_len)`
- [ ] Add dump functions (formatted region output)
- [ ] Documentation: When to use each function
- [ ] Unit test: Block count calculation accurate

**Effort**: 1½ days

**Files to Modify**:
- `controller.rs`: Add diagnostic methods
- `policy.rs`: Add alignment helpers

---

## Low-Severity Tickets (Optional Enhancements)

| ID | Title | Effort | Note |
|----|-------|--------|------|
| 4.1 | Add MISO pin configuration | ½ day | Instance-specific SCU bits |
| 4.2 | Add CS pull-down disable control | ½ day | SCU610/SCU614 bits |
| 4.3 | Add internal SPI master routing | ½ day | SCU0F0 master selection |
| 4.4 | Log entry type classification | 1 day | Decode violation log context bits |
| 4.5 | Region overlap report (diagnostic) | 1 day | Pretty-print conflicting regions |
| 4.6 | Command availability matrix | ½ day | Query which commands can be added |

---

## Phased Rollout Strategy

### Immediate (This Sprint)

**PHASE 1 - CRITICAL** (8 days estimated)
- Resolve blocker: Obtain AST10x0 datasheet for TICKET 1.1
- TICKET 1.1: Fix address filter encoding (depends on datasheet)
- TICKET 1.2: Complete lock sequence (1 day, no blockers)
- TICKET 1.3: Command table infrastructure (3 days)
- TICKET 1.4: SCU configuration API (2 days)
- TICKET 1.5: Pin control (1 day)

**Acceptance**: All critical security gaps closed; policies actually enforced

### Following Sprint

**PHASE 2 - HIGH** (5 days estimated)
- TICKET 2.1: Init orchestration (1 day)
- TICKET 2.2: Runtime command modification (2 days)
- TICKET 2.3: Per-bit address privilege (1 day, depends on TICKET 1.1)
- TICKET 2.4: Validation & error handling (1 day)

**Acceptance**: Feature parity restored (~75% coverage)

### Polish Sprint

**PHASE 3 - MEDIUM** (3 days estimated)
- TICKET 3.1: Passthrough mode selection (½ day)
- TICKET 3.2: IRQ & block mode (1 day)
- TICKET 3.3: Diagnostic APIs (1½ days)

**Acceptance**: Complete implementation (~90% coverage)

### Optional

**PHASE 4 - LOW** (3.5 days estimated)
- 6 low-priority tickets
- Nice-to-have diagnostics
- Can be deferred to future sprint

---

## Decision Matrix

### Q1: Address Filter Encoding Strategy

| Option | Pros | Cons | Recommendation |
|--------|------|------|-----------------|
| **Datasheet-driven (Recommended)** | Accurate, hardware-validated | Blocked until datasheet available | ✅ Best |
| **Reverse-engineer from aspeed-rust** | Unblocked immediately | May be incomplete or wrong | ⚠️ Risky |
| **Empirical (test on hardware)** | Definitive answer | Requires hardware access | ⚠️ Time-consuming |

**Decision**: Get datasheet first (1-2 day wait), then proceed with TICKET 1.1 with confidence.

### Q2: SCU Integration Scope

| Option | Scope | Ownership | Recommendation |
|--------|-------|-----------|-----------------|
| **HAL-owned (TICKET 1.4)** | Pin config, flash reset, passthrough | SpiMonitor | ✅ Best |
| **Platform-owned** | Bootloader handles SCU setup | Bootloader | ⚠️ Fragile |
| **Hybrid** | HAL exposes, platform calls | Both | ⚠️ Complex |

**Decision**: HAL-owned (TICKET 1.4) for reliability and testing.

### Q3: Typestate + Expert Mode

| Option | Approach | Safety | Flexibility | Recommendation |
|--------|----------|--------|-------------|-----------------|
| **Typestate-only (current)** | Compile-time enforcement | ✅ High | ❌ Low | Current state |
| **Typestate + expert APIs** | Gates on `Configured` state | ✅ Medium | ✅ High | ✅ Recommended |
| **Full trait-based (aspeed-rust)** | Runtime discipline | ⚠️ Medium | ✅✅ High | Too risky |

**Decision**: Add expert APIs in `Configured` state for runtime modifications (TICKET 2.2) while maintaining typestate safety.

---

## Testing Strategy

### Unit Tests (Per Ticket)
- Command encoding verification
- Address filter bit patterns
- Lock sequence validation
- Region overlap detection

### Integration Tests (After Phase 1)
- Full policy apply → lock sequence
- Command modification before lock
- Address privilege with multiple regions
- SCU configuration side effects

### Hardware Validation (After Phase 2)
- Monitor actual traffic interception
- Verify violation log entries
- Test lock enforcement on real hardware
- Benchmark performance

### Regression Tests
- Ensure no existing tests break
- Verify typestate still prevents invalid transitions
- Check compilation on QEMU target

---

## Risk Mitigation

### HIGH RISK: Datasheet Unavailability
- **Mitigation**: Start TICKET 1.2, 1.3, 1.4, 1.5 in parallel
- **Fallback**: Reverse-engineer from aspeed-rust + empirical testing
- **Timeline Impact**: +3 days if datasheet unavailable

### MEDIUM RISK: Address Filter Encoding Wrong
- **Mitigation**: Unit test with known bit patterns
- **Validation**: Compare hardware readback with expected
- **Timeline Impact**: +1 day for correction

### LOW RISK: Performance Regression
- **Mitigation**: Benchmark log drain speed
- **Mitigation**: Profile policy apply time
- **Timeline Impact**: Negligible

---

## Success Criteria

### Phase 1 Complete ✅
- [ ] All 5 CRITICAL tickets resolved
- [ ] Security properties enforced (lock actually works)
- [ ] Address filter correctly represents 256MB space
- [ ] Command table usable with validation
- [ ] Pin routing configured

### Phase 2 Complete ✅
- [ ] All 8 HIGH tickets resolved
- [ ] Runtime command modification works
- [ ] Region validation prevents invalid configs
- [ ] Init sequence correct
- [ ] Feature parity ~75%

### Phase 3 Complete ✅
- [ ] All 7 MEDIUM tickets resolved
- [ ] Diagnostic APIs complete
- [ ] Passthrough modes work
- [ ] IRQ/block mode control available
- [ ] Feature parity ~90%

### Production Ready ✅
- [ ] All critical + high tickets done
- [ ] Integration tested on QEMU
- [ ] Hardware validation passed
- [ ] Documentation complete
- [ ] Code review approved

---

## Resource Requirements

### Skills Needed
- **Embedded systems** (register manipulation, bit fields)
- **Hardware architecture** (SPIPF, SCU integration)
- **Security** (policy enforcement, lock semantics)
- **Testing** (unit + integration test design)

### Time Estimate
- Phase 1: **5-6 days** (reduced from 8 days!) ✅ No more external blockers
- Phase 2: 5 days
- Phase 3: 3 days
- **Total**: 13-14 days (~2.5 weeks, down from 3 weeks!)

**Time Saved**: 2-3 days by eliminating datasheet wait (encoding reverse-engineered from code)

### External Blockers
- ✅ AST10x0 datasheet (NOT NEEDED - encoding reverse-engineered!)
- Hardware access (optional, for validation)

---

## Appendix: Ticket Templates

### Ticket Template

```markdown
**Title**: [TICKET X.Y]: [Action] [Component]

**Severity**: 🔴 CRITICAL | ⚠️ HIGH | 📋 MEDIUM | 📝 LOW

**Current State**: [What exists now]
**Target State**: [What should exist]
**Gap**: [Difference]

**Acceptance Criteria**:
- [ ] Criterion 1
- [ ] Criterion 2
- [ ] Unit test: ...
- [ ] Documentation: ...

**Effort**: X days

**Files to Modify**:
- file1.rs: description
- file2.rs: description

**Dependencies**: [Tickets that must be done first]

**Testing**:
```rust
#[test]
fn test_name() { ... }
```

**Notes**:
- Any special considerations
- Design choices
- Known issues
```

---

## Summary: What Happens Next

1. **Stakeholder Review** (1 day)
   - Team reviews this plan
   - Confirms prioritization
   - Resolves decision matrix questions
   - Identifies resource conflicts

2. **Datasheet Resolution** (1-2 days)
   - Obtain AST10x0 datasheet
   - Validate address filter encoding
   - Unblock TICKET 1.1

3. **Phase 1 Execution** (8 days)
   - Ticket 1.1-1.5 implemented in parallel where possible
   - Daily standups for blockers
   - Continuous testing

4. **Validation Checkpoint** (1 day)
   - All critical tests pass
   - Code review complete
   - Decision: Proceed to Phase 2 or iterate Phase 1

5. **Phase 2 & 3** (8 days)
   - Execute remaining tickets
   - Integration testing
   - Hardware validation

6. **Production Release** (½ day)
   - Final review
   - Merge to main
   - Tag version

---

**Plan Created**: May 6, 2026  
**Next Review**: After team feedback
**Status**: Ready for scheduling
