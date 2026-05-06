# SCU Access Validation Report (AST10x0)

Date: 2026-05-06
Scope: `target/ast10x0` code paths validated against `target/ast10x0/peripherals/scu` and `target/ast10x0/peripherals/scu/datasheet/registermap.md`.

## Validation Method

1. Enumerated all SCU MMIO touches in `target/ast10x0/peripherals/scu/*.rs`.
2. Enumerated all external SCU callsites (`ScuRegisters::new_global` and SCU helper methods).
3. Checked whether any code bypasses SCU module boundaries with raw PAC SCU register access.
4. Compared accessed register offsets against the local SCU register map document.

## Access Inventory

### SCU Registers Accessed by the SCU Module

- `SCU004` (`status.rs`): silicon revision read.
- `SCU040` / `SCU044` / `SCU050` / `SCU054` (`reset.rs`): reset assert/deassert + readback.
- `SCU080` / `SCU084` / `SCU090` / `SCU094` (`clock.rs`): clock gate/ungate + readback.
- `SCU0F0` (`routing.rs`, `status.rs`): SPI monitor passthrough, internal route, ext mux, route raw read.
- `SCU690` / `SCU694` (`routing.rs`, `status.rs`): SPI monitor MISO multi-function pin controls + raw reads.

### External Callers Using SCU API

- Board orchestration:
	- `target/ast10x0/board/src/spim_wiring.rs`
	- `target/ast10x0/board/src/monitor.rs`
	- `target/ast10x0/board/src/lib.rs`
- SPI monitor facade:
	- `target/ast10x0/peripherals/spimonitor/controller.rs`
- Flash backend + tests:
	- `target/ast10x0/backend/flash/src/lib.rs`
	- `target/ast10x0/tests/flash/target.rs`
	- `target/ast10x0/tests/flash/target_dual_cs.rs`

### Encapsulation Result

- No direct/raw SCU register access found outside `target/ast10x0/peripherals/scu`.
- External code uses typed SCU helpers (good boundary hygiene).

## Discrepancies

### 1) Missing SCU protection-key unlock sequence before SCU writes

Severity: High
Status: Resolved (2026-05-06)

Originally observed:
- `registermap.md` documents `SCU000` protection key (`0x1688A8A8`) as unlock required for SCU registers.
- No SCU helper in `peripherals/scu` wrote `SCU000` or `SCU010`.
- SCU write operations were performed in `reset.rs`, `clock.rs`, and `routing.rs` without an explicit unlock step.

Impact:
- On hardware/boot states where SCU is locked, writes may be ignored or partially applied.
- Boot-time routing/reset/clock behavior can become non-deterministic across environments.

Applied fix:
- Added `ScuRegisters::unlock_write_protection()` in `peripherals/scu/registers.rs`.
- Added unlock calls at entry of all mutating helpers in:
	- `peripherals/scu/reset.rs`
	- `peripherals/scu/clock.rs`
	- `peripherals/scu/routing.rs`

### 2) Register map documentation omission for `SCU094`

Severity: Medium
Status: Resolved (2026-05-06)

Originally observed:
- Code uses `SCU094` in `clock.rs` for upper-half clock-stop clear.
- `datasheet/registermap.md` included `0x090` but omitted `0x094`.

Impact:
- Documentation and implementation are out of sync.
- Future reviewers may incorrectly classify `SCU094` usage as invalid.

Applied fix:
- Updated `datasheet/registermap.md` to include:
	- `0x094 | 32b | Clock Stop Control Clear Set 2 | W1C | Clear bits in SCU090`

## Non-Discrepancy Observations

- SCU access remains centralized behind `ScuRegisters` and typed helpers.
- SPI monitor routing and mux usage is consistently mediated through `scu::routing` APIs.

## Comparison Against aspeed-rust Access Patterns

Reference codebase: `/home/rusty1968/work/storage/aspeed-rust`

### Pattern A: SCU unlock key usage

Observed in aspeed-rust:
- `src/i2c_core/global.rs` explicitly writes unlock key before SCU mutations:
	- comment: "Write magic value 0x1688_A8A8 to SCU000 to unlock"
	- code: `scu.scu000().write(|w| w.bits(0x1688_A8A8));`

Comparison result:
- This supports the report's discrepancy #1.
- In aspeed-rust, at least some init flows treat unlock as mandatory precondition.
- Current `reference/target/ast10x0/peripherals/scu` code has no analogous unlock helper.

Confidence update:
- Discrepancy #1 remains High confidence.

### Pattern B: Upper clock-clear register (`SCU094`) usage

Observed in aspeed-rust:
- `src/syscon.rs` uses `scu094()` for upper-group clock enable path.
- `src/i3c/hardware.rs` also references `scu094()`.

Comparison result:
- This supports the report's discrepancy #2.
- `SCU094` is not only valid, it is actively used in multiple aspeed-rust subsystems.

Confidence update:
- Discrepancy #2 remains Medium confidence (documentation mismatch, not code bug).

### Pattern C: SPI monitor routing control via `SCU0F0`

Observed in aspeed-rust:
- `src/spimonitor/hardware.rs` and `src/spi/spim.rs` use `SCU0F0` for:
	- internal SPI master detour select
	- external mux select per SPIPF instance
	- clear route bits (`& !0xF` style)

Comparison result:
- Current `reference` SCU routing helpers match these semantics closely:
	- `set_spim_internal_master_route`
	- `set_spim_ext_mux`
	- `clear_spim_internal_master_route`

Confidence update:
- No discrepancy here; implementation pattern is aligned.

### Pattern D: SCU pin-mux control via `SCU690/SCU694`

Observed in aspeed-rust:
- `src/spimonitor/hardware.rs`, `src/spi/spim.rs`, and `src/pinctrl.rs` modify/read `SCU690` and `SCU694` for SPIM pin function control.

Comparison result:
- Current `reference` usage of `set_spim_miso_multi_func` and status reads aligns with this family of accesses.

Confidence update:
- No discrepancy here; pattern is aligned.

### Pattern E: Encapsulation style

Observed in aspeed-rust:
- Many modules take direct PAC pointers (`Scu::ptr()`) and perform SCU RMW locally.

Comparison result:
- `reference` project is intentionally stricter: SCU access is centralized in `peripherals/scu` and consumed through typed helpers.
- This is an architectural divergence, not a correctness issue.

Confidence update:
- No discrepancy; this is a positive hardening difference.

## Conclusion

Initial validation found 2 discrepancies:
- 1 high-severity runtime access-precondition issue (missing explicit unlock handling).
- 1 medium-severity documentation mismatch (`SCU094` omission).

Current status after remediation:
- 2/2 discrepancies resolved in-tree.

All other observed SCU accesses are structurally consistent with the intended SCU module boundary.
