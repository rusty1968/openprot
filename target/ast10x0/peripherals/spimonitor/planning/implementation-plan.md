# SPI Monitor Implementation Plan

## Lifecycle Justification

The SPI monitor API is intentionally lifecycle-driven:

- `Uninitialized` -> hardware not yet programmed
- `Configured` -> policy tables and control fields staged and validated
- `Locked` -> write-disable bits asserted; runtime policy is immutable

This is not only an API preference. It is required by hardware behavior and
minimal-TCB goals.

### Why explicit lifecycle states are required

1. One-way hardware lock semantics
- SPIPF lock bits are effectively write-once for the boot/runtime session.
- If code can call lock-related operations out of order, policy may become
  partially programmed and permanently stuck.

2. Prevent unsafe intermediate exposure
- Programming command/region tables takes multiple writes.
- Without a staged `Configured` phase and explicit validation, a task could
  start using the monitor while rules are incomplete.

3. Enforce least-privilege steady state
- The expected runtime posture is read-mostly, policy-stable operation.
- Requiring a transition to `Locked` formalizes that post-boot mutable control
  is not part of normal operation.

4. Constrain bug blast radius in trusted code
- A typed lifecycle blocks classes of misuse at compile-time and runtime:
  - lock before tables are written
  - reprogram after lock
  - skipping readback checks

5. Improve auditability and attestation
- Lifecycle boundaries provide clear checkpoints for logs and tests:
  - "policy applied"
  - "policy verified"
  - "policy locked"
- This supports security review and incident triage.

### Operational model implied by lifecycle

- Boot/provisioning code owns transitions from `Uninitialized` to `Configured`.
- A validation gate (readback and invariants) is required before `Locked`.
- Runtime code consumes status from the locked monitor but does not mutate it.
- Any future "update mode" must be an explicit, authenticated state machine,
  not ad-hoc writes to locked policy tables.

Said differently: the intended operational model is static configuration after
bring-up. The SPI monitor should be programmed during trusted initialization,
verified, and then treated as read-only policy state for the remainder of the
session. This applies both to policy tables and to SCU-backed routing or
passthrough choices associated with the monitor.

The current repository scaffold is not yet the full enforcement point for that
rule. The typestate API already models `Configured` and `Locked` as distinct
states, but some raw register access remains exposed while register coverage and
precise lock semantics are still being finished. That gap should be treated as
an implementation TODO, not as permission to rely on dynamic post-lock
reconfiguration.

### Test implications

- Unit tests: verify invalid transitions fail deterministically.
- Integration tests: verify lock permanence and no post-lock writes.
- Silicon validation: verify lock bits and policy table contents survive the
  full boot-to-runtime handoff window.

## Phase 1: Foundations

- Complete `registers.rs` accessors for required SPIPF registers.
- Stabilize public enums and errors in `types.rs`.
- Keep unsafe access contained to register constructor perimeter.

## Phase 2: Controller API

- Provide lifecycle API in `controller.rs`:
  - construct
  - apply policy
  - lock
  - read back status
- Validate transitions and lock semantics.
- Tighten raw register exposure so post-lock APIs become observational rather
  than mutating.

## Phase 3: Policy Model

- Introduce declarative policy object in `policy.rs`.
- Add predefined profiles in `profile.rs` for runtime and update modes.
- Add unit tests for policy encoding and bounds checks.

## Phase 4: Integration

- Hook monitor policy into trusted bring-up sequence.
- Add readback assertions in integration tests.
- Document silicon-only checks for lock permanence and routing behavior.

---

## Gap Closure Plan

This section specifies the concrete changes needed to make the Rust scaffold
semantically equivalent to the Zephyr SPIM driver surface. Each gap is a
self-contained work item with defined file targets and acceptance criteria.

The Zephyr API reference is:
- `spim_monitor_enable(dev, bool)`
- `spim_passthrough_config(dev, flags, bool)`
- `spim_ext_mux_config(dev, spim_ext_mux_sel)`
- `spim_address_privilege_config(dev, rw_select, op, addr, len)`
- `spim_isr_callback_install(dev, fn)` — platform layer only, not in this crate
- `spim_get_log_info(dev, &info)` / log drain loop
- `spim_get_ctrl_idx(dev)`

### Gap A — Monitor enable/disable

**Zephyr:** `spim_monitor_enable(dev, bool)` — controls global enable bit in
SPIPF000.

**Changes:**
- `registers.rs`: Add `read_ctrl_enable_bit()` / `write_ctrl_enable_bit(bool)`
  helpers that isolate bit 0 of SPIPF000 with a read-modify-write. No new raw
  accessor needed; reuse `modify_ctrl`.
- `controller.rs`: Add `fn enable(&self)` and `fn disable(&self)` to
  `SpiMonitor<Configured>`. These are intentionally available on `Configured`
  only; enabling the monitor before policy tables are staged is a logic error.
  `SpiMonitor<Locked>` does not expose `disable` because runtime
  de-activation should be a deliberate authenticated action — callers with a
  `LockedSpiMonitor` must consume a separate platform-level escape hatch if
  needed.
- `types.rs`: No new type required.

**Acceptance:** `SpiMonitor<Configured>::enable()` sets bit 0 of SPIPF000;
`disable()` clears it. Readback confirms value. Unit test on a mock register
word.

---

### Gap B — Passthrough mode

**Zephyr:** `spim_passthrough_config(dev, flags, bool)` — bypasses the monitor
filter and routes SPI traffic unfiltered. Called in `gpio_ctrl.c` during mux
ownership transitions.

**Changes:**
- `types.rs`: Add `pub enum PassthroughMode { Enabled, Disabled }`.
- `registers.rs`: Add `read_passthrough_bit()` / helpers that target the
  passthrough enable field in SPIPF000 (confirm exact bit from datasheet;
  placeholder pending silicon spec).
- `controller.rs`: Add `fn set_passthrough(&self, mode: PassthroughMode)` to
  both `SpiMonitor<Configured>` and `SpiMonitor<Locked>`. Passthrough is a
  runtime routing control that must survive the lock transition — disabling
  the filter momentarily during a mux switch is a legitimate locked-state
  operation.

**Acceptance:** `set_passthrough(PassthroughMode::Enabled)` writes expected
register field. State readable back via `read_passthrough_bit()`.

**Note:** `flags` parameter in Zephyr is currently always `0` in all call
sites; no bitmask expansion is needed yet. If the silicon register exposes
per-direction passthrough flags, this API should be extended at that time.

---

### Gap C — External mux selection

**Zephyr:** `spim_ext_mux_config(dev, mux_sel)` where `mux_sel` is
`SPIM_EXT_MUX_SEL_0` or `SPIM_EXT_MUX_SEL_1`. Platform headers alias
`SPIM_EXT_MUX_ROT` and `SPIM_EXT_MUX_BMC_PCH` to `SEL_0`/`SEL_1` with
optional polarity inversion (`CONFIG_SPI_MUX_INVERSE`).

**Changes:**
- `types.rs`: Add `pub enum ExtMuxSel { Sel0, Sel1 }`. Do not add ROT/BMC/PCH
  aliases here — those are platform-layer policy decisions, not peripheral
  semantics.
- `registers.rs`: Add `read_ext_mux_sel()` / `write_ext_mux_sel(ExtMuxSel)`
  helpers targeting the mux select field in SPIPF000 (confirm bit position from
  datasheet).
- `controller.rs`: Add `fn set_ext_mux(&self, sel: ExtMuxSel)` to
  `SpiMonitor<Configured>` and `SpiMonitor<Locked>`. Mux selection must be
  changeable in locked state for the boot-hold/release flow to work.

**Acceptance:** `set_ext_mux(ExtMuxSel::Sel0)` and `set_ext_mux(Sel1)` produce
distinct register values confirmed by readback.

---

### Gap D — Privilege region operation (enable vs disable)

**Zephyr:** `spim_address_privilege_config(dev, rw_select, op, addr, len)`
where `op` is `SPI_FILTER_PRIV_ENABLE` or `SPI_FILTER_PRIV_DISABLE`. Both are
called in `intel_pfr_spi_filtering.c` — disabling a region is as important as
enabling one for PFM compliance.

**Changes:**
- `types.rs`: Add `pub enum PrivilegeOp { Enable, Disable }`. Add `pub op:
  PrivilegeOp` field to `RegionPolicy`. Update all existing constructors and
  tests.
- `policy.rs`: Update `MonitorPolicy::regions` entries to carry `op`. Add a
  helper `RegionPolicy::allow(start, length, direction)` and
  `RegionPolicy::deny(start, length, direction)` for ergonomic construction.
- `controller.rs`: In `apply_policy`, the address filter loop (currently
  missing — see Gap E) must pass `op` to `write_addr_filter_slot`. The
  encoding for enable vs disable in the hardware register must be confirmed from
  the datasheet; placeholder: `0` = privilege active, `1` = blocked.
- `profile.rs`: Update `runtime_read_only` and `firmware_update_window` to
  explicitly set `op` on any region entries added in future.

**Acceptance:** `RegionPolicy` with `PrivilegeOp::Disable` programs the filter
slot to block access. Unit test confirms the register word differs from the
`Enable` case.

---

### Gap E — Region policy application missing from `apply_policy`

**Current bug:** `SpiMonitor<Uninitialized>::apply_policy` programs the command
table but never applies `policy.regions`. The loop body is absent.

**Changes:**
- `controller.rs`: In `apply_policy`, after the command-table loop, iterate
  `policy.regions`. For each `Some(region)`, encode the start address, length,
  direction (`PrivilegeDirection`), and op (`PrivilegeOp`) into the hardware
  address filter slot format and call `regs.write_addr_filter_slot(i, word)`.
  Return `Err(SpiMonitorError::InvalidRegion)` if the slot index exceeds the
  hardware table depth (confirm from datasheet; assume 16 entries for now).

**Acceptance:** After `apply_policy` with a region entry, `read_addr_filter_slot`
returns the encoded value. Host unit test using a fake register store.

---

### Gap F — Violation log data types and register accessors

**Zephyr:** `spim_get_log_info` returns a struct with `log_ram_addr`,
`log_max_sz`, `log_idx_reg`. Log words are decoded as:
- bits[19:18] == 0x0 → blocked command, bits[7:0] = opcode
- bits[19:18] == 0x1 → blocked write address, bits[17:0] << 14 = address
- bits[19:18] == 0x2 → blocked read address, bits[17:0] << 14 = address

This decoding belongs in the peripheral crate. The ISR/workqueue plumbing does
not.

**Changes:**
- `types.rs`: Add:
  ```rust
  pub enum ViolationLogEntry {
      BlockedCommand(u8),
      BlockedWriteAddr(u32),
      BlockedReadAddr(u32),
      Invalid(u32),
  }
  impl ViolationLogEntry {
      pub fn parse(word: u32) -> Self { ... }
  }
  ```
- `registers.rs`: Add accessors:
  - `read_log_idx_reg() -> u32` — current write pointer / count register
  - `read_log_max_sz() -> u32` — maximum log capacity in bytes
  - `log_ram_base_addr() -> usize` — base address of the violation log RAM
    (returns a `usize`; caller casts to `*const u32` when draining)
  - These are separate SPIPF registers; confirm addresses from datasheet.
- `controller.rs`: Add `fn drain_log<'a>(&self, buf: &'a mut [ViolationLogEntry]) -> &'a [ViolationLogEntry]`
  to `SpiMonitor<Locked>` (and `SpiMonitor<Configured>` for diagnostic use).
  The method reads `read_log_idx_reg()` to determine how many entries are
  available, iterates up to `buf.len()` words from log RAM, parses each with
  `ViolationLogEntry::parse`, fills `buf`, and returns the filled slice.
  No allocation. No OS primitives. Caller is responsible for synchronization
  and log-pointer reset.

**Out of scope for this crate:**
- ISR callback registration
- Workqueue deferral
- Semaphore-protected log-pointer reset
- BMC-reset-triggered pointer reset (`demo_rst_log_ptr` equivalent)

**Acceptance:** `ViolationLogEntry::parse` is unit-tested against each of the
three bit-field patterns plus an invalid word. `drain_log` is integration-tested
on a simulated register store.

---

## Implementation Order

The gaps should be addressed in this sequence to minimize merge conflicts and
keep each change reviewable in isolation:

1. **Gap D** (add `PrivilegeOp` to types) — pure type change, no register work
2. **Gap E** (region loop in `apply_policy`) — depends on Gap D for `op`
3. **Gap F types** (`ViolationLogEntry` + `parse`) — pure type/logic, no hardware dependency
4. **Gap A** (enable/disable) — narrow register + controller change
5. **Gap B** (passthrough) — narrow register + controller change
6. **Gap C** (ext mux) — narrow register + controller change
7. **Gap F registers + drain_log** — depends on F types, A/B/C can land first

Each item is a single PR / commit group. No item has a hard dependency on
items later in the list.
