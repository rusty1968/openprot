# AST10x0 SPI Monitor: Overview and Usage Model

> **Status (2026-05-06):** API design complete. Phases 1–3 implemented and compiling.
> See [Implementation Status](#implementation-status) for details. Semantic gaps
> vs. Zephyr driver have been closed; see `review-against-aspeed-zephyr.md`.

## Purpose

The AST10x0 SPI monitor blocks (SPIPF1..SPIPF4) are policy and observability
peripherals that sit on SPI paths and enforce command/address constraints while
collecting transaction context. They are not flash controllers themselves. The
SMC/FMC/SPI controllers move data; SPI monitor blocks gate and inspect the
traffic.

In this crate, the SPI monitor module is intended to provide a small, auditable,
register-first API for:

- command allow-list enforcement
- read/write address privilege filtering
- monitor routing and passthrough configuration
- lock-down of policy registers after setup

## What The Monitors Are

On AST10x0, each monitor instance has an independent register block:

- SPIPF1 at 0x7E79_1000 (SPIM0)
- SPIPF2 at 0x7E79_2000 (SPIM1)
- SPIPF3 at 0x7E79_3000 (SPIM2)
- SPIPF4 at 0x7E79_4000 (SPIM3)

These blocks typically expose:

- global monitor control bits
- command table entries (allow / lock / valid-once behavior)
- address filter/privilege tables for read and write paths
- register write-lock bits for tamper resistance after configuration

The monitor should be treated as a policy firewall for SPI transactions, not a
transport endpoint.

## Why We Need Them

### 1) Platform Security Policy Enforcement

Firmware often needs tighter control than "all SPI opcodes are legal."
Examples:

- forbid erase/program opcodes in runtime regions
- allow read-only opcodes for immutable code partitions
- permit updates only in bounded windows during authenticated update flow

Without SPI monitor policy, software bugs in upper layers can accidentally
issue destructive commands.

### 2) Least-Privilege Runtime Behavior

A monitor enables enforcing different policies across lifecycle states:

- boot: broader command set for provisioning/bring-up
- runtime: reduced command set and locked policy
- recovery/update: temporarily expanded permissions under strict checks

This supports a minimal-TCB posture by making policy hardware-enforced rather
than purely convention-based.

In practical terms, this means SPI monitor configuration is expected to be
static after trusted setup. Boot or provisioning code may program command
tables, address filters, passthrough settings, and related route controls, but
steady-state runtime should treat that configuration as fixed. The monitor is
meant to act as a hardware guardrail that is established once, verified, and
then left in place for the rest of the boot session.

This is an intentional design choice, not just a convenience. If runtime code
could freely rewrite monitor policy, the monitor would stop being a reliable
enforcement boundary and would instead become another mutable software-owned
configuration surface. The desired posture is therefore: configure early,
validate, lock, and operate under that locked policy.

### 3) Operational Safety and Debugging

Even when full blocking is not enabled, monitor routing and status are valuable
for diagnostics:

- prove policy was programmed before handoff
- verify lock bits are asserted
- trace command-table state during test and failure triage

## Usage Model In This Crate

The intended model mirrors other peripheral modules in this repository:

1. Register Layer (current starting point)
- thin, typed wrapper around PAC register blocks
- single unsafe perimeter for pointer ownership
- safe raw read/modify/write helpers by register intent

2. Type/Policy Layer
- enums/bitflags for command slot state, privilege mode, lock policy,
  passthrough/routing mode
- explicit error enums for invalid slot/index/state transitions

3. Controller/Workflow Layer
- lifecycle APIs that enforce legal sequencing, for example:
  configure -> validate -> lock -> operational
- helper methods for common policy profiles (read-only runtime,
  update-window-enabled, manufacturing-open)

4. Integration Layer
- wiring from SMC-facing code paths or platform bring-up code
- optional test helpers for host/QEMU where monitor behavior is partially
  modeled

## Implemented API Surface

The following operations are available on `SpiMonitor<Configured>` and `SpiMonitor<Locked>`:

### Core lifecycle
- `SpiMonitor<Uninitialized>::new(controller)` → construct
- `SpiMonitor<Uninitialized>::apply_policy(policy)` → `SpiMonitor<Configured>` (programs command table and address filters)
- `SpiMonitor<Configured>::lock()` → `SpiMonitor<Locked>` (one-way transition)

### Monitor control (Configured state)
- `enable()` / `disable()` — global monitor enable bit
- `set_ext_mux(ExtMuxSel)` — route SPI path (Sel0 vs Sel1)
- `set_passthrough(PassthroughMode)` — bypass filter or enforce policy
- `drain_log(&mut [ViolationLogEntry])` — read violation log for diagnostics

### Monitor control (Locked state)
- `set_ext_mux(ExtMuxSel)` — mux switching available in locked state for boot transitions
- `set_passthrough(PassthroughMode)` — passthrough available in locked state for boot transitions
- `drain_log(&mut [ViolationLogEntry])` — read violation log

### Policy model
- `MonitorPolicy` struct with command allow-list and address regions
- `RegionPolicy` with direction (Read/Write), operation (Enable/Disable), start address, length
- `profile.rs` built-in: `runtime_read_only()`, `firmware_update_window()`
- `PrivilegeOp::Enable` / `Disable` for region policies
- `PassthroughMode::Enabled` / `Disabled` for filter bypass
- `ExtMuxSel::Sel0` / `Sel1` for mux routing

## Design Rationale

### Why passthrough and mux control are available in Locked state

Both `set_passthrough()` and `set_ext_mux()` are callable on `SpiMonitor<Locked>`. This is intentional:

- Boot-hold and boot-release sequences in platform code require mux switching
  and filter bypass *after* policy is locked.
- Locking policy (command table, address regions) prevents policy *edits* but
  not transient control of the SPI path ownership.
- In process-isolation designs, these routing operations may be separate from
  policy management and need to be available post-lock.

### Why violation log drain is available in Configured state

`drain_log()` is available on both `Configured` and `Locked` for diagnostics
during bring-up and runtime attestation. The crate provides only the synchronous
data-plane (reading log words from MMIO and decoding them). Interrupt
registration, workqueue deferral, and log-pointer reset are platform
responsibilities.

## Constraints and Reality Checks

- QEMU coverage for SPI monitor functionality may be incomplete; policy logic
  must still be tested via unit/host tests and silicon validation.
- Register lock semantics are one-way in many flows; APIs make this explicit
  through typestate (`Configured` → `Locked`).
- This module stays focused on monitor policy. Flash transport and DMA remain
  in SMC/SPI controller modules.
- Register bit positions and offsets (control, passthrough, mux, lock) are
  currently placeholders and require AST10x0 datasheet confirmation.
- The Zephyr driver surface is covered except for `aspeed_spi_monitor_sw_rst`
  (software reset), which remains an open gap.

## Relationship To Zephyr-Style Systems

In monolithic-kernel designs, one driver may configure all monitors and all
controllers in one address space. In this repository's direction, monitor
configuration is still centralized in trusted code, but runtime use should
preserve minimal-TCB boundaries by avoiding unnecessary cross-controller shared
state.

The monitor crate therefore acts as a reusable hardware-policy component with a
clear separation from flash data-plane logic.

## Proposed Module Tree

The SPI monitor module should follow the same layering pattern used by `smc/`.

```text
target/ast10x0/peripherals/spimonitor/
  mod.rs
  registers.rs
  types.rs
  controller.rs
  policy.rs
  profile.rs
  planning/
    overview-and-usage-model.md
    implementation-plan.md
```

### File Responsibilities

- `mod.rs`
  - public module surface and curated re-exports
  - keeps consumers from depending on internal file boundaries

- `registers.rs`
  - low-level PAC-backed register wrapper
  - single unsafe perimeter for register block ownership
  - raw read/write/modify methods by register intent

- `types.rs`
  - all public enums/bitflags/errors for monitor semantics
  - examples: monitor instance id, allow-command flags, privilege direction,
    lock status, policy validation errors

- `controller.rs`
  - stateful typed facade over `registers.rs`
  - sequencing guardrails: init -> configure -> lock -> operational
  - table manipulation helpers with bounds/lock checks

- `policy.rs`
  - pure data policy model and validators (no MMIO)
  - enables host-side testing of policy logic independent of hardware

- `profile.rs`
  - predefined policy bundles for common use cases
  - examples: `runtime_read_only`, `firmware_update_window`,
    `manufacturing_open`

- `planning/implementation-plan.md`
  - phased bring-up checklist and test matrix

## Usage Model By Layer

1. **Policy construction** (`policy.rs`, `profile.rs`)
   - Build or select a policy profile (e.g., `runtime_read_only()`) in trusted setup code.
   - Add region entries via `add_region()` if provisioned policy is available.
   - `MonitorPolicy` is a pure-data struct, testable on the host without hardware.

2. **Hardware apply** (`controller.rs` + `registers.rs`)
   - `SpiMonitor::new(controller)` acquires ownership of the SPIPF register block.
   - `apply_policy(policy)` programs both command table and address filter slots.
   - Returns `SpiMonitor<Configured>`, now in a mutable but not-yet-locked state.

3. **Pre-lock diagnostics** (optional)
   - Call `enable()` to arm the monitor.
   - Optionally `drain_log()` to verify no spurious violations are logged.
   - Set mux and passthrough to initial state if needed.

4. **Policy lock**
   - Call `lock()` on `Configured` to transition to `Locked`.
   - Write-disable bits are asserted in the hardware (bit position TBD).
   - No further changes to policy tables are possible (enforced by SPIPF hardware).

5. **Runtime operation**
   - Monitor remains active as hardware guardrail.
   - No dynamic policy edits are possible.
   - Mux and passthrough *can* be changed (for boot transitions).
   - Log drain is available for diagnostics and attestation.
   - If a future authenticated update or recovery mode is needed, it must be an
     explicit lifecycle transition owned by trusted code — not ad-hoc
     reconfiguration from a locked state.

## Relationship to Zephyr SPIM Driver

This crate implements the full data-plane of the Zephyr `spi_monitor_aspeed`
driver at the Rust/PAC level. The mapping is:

| Zephyr function | Rust equivalent |
|---|---|
| `spim_monitor_enable(dev, bool)` | `enable()` / `disable()` |
| `spim_passthrough_config(dev, 0, bool)` | `set_passthrough(PassthroughMode)` |
| `spim_ext_mux_config(dev, sel)` | `set_ext_mux(ExtMuxSel)` |
| `spim_address_privilege_config(dev, rw, op, addr, len)` | Region in `MonitorPolicy`, applied via `apply_policy` |
| `spim_get_log_info` + drain loop | `drain_log(&mut buf)` |
| `spim_isr_callback_install` | Out of scope — platform layer |
| `aspeed_spi_monitor_sw_rst` | Open gap — to be implemented |

The crate does not replace the Zephyr driver; it provides an equivalent interface
for non-Zephyr (e.g., `reference`/`openprot`) Rust firmware.

## Implementation Status

### Phase 1: Register and Type Foundation ✅ COMPLETE

- [x] `registers.rs` covers SPIPF control, lock, command table, address filter, and log registers
- [x] `types.rs` defines `PrivilegeDirection`, `PrivilegeOp`, `PassthroughMode`, `ExtMuxSel`, `ViolationLogEntry`
- [x] `types.rs` includes `parse_log_word` for violation log decoding

### Phase 2: Controller Semantics ✅ COMPLETE

- [x] `controller.rs` implements typestate lifecycle: `Uninitialized` → `Configured` → `Locked`
- [x] `enable()` / `disable()` for monitor control
- [x] `set_passthrough()` available on both `Configured` and `Locked`
- [x] `set_ext_mux()` available on both `Configured` and `Locked`
- [x] `drain_log()` available on both states for diagnostics
- [x] `lock()` transition enforces one-way policy lockdown

### Phase 3: Policy and Profiles ✅ COMPLETE

- [x] `policy.rs` defines `MonitorPolicy` with command table and region array
- [x] `policy.rs` includes `add_region()` helper for region management
- [x] `profile.rs` provides `runtime_read_only()` and `firmware_update_window()`
- [x] Region encoding function in `controller.rs` converts policy to hardware word format

### Phase 4: Integration and Verification ⏳ IN PROGRESS

- [ ] Wire monitor setup into reference platform bring-up path
- [ ] Add host-side policy tests (region overlap, bounds checking)
- [ ] Add QEMU register readback tests
- [ ] Silicon validation checklist for lock permanence, routing behavior

### Open Gaps

- ⚠️ `aspeed_spi_monitor_sw_rst` equivalent not yet implemented (software reset)
  - Requires: `registers.rs` helper to trigger reset bit, `controller.rs` method
  - Used in: boot-release sequence when transferring monitor to host
  - Severity: Medium — needed for boot sequencing

- ⚠️ Register bit/offset placeholders require datasheet confirmation
  - Control bits (enable, passthrough, mux, lock): currently bits 0–2, 31
  - Log register offsets (index, size, RAM addr): currently 0x080, 0x084, 0x088
  - Address filter slot encoding: direction/op/address/length packing (TBD)
  - Severity: High — functionality depends on correct values
