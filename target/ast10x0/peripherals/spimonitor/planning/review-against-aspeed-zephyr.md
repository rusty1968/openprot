# Review: Overview and Usage Model vs. aspeed-zephyr-project

> **Status (2026-05-06):** Initial gaps identified in this review (A–F) have been
> implemented in the Rust crate. See [§ Semantic Gap Closure Status](#semantic-gap-closure-status)
> for the current implementation state. The architecture question (standalone Rust
> HAL vs. Zephyr integration) has been resolved: this crate is a standalone
> `no_std` peripheral HAL for the `reference`/`openprot` Rust projects. It does
> not wrap or replace the Zephyr driver.

## Executive Summary

The overview-and-usage-model.md document proposes a comprehensive Rust-based HAL architecture for SPI monitor policy enforcement with command-table and address-filtering capabilities. The initial scaffold diverged from the Zephyr implementation in three areas:

1. **Language and Architecture**: Rust/register-first PAC bindings vs. Zephyr C driver model — intentional; the crate targets non-Zephyr Rust firmware.
2. **Feature Scope**: Original scaffold emphasised command tables while Zephyr primary usage is address filtering and mux switching — now corrected.
3. **Missing operations**: Passthrough, ext-mux, privilege op (enable/disable), violation log — now implemented.

---

## Semantic Gap Closure Status

The following table maps each Zephyr SPIM driver function to its Rust
equivalent and records implementation status.

| Zephyr function | Rust equivalent | File | Status |
|---|---|---|---|
| `spim_monitor_enable(dev, true/false)` | `SpiMonitor<Configured>::enable()` / `disable()` | `controller.rs` | ✅ implemented |
| `spim_passthrough_config(dev, 0, bool)` | `set_passthrough(PassthroughMode)` on `Configured` + `Locked` | `controller.rs` | ✅ implemented |
| `spim_ext_mux_config(dev, sel)` | `set_ext_mux(ExtMuxSel)` on `Configured` + `Locked` | `controller.rs` | ✅ implemented |
| `spim_address_privilege_config(dev, rw, op, addr, len)` | `RegionPolicy { direction, op, start, length }` applied in `apply_policy` | `controller.rs`, `types.rs` | ✅ implemented |
| `SPI_FILTER_PRIV_ENABLE` / `DISABLE` | `PrivilegeOp::Enable` / `Disable` | `types.rs` | ✅ implemented |
| `SPIM_EXT_MUX_SEL_0` / `SEL_1` | `ExtMuxSel::Sel0` / `Sel1` | `types.rs` | ✅ implemented |
| `spim_get_log_info` + log drain loop | `drain_log(&mut buf)` on `Configured` + `Locked` | `controller.rs` | ✅ implemented |
| Log word decode (bits[19:18]) | `ViolationLogEntry::parse(u32)` | `types.rs` | ✅ implemented |
| Log register accessors | `read_log_idx_reg()`, `read_log_max_sz()`, `log_ram_base_addr()` | `registers.rs` | ✅ implemented (placeholder offsets — needs datasheet confirmation) |
| `spim_isr_callback_install(dev, fn)` | Out of scope — platform layer responsibility | — | 🔲 by design |
| `aspeed_spi_monitor_sw_rst(dev)` | Not yet implemented | `controller.rs` | ⚠️ open gap |
| `spim_get_ctrl_idx(dev)` | `SpiMonitor::controller()` returns `SpiMonitorController` enum | `controller.rs` | ✅ covered by enum |

### Open gap: software reset

`aspeed_spi_monitor_sw_rst` is called in `BMCBootRelease()` and PCH
equivalent to reset the monitor instance after releasing the mux to the
host. There is currently no `sw_reset()` method on any `SpiMonitor` state.

**Required:**
- `registers.rs`: add `trigger_sw_reset()` — write-one-to-clear or
  strobe bit in SPIPF000 (bit position TBD from datasheet).
- `controller.rs`: expose `fn sw_reset(&self)` on `SpiMonitor<Configured>`
  and `SpiMonitor<Locked>`. Reset is a transient control action, not a
  lifecycle transition, so no typestate change is needed.

---

## Detailed Findings

### 1. Architecture: Rust/PAC vs. C/Zephyr Driver Model

**Resolution:** This crate is a standalone `no_std` peripheral HAL targeting
the `reference` and `openprot` Rust firmware projects. It does not attempt to
wrap or replace the Zephyr driver. The four-layer architecture
(`registers.rs` → `types.rs` → `policy.rs` → `controller.rs`) is intentional
and serves the Rust firmware stack.

The Zephyr driver (`spi_monitor_aspeed.c`) and this crate cover the same
hardware from different software stacks. They are not in conflict.

**Remaining action:** Add a brief note to `overview-and-usage-model.md`
stating the deployment context explicitly.

---

### 2. Feature Scope: Command Tables vs. Address Filtering

**Original finding:** The scaffold over-emphasised command tables. Zephyr
primary usage is address-region filtering and mux switching. No evidence was
found that command-table slots are used in production Zephyr firmware.

**Current state:**
- Address filtering is now the primary path through `apply_policy`. The region
  loop was missing from the original scaffold (Gap E) and has been added.
- `PrivilegeOp` (`Enable`/`Disable`) is now part of `RegionPolicy`, matching
  the Zephyr `SPI_FILTER_PRIV_ENABLE`/`DISABLE` semantics.
- Command table support remains in `apply_policy` for hardware completeness
  but is not required for the core address-filtering use case.

**Open question:** Validate with ASPEED hardware team whether command-table
slots in SPIPF are functional in the AST10x0 silicon or are a reserved/future
feature.

---

### 3. Mux Switching (PRIMARY use case in Zephyr)

**Original finding:** Mux switching (`spim_ext_mux_config`) was mentioned only
in passing in the overview but is one of the two most-used SPIM operations in
the Zephyr project (`switch_spim_mux` called throughout
`lib/hrot_hal/gpio/gpio_aspeed.c` and `preload-fw/src/gpio/gpio_ctrl.c`).

**Current state:** `ExtMuxSel` type and `set_ext_mux()` are implemented on
both `SpiMonitor<Configured>` and `SpiMonitor<Locked>`. Mux selection is
deliberately available in the locked state because it is used during
boot-hold/release transitions that happen after policy is locked.

**Platform polarity note:** Zephyr maps `SPIM_EXT_MUX_SEL_0` to ROT or
BMC/PCH depending on `CONFIG_SPI_MUX_INVERSE`. The Rust crate exposes raw
`Sel0`/`Sel1`. Platform code is responsible for the logical mapping.

---

### 4. Passthrough Mode

**Original finding:** Not present in the original scaffold despite being called
in every boot-hold/release function in Zephyr (`spim_passthrough_config`).

**Current state:** `PassthroughMode { Enabled, Disabled }` is implemented and
`set_passthrough()` is available on both `Configured` and `Locked` states.

**Note:** The Zephyr `flags` parameter to `spim_passthrough_config` is always
`0` in all observed call sites. No per-direction passthrough bitmask is needed
unless the silicon register exposes that granularity.

---

### 5. Violation Log

**Original finding:** The Zephyr driver decodes violation log words from SPIPF
log RAM into three categories: blocked command, blocked write address, blocked
read address. The original scaffold had no log types, no log register
accessors, and no drain method.

**Current state:**
- `ViolationLogEntry` enum with `parse(u32)` implements the Zephyr
  `spim_log_parser` decode logic (bits[19:18] context field).
- `read_log_idx_reg()`, `read_log_max_sz()`, `log_ram_base_addr()` added to
  `registers.rs`.
- `drain_log(&mut buf)` available on `SpiMonitor<Configured>` and
  `SpiMonitor<Locked>`.

**ISR/workqueue boundary:** The Zephyr logging path uses `k_work`, `k_sem`,
and a workqueue-deferred drain callback. None of that is in this crate. The
crate provides the synchronous data-plane; the platform layer owns interrupt
registration, workqueue dispatch, and log-pointer reset.

**Placeholder offsets:** Log register offsets (0x080, 0x084, 0x088) are
placeholders pending datasheet confirmation. These are marked with TODO
comments in `registers.rs`.

---

### 6. Integration Model: Boot Sequencing

**Finding:** Zephyr `BMCBootHold()` / `BMCBootRelease()` orchestrate monitor
mux and flash controller together in a single function. The Rust crate does
not attempt to model this orchestration — that is correctly the caller's
responsibility.

**What the crate provides:**
- `set_ext_mux()` for the monitor mux half of the sequence
- `sw_reset()` (gap — not yet implemented) for the monitor reset in release
- `set_passthrough()` for the filter bypass during transitions

**What stays outside the crate:** flash controller reset, GPIO control, boot
CPU assertion, sequencing logic.

---

### 7. Process Isolation Context

The `notes/note1.md` in aspeed-zephyr-project notes a future per-controller
process isolation model where monitor policy updates become RPC calls. The
crate's design is compatible with this:

- `MonitorPolicy` is a pure-data struct serialisable over any transport.
- `policy.rs` has no MMIO dependency and can be used on either side of an IPC
  boundary.
- Typestate enforcement (`Configured` → `Locked`) maps cleanly to a one-shot
  policy-apply RPC with no post-lock mutation surface.

---

## Validation Checklist

### Still Requires Hardware/Zephyr Team Input

- [ ] **Command tables**: Are SPIPF command-table slots functional in AST10x0
  silicon, or reserved/future?
- [ ] **Lock register**: Confirm SPIPF000 bit position for write-lock. Current
  crate uses bit 31 as a placeholder.
- [ ] **Log register offsets**: Confirm offsets for log index, max size, and
  RAM base address registers (currently 0x080/0x084/0x088).
- [ ] **SW reset bit**: Confirm SPIPF000 bit for software reset (needed for
  open gap above).
- [ ] **Passthrough bit**: Confirm SPIPF000 bit for passthrough enable
  (currently bit 1 placeholder).
- [ ] **Ext mux bit**: Confirm SPIPF000 bit for external mux select (currently
  bit 2 placeholder).
- [ ] **Address filter slot encoding**: Confirm hardware word format for
  `spipfwa` entries (direction, op, address granule, length encoding).

### Closed Items (Previously Open)

- [x] Monitor enable/disable missing from scaffold → `enable()`/`disable()` added
- [x] Passthrough mode not modelled → `PassthroughMode` + `set_passthrough()` added
- [x] Ext mux not modelled → `ExtMuxSel` + `set_ext_mux()` added
- [x] `PrivilegeOp` (enable vs disable region) missing → added to `RegionPolicy`
- [x] Region application loop missing from `apply_policy` → loop added
- [x] Violation log types absent → `ViolationLogEntry` + `parse()` added
- [x] Log register accessors absent → added to `registers.rs`
- [x] `drain_log` absent → added to `controller.rs`
- [x] Deployment context unclear → resolved: standalone `no_std` Rust HAL

---

## Summary of Gap Status

| Gap | Description | Status |
|---|---|---|
| A | Monitor enable/disable | ✅ closed |
| B | Passthrough mode | ✅ closed |
| C | External mux selection | ✅ closed |
| D | `PrivilegeOp` on `RegionPolicy` | ✅ closed |
| E | Region loop in `apply_policy` | ✅ closed |
| F | Violation log types + register accessors + `drain_log` | ✅ closed (placeholder register offsets) |
| — | Software reset (`aspeed_spi_monitor_sw_rst`) | ⚠️ open |
| — | Register bit/offset confirmation from datasheet | ⚠️ open (all placeholder values) |

---

## Conclusion

The crate now covers the full Zephyr SPIM driver API surface at the data-plane
level, with the exception of `aspeed_spi_monitor_sw_rst`. The architecture
question is resolved: this is a standalone Rust peripheral HAL, not a Zephyr
wrapper. All placeholder register bit positions and offsets require
confirmation from the AST10x0 datasheet before the crate can be validated on
silicon.
