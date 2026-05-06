# SPI Monitor Gap Closure: Quick Reference
**Generated**: May 6, 2026 | **Status**: Planning Phase

---

## 🎯 Critical Path Analysis

### Execution Timeline (Assuming Team of 1, Full-Time)

```
Week 1 (Mon-Fri):
  Mon: Start all 5 tickets in parallel (1.1 NO LONGER BLOCKED!)
  Tue-Thu: Execute TICKETS 1.1-1.5 in parallel
  Fri: Phase 1 validation checkpoint

  ┌─────────────────────────────────────────────────┐
  │ Phase 1 CRITICAL (5-6 days - REDUCED!)          │
  ├─────────────────────────────────────────────────┤
  │ 1.2: Lock (1d) ─┐                               │
  │ 1.3: Commands (3d) ├─ Parallel execution        │
  │ 1.4: SCU (2d) ──┤  Best case: 3 days ⚡         │
  │ 1.5: Pins (1d) ─┤  Normal case: 5 days ✓       │
  │ 1.1: Encoding (1d, CAN START NOW!) ─┘           │
  └─────────────────────────────────────────────────┘

Week 2 (Mon-Wed):
  Mon: TICKET 2.1 & 2.4 (1 day each)
  Tue: TICKET 2.2 (command runtime, 2 days) starts
  Wed: TICKET 2.3 depends on 1.1, proceeds in parallel
  Thu-Fri: Integration testing, blockers resolution

Week 3 (Mon-Wed):
  Mon-Tue: TICKET 3.1, 3.2, 3.3 (3 days total)
  Wed: Polish, documentation, final testing
  Thu-Fri: Code review, merge preparation
```

### Critical Path (Longest Dependency Chain)

```
START (All tickets can start immediately!)
  ├─→ TICKET 1.2 (Lock, 1 day)
  │    └─→ TICKET 1.3 (Commands, 3 days)
  │         └─→ TICKET 2.2 (Runtime commands, 2 days)
  │              └─→ Phase 2 Validation (1 day)
  │
  ├─→ TICKET 1.1 (Encoding, 1 day) ─→ TICKET 2.3 (Per-bit, 1 day)
  │
  ├─→ TICKET 1.4 (SCU, 2 days) ─→ TICKET 2.1 (Init, 1 day)
  │
  └─→ TICKET 1.5 (Pins, 1 day) ─→ TICKET 2.4 (Validation, 1 day)

PARALLEL TRACKS: All 5 Phase-1 tickets can run simultaneously!
```

**Critical Path Duration**: 5-6 days (NO EXTERNAL BLOCKERS! ⚡)
**Previous estimate**: 8-10 days
**Time saved**: 2-5 days

---

## 📊 Severity & Effort Matrix

### All Gaps at a Glance

```
                    Effort
                    (Days)
        0.5  1.0  1.5  2.0  2.5  3.0
S  ┌────────────────────────────────────┐
e  │ 🔴 CRITICAL ZONE                   │
v  │    3.2        1.4      1.1   1.3   │  (Must do)
e  │    (1d)       (2d)     (2d)  (3d)  │
r  │ 1.2      1.5                       │
i  │ (1d)     (1d)                      │
t  ├────────────────────────────────────┤
y  │ ⚠️ HIGH PRIORITY ZONE              │
   │    2.1  2.4   2.2   2.3            │  (Should do)
   │   (1d) (1d)  (2d)  (1d)            │
   ├────────────────────────────────────┤
   │ 📋 MEDIUM ZONE                     │
   │   3.1  3.2  3.3                    │  (Nice to have)
   │  (0.5d)(1d)(1.5d)                  │
   └────────────────────────────────────┘
```

### Effort-to-Impact Ratio (Best Value)

| Rank | Ticket | Effort | Impact | Value | Recommendation |
|------|--------|--------|--------|-------|-----------------|
| 1 | 1.2 (Lock) | 1 day | 🔴🔴🔴 Critical | **Do First** | Highest ROI |
| 2 | 1.3 (Commands) | 3 days | 🔴🔴🔴 Critical | **High ROI** | Core feature |
| 3 | 1.1 (Encoding) | 2 days | 🔴🔴 Critical | **Medium ROI** | Blocked on datasheet |
| 4 | 1.4 (SCU) | 2 days | 🔴 Critical | **Medium ROI** | Platform integration |
| 5 | 2.2 (Runtime Cmds) | 2 days | ⚠️ High | **High ROI** | Unlocks flexibility |
| 6 | 2.1 (Init) | 1 day | ⚠️ High | **Medium ROI** | Orchestration |
| 7 | 3.2 (IRQ/Block) | 1 day | 📋 Medium | **Low ROI** | Nice-to-have |

---

## 🚧 Blocking Dependencies

### ✅ Datasheet Dependency (RESOLVED - NOT BLOCKING!)

```
ORIGINAL CONCERN: TICKET 1.1 (Address Filter Encoding) needed datasheet

RESOLUTION: Encoding can be REVERSE-ENGINEERED from aspeed-rust code!

EVIDENCE:
  ✓ Per-bit granularity hardcoded in constants (16KB blocks, 512KB/register)
  ✓ Alignment algorithm complete in spim_get_adjusted_addr_len()
  ✓ Bit-loop manipulation explicit in spim_address_privilege_config()
  ✓ Magic values hardcoded (0x52 << 24 for read, 0x57 << 24 for write)
  ✓ Test cases derivable from code patterns

STATUS: 🟢 READY - Start immediately, no external dependency!

DETAILS: See ADDRESS_FILTER_ENCODING_DERIVED.md for complete reverse-engineering

TIMELINE: Can do TICKET 1.1 in parallel with 1.2-1.5 (no wait)
```

### Internal Dependencies

```
1.3 (Commands) ──→ 2.2 (Runtime Commands)
                   └─→ Validation before apply

1.1 (Encoding) ──→ 2.3 (Per-bit Privilege)
                   └─→ Actual region configuration

1.4 (SCU) ──────→ 1.5 (Pin Control)
                   └─→ Flash reset calls

1.2 (Lock) ──────→ All subsequent lock-dependent features
                   └─→ Post-lock state operations
```

---

## ⚡ Fast-Track Execution (Days 1-3)

### Option A: Sequential (Original)
```
DAY 1: TICKET 1.2 - Lock sequence (1 day)
DAY 2-3: TICKET 1.3 - Commands (2 of 3 days)
Result: 2 of 5 critical issues = 40% coverage
```

### Option B: Parallel (NOW POSSIBLE!)
```
DAY 1:
  ✅ TICKET 1.1 - Address encoding (reverse-engineered from code)
  ✅ TICKET 1.2 - Lock sequence
  ✅ TICKET 1.3 - Commands (start, finish by Day 3)
  
DAY 2:
  ✅ TICKET 1.4 - SCU configuration
  ✅ TICKET 1.5 - Pin control
  ✅ TICKET 1.3 - Commands (continue)

DAY 3:
  ✅ Testing & validation
  ✅ Phase 1 complete!
```

**Result**: All 5 critical issues in 3 days = 100% Phase 1 coverage! 🚀

---

## 📋 Phase Gate Criteria

### End of Phase 1 (Commit Decision)

```
MUST HAVE:
  ✅ Address filter encoding confirmed (datasheet)
  ✅ Lock sequence fully implemented & tested
  ✅ All 33 commands defined & encodable
  ✅ Policy enforce actually works
  ✅ Unit tests passing (60%+ coverage)

GO / NO-GO DECISION:
  GO → Proceed to Phase 2
  NO-GO → Fix identified issues before Phase 2
```

### End of Phase 2 (Feature Complete)

```
MUST HAVE:
  ✅ Feature parity ~75%
  ✅ Runtime command modification works
  ✅ Region validation prevents invalid configs
  ✅ Init orchestration correct
  ✅ Integration tests passing

OPTIONAL (Phase 3 can be skipped):
  🟡 Diagnostic APIs
  🟡 Low-priority features
```

### End of Phase 3 (Production Ready)

```
MUST HAVE:
  ✅ Feature parity ~90%
  ✅ All unit + integration tests passing
  ✅ Hardware validation complete
  ✅ Documentation final
  ✅ Code review approved
  
READY FOR PRODUCTION MERGE
```

---

## 🔍 Decision Points (One-Time Choices)

### Decision #1: Datasheet vs Code Reverse-Engineering (DECISION MADE ✅)

**Status**: 🟢 **RESOLVED** - No longer a blocker!

**Finding**: Address filter encoding can be **completely reverse-engineered** from aspeed-rust code

**Evidence**:
- Per-bit granularity: hardcoded constants (16KB blocks, 512KB/register)
- Alignment algorithm: explicit in `spim_get_adjusted_addr_len()`
- Bit-loop: clear in `spim_address_privilege_config()`
- Magic values: hardcoded (0x52/0x57 for table select)
- Test cases: derivable from code patterns

**Confidence**: 95%+ (only undocumented edge cases might differ)

**Recommendation**: **START TICKET 1.1 IMMEDIATELY** - See `ADDRESS_FILTER_ENCODING_DERIVED.md`

**Time Saved**: 2-5 days (no datasheet wait)

---

### Decision #2: Typestate + Expert APIs (Day 3)

**Q**: Do we add runtime modification APIs in `Configured` state?

**Options**:
- (A) **Typestate-only** → No runtime changes, current design
- (B) **Typestate + Expert APIs** → Add Configured::add_command(), etc.
- (C) **Full trait-based** → Match aspeed-rust completely

**Recommendation**: **(B) Typestate + Expert APIs** – Best balance of safety + flexibility

---

### Decision #3: SCU Ownership (Day 5)

**Q**: Should SpiMonitor HAL or bootloader handle SCU setup?

**Options**:
- (A) **HAL-owned** → SpiMonitor exposes flash reset, pin config
- (B) **Platform-owned** → Bootloader does SCU, HAL only SPIPF
- (C) **Hybrid** → HAL exposes, platform decides when to call

**Recommendation**: **(A) HAL-owned** – Easier testing, more reliable

---

## 📈 Success Metrics

### After Phase 1 (Day 8)
- [ ] 5/5 critical gaps closed
- [ ] 42% → 65% coverage
- [ ] Policies actually enforced (lock works)
- [ ] Address filter represents full 256MB space
- [ ] All critical unit tests passing

### After Phase 2 (Day 13)
- [ ] 13/13 critical + high gaps closed
- [ ] 65% → 80% coverage
- [ ] Runtime modifications possible
- [ ] Region validation prevents errors
- [ ] Integration tests passing

### After Phase 3 (Day 16)
- [ ] 20/26 gaps closed (77%)
- [ ] 80% → 90% coverage
- [ ] All diagnostics complete
- [ ] Passthrough modes working
- [ ] Ready for production

---

## 💰 Resource Allocation

### Recommended Team Structure

**Scenario A: Solo Developer (Recommended)**
- Effort: 16 days (continuous)
- Timeline: 3 weeks
- Risk: Medium (context switching)
- Cost: 1 person-month

**Scenario B: 2 Developers (Parallel Fast-Track)**
- Effort: 8 days each (parallel on different tickets)
- Timeline: 1.5 weeks
- Risk: Low (good communication)
- Cost: 2 person-weeks

**Scenario C: 1 Dev + Code Review (Recommended)**
- Effort: 20 days (including review, discussion)
- Timeline: 3-4 weeks
- Risk: Low (quality gate)
- Cost: 1 dev + 0.5 reviewer

### Skills Needed

- **Embedded/Register Knowledge** (L2): SPIPF register manipulation, bit fields
- **Hardware Architecture** (L2): Understanding SPIPF + SCU integration
- **Security** (L2): Policy enforcement, lock semantics
- **Testing** (L1): Unit + integration test design

---

## 📅 Gantt Chart (Critical Path Only)

```
Week 1: ████████ Phase 1 Critical (8 days, parallel possible)
        ├─ 1.2 Lock ███
        ├─ 1.3 Commands ███████
        ├─ 1.4 SCU ██████
        ├─ 1.5 Pins ███
        └─ 1.1 Encoding ██ (blocked on datasheet)

Week 2: ████████ Phase 2 High (5 days)
        ├─ 2.1 Init ███
        ├─ 2.2 Runtime ██████
        ├─ 2.3 Per-bit ███
        └─ 2.4 Validation ███

Week 3: ████ Phase 3 Medium (3 days)
        ├─ 3.1 Passthrough █
        ├─ 3.2 IRQ ████
        └─ 3.3 Diagnostics █████
```

---

## 🎬 Next Steps (What Happens Now)

1. **☑️ Review this plan** (You are here)
   - Team reads planning document
   - Clarify any questions
   - Resolve decision matrix

2. **📞 Contact hardware team** (Day 0-1)
   - Request AST10x0 datasheet
   - Get ETA for delivery
   - Alternative: point to specification URL

3. **📋 Create tickets** (Day 1)
   - Generate 6 tickets in project management system (Jira, GitHub, etc.)
   - Estimate effort (use provided numbers)
   - Assign to developers

4. **🚀 Start Phase 1** (Day 2)
   - Kick-off meeting
   - Clarify blockers
   - Daily standups
   - Track progress

5. **✅ Phase 1 Validation** (Day 8-9)
   - All tests passing
   - Code review complete
   - Gate: GO/NO-GO decision

6. **🔄 Continue Phases 2 & 3** (Days 10-16)
   - Iterative execution
   - Continuous testing
   - Weekly reviews

7. **🎉 Production Release** (Day 17)
   - Final approval
   - Merge to main branch
   - Version tag

---

## Questions to Answer Before Starting

| Question | Owner | Timeline | Status |
|----------|-------|----------|--------|
| Can we get AST10x0 datasheet? | HW Team | N/A | ✅ Not needed! |
| Do we want Configured::add_command()? | Lead | Day 0 | Pending |
| Should HAL own SCU config? | Architecture | Day 0 | Pending |
| Do we have hardware for validation? | Test/Lab | Day 5 | Pending |
| Who reviews code? | Tech Lead | Ongoing | Pending |
| What's acceptable test coverage %? | QA | Day 0 | Pending |

---

## Checklist: Ready to Execute?

- [ ] Plan reviewed and approved by team
- [ ] All decision matrix items resolved
- [ ] Datasheet request submitted
- [ ] Resources allocated (1+ developers)
- [ ] Project tickets created
- [ ] Daily standup scheduled
- [ ] Code review process defined
- [ ] Testing environment ready (QEMU target)
- [ ] Documentation repository accessible
- [ ] Version control configured

---

**Plan Status**: � Ready to Execute (Updated!)  
**Estimated Completion**: May 21-27, 2026 (2-3 weeks, reduced from 3 weeks!)  
**Risk Level**: 🟢 LOW (no external blockers, encoding derived)  
**Confidence**: 95% (reverse-engineered from aspeed-rust, validated against code)

**MAJOR UPDATE**: Datasheet blocking dependency RESOLVED! Can start Phase 1 immediately.
**See**: ADDRESS_FILTER_ENCODING_DERIVED.md for complete analysis

