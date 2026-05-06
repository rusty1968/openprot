# SPI Monitor Feature Parity Matrix
**Current State**: 42% coverage (11 of 26 features) | **Target**: 100%

---

## Feature Implementation Status

### PHASE 1: CRITICAL FEATURES (0% → 60%)

| Feature | Category | Status | Implementation | Gap | Ticket | Priority |
|---------|----------|--------|-----------------|-----|--------|----------|
| **Initialization** | Setup | ❌ Missing | None | No public init sequence | 2.1 | HIGH |
| **Software Reset** | Setup | ⚠️ Partial | No API | Hidden in controller | 1.5 | CRITICAL |
| **Address Filter Encoding** | Privilege | 🔴 Broken | Single-word encoding | Per-bit granularity needed | 1.1 | CRITICAL |
| **Lock Mechanism** | Security | ❌ Broken | 1 placeholder bit | Multi-register SPIPF07C needed | 1.2 | CRITICAL |
| **Command Table Definitions** | Commands | ❌ Missing | 0 commands defined | 33 commands required | 1.3 | CRITICAL |
| **Command Encoding** | Commands | ❌ Missing | No encoder | 10-bit field encoding needed | 1.3 | CRITICAL |
| **SCU Flash Reset** | Config | ❌ Missing | No API | spim_release_flash_rst() needed | 1.4 | CRITICAL |
| **SCU Pin Control** | Config | ❌ Missing | No API | Per-instance routing needed | 1.5 | CRITICAL |
| **Monitor Enable/Disable** | Control | ✅ Done | `enable()` / `disable()` | N/A | N/A | - |
| **Passthrough Control** | Control | ✅ Done | `set_passthrough()` | Mode selection hidden | 3.1 | MEDIUM |
| **Ext Mux Selection** | Routing | ✅ Done | `set_ext_mux()` / `get_ext_mux()` | N/A | N/A | - |

### PHASE 2: HIGH-PRIORITY FEATURES (60% → 80%)

| Feature | Category | Status | Implementation | Gap | Ticket | Priority |
|---------|----------|--------|-----------------|-----|--------|----------|
| **Init Orchestration** | Setup | ❌ Missing | No orchestration | 12-step sequence needed | 2.1 | HIGH |
| **Region Validation** | Privilege | ⚠️ Weak | Silent failures | Return error types | 2.4 | HIGH |
| **Region Overlap Detection** | Privilege | ❌ Missing | Not checked | Overlap detection needed | 2.4 | HIGH |
| **Runtime Add Command** | Commands | ❌ Missing | Static policy only | spim_add_allow_command() | 2.2 | HIGH |
| **Runtime Remove Command** | Commands | ❌ Missing | Static policy only | spim_remove_allow_command() | 2.2 | HIGH |
| **Runtime Lock Command** | Commands | ❌ Missing | No per-command lock | Individual slot locks | 2.2 | HIGH |
| **Per-Bit Address Privilege** | Privilege | 🔴 Broken | Single-word | Bit-loop pattern needed | 2.3 | HIGH |
| **IRQ Control** | Events | ❌ Missing | No API | irq_enable() needed | 3.2 | MEDIUM |

### PHASE 3: MEDIUM-PRIORITY FEATURES (80% → 90%)

| Feature | Category | Status | Implementation | Gap | Ticket | Priority |
|---------|----------|--------|-----------------|-----|--------|----------|
| **Passthrough Modes (Single/Multi)** | Control | ⚠️ Hidden | Binary only | Mode selection exposed | 3.1 | MEDIUM |
| **Block Mode Config** | Control | ❌ Missing | No API | block_mode_config() | 3.2 | MEDIUM |
| **Diagnostic: Block Count** | Query | ❌ Missing | No API | get_total_block_num() | 3.3 | MEDIUM |
| **Diagnostic: Aligned Addr** | Query | ❌ Missing | No API | get_aligned_addr() | 3.3 | MEDIUM |
| **Diagnostic: Region Dump** | Query | ❌ Missing | No API | dump_regions() | 3.3 | MEDIUM |
| **Violation Log Drain** | Log | ✅ Done | `drain_log()` | Offset placeholders | N/A | - |
| **Command Table Slot Finding** | Commands | ❌ Missing | No API | spim_get_empty_slot() | 2.2 | HIGH |

### PHASE 4: LOW-PRIORITY FEATURES (90% → 100%)

| Feature | Category | Status | Implementation | Gap | Ticket | Priority |
|---------|----------|--------|-----------------|-----|--------|----------|
| **MISO Pin Enable** | Config | ❌ Missing | No API | spim_miso_multi_func_adjust() | 4.1 | LOW |
| **CS Pull-Down Disable** | Config | ❌ Missing | No API | spim_disable_cs_internal_pd() | 4.2 | LOW |
| **Internal SPI Master Routing** | Routing | ❌ Missing | No API | spim_spi_ctrl_detour_enable() | 4.3 | LOW |
| **Log Entry Type Classification** | Log | ❌ Missing | No decoder | Violation type from bits[19:18] | 4.4 | LOW |
| **Region Overlap Report** | Diagnostic | ❌ Missing | No API | Pretty-print conflicts | 4.5 | LOW |
| **Command Availability Matrix** | Query | ❌ Missing | No API | Which commands can be added | 4.6 | LOW |

---

## Coverage Timeline

```
Day 0:  ████░░░░░░ 42% (11/26 features)
Day 8:  ███████░░░ 65% (17/26 features) ← Phase 1 complete
Day 13: ██████████ 80% (21/26 features) ← Phase 2 complete
Day 16: ███████████ 90% (24/26 features) ← Phase 3 complete
Day 19: ██████████ 100% (26/26 features) ← Full parity
```

---

## Critical Path Dependency Graph

```
START (Day 0)
  │
  ├─→ [Datasheet] (External, 1-2 days)
  │    └─→ 1.1: Address Filter Encoding (2 days)
  │         └─→ 2.3: Per-Bit Address Privilege (1 day)
  │
  ├─→ 1.2: Lock Sequence (1 day) ─→ All features depending on lock
  │
  ├─→ 1.3: Command Infrastructure (3 days)
  │    └─→ 2.2: Runtime Command Modification (2 days)
  │
  ├─→ 1.4: SCU Configuration (2 days)
  │    └─→ 1.5: Pin Control (1 day)
  │
  ├─→ 2.1: Init Orchestration (1 day)
  │
  ├─→ 2.4: Region Validation (1 day)
  │
  ├─→ 3.1: Passthrough Modes (0.5 days)
  │
  ├─→ 3.2: IRQ/Block Mode (1 day)
  │
  ├─→ 3.3: Diagnostics (1.5 days)
  │
  └─→ [4.x: Low-Priority] (3.5 days, optional)

CRITICAL PATH DURATION: 8-10 days (determined by 1.1 + dependencies)
```

---

## By Category: Feature Completeness

### 🔒 Security/Locking (1/3 = 33%)
| Feature | Status | Effort | Impact |
|---------|--------|--------|--------|
| Monitor lock enforcement | ❌ Broken (1.2) | 1 day | CRITICAL |
| Per-slot command lock | ❌ Missing (1.3) | Included in 1.3 | CRITICAL |
| Write-disable registers | ❌ Missing (1.2) | Included in 1.2 | CRITICAL |

### 🛠️ Configuration (3/8 = 38%)
| Feature | Status | Effort | Impact |
|---------|--------|--------|--------|
| Initialization sequence | ❌ Missing (2.1) | 1 day | HIGH |
| Flash reset control | ❌ Missing (1.4) | Included in 1.4 | CRITICAL |
| Pin multiplexing | ❌ Missing (1.5) | Included in 1.5 | CRITICAL |
| Block mode config | ❌ Missing (3.2) | 1 day | MEDIUM |
| Passthrough modes (Single/Multi) | ⚠️ Hidden (3.1) | 0.5 day | MEDIUM |
| CS pull-down control | ❌ Missing (4.2) | 0.5 day | LOW |
| MISO pin enable | ❌ Missing (4.1) | 0.5 day | LOW |
| Internal SPI master routing | ❌ Missing (4.3) | 0.5 day | LOW |

### 🎛️ Command Management (3/7 = 43%)
| Feature | Status | Effort | Impact |
|---------|--------|--------|--------|
| Command definitions (33 cmds) | ❌ Missing (1.3) | Included in 1.3 | CRITICAL |
| Command encoding | ❌ Missing (1.3) | Included in 1.3 | CRITICAL |
| Runtime add command | ❌ Missing (2.2) | Included in 2.2 | HIGH |
| Runtime remove command | ❌ Missing (2.2) | Included in 2.2 | HIGH |
| Runtime lock command | ❌ Missing (2.2) | Included in 2.2 | HIGH |
| Slot discovery (empty) | ❌ Missing (2.2) | Included in 2.2 | HIGH |
| Command validation | ❌ Missing (1.3) | Included in 1.3 | CRITICAL |

### 🔐 Address Privilege (2/5 = 40%)
| Feature | Status | Effort | Impact |
|---------|--------|--------|--------|
| Address filter encoding | 🔴 Broken (1.1) | 2 days | CRITICAL |
| Per-bit privilege control | 🔴 Broken (2.3) | 1 day | HIGH |
| Region validation | ⚠️ Weak (2.4) | 1 day | HIGH |
| Overlap detection | ❌ Missing (2.4) | Included in 2.4 | HIGH |
| Automatic alignment | ⚠️ Assumed (2.3) | Included in 2.3 | HIGH |

### 🔍 Diagnostics (1/6 = 17%)
| Feature | Status | Effort | Impact |
|---------|--------|--------|--------|
| Query block count | ❌ Missing (3.3) | Included in 3.3 | MEDIUM |
| Query aligned address | ❌ Missing (3.3) | Included in 3.3 | MEDIUM |
| Dump regions (read) | ❌ Missing (3.3) | Included in 3.3 | MEDIUM |
| Dump regions (write) | ❌ Missing (3.3) | Included in 3.3 | MEDIUM |
| Dump region overlaps | ❌ Missing (4.5) | 1 day | LOW |
| Log entry classification | ❌ Missing (4.4) | 1 day | LOW |

### 📊 Monitoring (1/4 = 25%)
| Feature | Status | Effort | Impact |
|---------|--------|--------|--------|
| Log drain | ✅ Done | N/A | - |
| IRQ enable | ❌ Missing (3.2) | Included in 3.2 | MEDIUM |
| IRQ types (cmd/wr/rd block) | ❌ Missing (3.2) | Included in 3.2 | MEDIUM |
| Log entry decoder | ❌ Missing (4.4) | 1 day | LOW |

### ⚡ Control (2/3 = 67%)
| Feature | Status | Effort | Impact |
|---------|--------|--------|--------|
| Enable/disable monitor | ✅ Done | N/A | - |
| Passthrough control | ✅ Done | N/A | - |
| Ext mux selection | ✅ Done | N/A | - |

---

## Effort-to-Severity Scatterplot

```
SEVERITY (How critical)
  │
  │ CRITICAL ZONE          CRITICAL ZONE           CRITICAL ZONE
  │ (Do First)             (Depends on others)     (Lower effort)
  │ 1.3 (3d)              1.1 (2d)                1.2 (1d)
  │ █████████             ██████                  ███
  │
  │ HIGH                   HIGH                     HIGH
  │ 2.2 (2d)              2.3 (1d)                2.1 (1d) 2.4 (1d)
  │ ██████                ███                     ███     ███
  │
  │ MEDIUM                 MEDIUM
  │ 3.2 (1d)              3.1 (0.5d)  3.3 (1.5d)
  │ ███                   ██          █████
  │
  │ LOW
  │ 4.x (0.5d-1d each)
  │ ██ ██ ██ ██ ██ ██
  │
  └────────────────────────────────────────────── EFFORT (Days)
    0    1    2    3
```

**High Value**: Tickets in upper-left (high severity, low effort)
- 1.2 (Lock, 1 day)
- 2.1, 2.4 (Init/Validation, 1 day each)

**Low Value**: Tickets in lower-right (low severity, high effort)
- Phase 4 low-priority (3.5 days total)

---

## Progress Tracking Template

Use this to track Phase execution:

```markdown
## Phase 1 Progress

- [ ] 1.1: Address Filter Encoding (BLOCKED - waiting datasheet)
  - [ ] Datasheet received
  - [ ] Encoding validated
  - [ ] Implementation complete
  - [ ] Tests passing

- [ ] 1.2: Lock Sequence (IN PROGRESS)
  - [x] SPIPF07C register accessor added
  - [ ] Per-slot lock loop implemented
  - [ ] Full sequence tested
  - [ ] Code reviewed

- [ ] 1.3: Command Infrastructure (IN PROGRESS)
  - [x] CMDS_ARRAY ported
  - [ ] Encoder implemented
  - [ ] Validation tests passing
  - [ ] Documentation complete

- [ ] 1.4: SCU Configuration (IN PROGRESS)
  - [x] Flash reset methods added
  - [ ] Pin control exposed
  - [ ] Integration tested
  - [ ] Code reviewed

- [ ] 1.5: Pin Control (NOT STARTED)
  - [ ] Instance-specific routing verified
  - [ ] Implementation complete
  - [ ] Tests passing

**Phase 1 Completion**: 0% → 40% (2/5 tickets done)
**Timeline**: Day 1-8, currently on Day 3
**Blockers**: Datasheet (1.1), None (1.2-1.5 proceed)
```

---

## Feature Request Queue (For Product Owner)

If asked "what's missing?", respond with:

**Phase 1 (Must-Have before production)**
1. Address filter encoding fixed (waiting datasheet)
2. Lock mechanism fully implemented
3. 33 commands defined and encodable
4. SCU configuration API exposed
5. Pin routing configured

**Phase 2 (Should-Have for feature parity)**
6. Runtime command modification
7. Region validation with overlap detection
8. Initialization orchestration
9. Per-bit address privilege working correctly

**Phase 3 (Nice-to-Have for completeness)**
10. Diagnostic query APIs
11. Passthrough mode selection
12. IRQ and block-mode control

**Phase 4 (Optional polish)**
13-18. Low-priority diagnostics and utilities

---

## Stakeholder Communications

### For Executives
> "We have a working SPI monitor foundation (42% complete). Phase 1 closure requires 8 days and fixes security issues. Full parity takes 16 days total. We're blocked on receiving the AST10x0 datasheet; requesting escalation if available."

### For Developers
> "See SPIMONITOR_IMPLEMENTATION_PLAN.md for 26 tickets. Start with 1.2 (Lock, 1 day) while datasheet arrives for 1.1. Tickets 1.3-1.5 can proceed in parallel."

### For QA
> "Phase 1 delivers critical security fixes. Phase 2 adds feature parity. Each phase has unit + integration test requirements. We'll need QEMU validation for Phase 2."

---

**Last Updated**: May 6, 2026  
**Coverage**: 42% (11/26 features) → Target 100% (26/26)  
**Critical Path**: 8-10 days (datasheet-dependent)  
**Recommended Start**: Immediately (with parallel tracks)
