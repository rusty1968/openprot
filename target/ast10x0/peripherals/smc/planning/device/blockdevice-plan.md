# Block Device Facade Plan (Contained / Reuse-Only)

Date: 2026-05-03
Owner: SMC device layer
Scope: Add a minimal block-device facade on top of existing SPI NOR mechanisms without architectural reshaping.

## 1. Goal

Provide a small, JEDEC-driven block-device facade equivalent in intent to `NorFlashBlockDevice::from_jedec_id(...)` so higher layers can consume flash as a geometry-aware block API without dealing with command-level details.

## 2. Constraints (Must-Haves)

1. Reuse existing mechanisms only.
- Reuse `SpiNorFlash`, `SpiNorFlashDevice`, `FlashConfig`, `JedecId`, and existing read/program/erase helpers.
- Reuse existing backend error mapping patterns where needed.

2. Keep the facade contained.
- No controller architecture changes.
- No transport-layer redesign.
- No state-machine changes in SMC controller wrappers.

3. Avoid broad API churn.
- Additive API only.
- Preserve current public behavior and call sites.

## 3. Non-Goals

1. No new transport path (PIO/DMA policy remains unchanged).
2. No SPI monitor/SCU composition work in this slice.
3. No global refactor of `backend/flash`.
4. No attempt to close all spitest parity deltas in this one change.

## 4. Reuse Inventory (What We Already Have)

1. Device operations:
- `SpiNorFlash::{read, program, erase_range, update_region, jedec, jedec_id}`

2. Geometry/config carrier:
- `FlashConfig` (`capacity_mb`, `page_size`, `sector_size`, `block_size`)

3. JEDEC representation:
- `JedecId`

4. Existing adapter behavior:
- `target/ast10x0/backend/flash/src/lib.rs` already wraps `SpiNorFlash` into `flash_api::backend::FlashBackend`

## 5. Proposed Facade Shape (Contained)

Create a small facade in `smc/device` that wraps a mutable `SpiNorFlash` reference and exposes block-oriented operations.

Proposed type:
- `SpiNorBlockDevice<'a, 'b>`
  - holds `&'a mut SpiNorFlash<'b>`
  - holds resolved geometry (`FlashConfig` or derived info)

Proposed constructors:
1. `from_flash(flash: &'a mut SpiNorFlash<'b>, cfg: FlashConfig) -> Result<Self, SmcError>`
2. `from_jedec_id(flash: &'a mut SpiNorFlash<'b>, jedec: JedecId) -> Result<Self, SmcError>`
- `from_jedec_id` maps known JEDEC IDs to `FlashConfig` using a local table (contained mapping).

Proposed methods (minimal):
1. `read_blocks(address: u32, out: &mut [u8]) -> Result<usize, SmcError>`
2. `write_blocks(address: u32, data: &[u8]) -> Result<usize, SmcError>`
3. `erase_blocks(address: u32, length: u32) -> Result<(), SmcError>`
4. `info() -> BlockInfo` (or equivalent small struct)

Alignment/policy behavior:
- enforce page alignment for writes using existing `program` contract
- enforce sector alignment for erase using existing `erase_range` contract
- delegate all actual I/O to existing `SpiNorFlash`

## 6. File-Level Change Plan

1. Add: `target/ast10x0/peripherals/smc/device/block_device.rs`
- new contained facade type and JEDEC->config mapping
- no direct register/transport interactions

2. Update: `target/ast10x0/peripherals/smc/device/mod.rs`
- export new block facade type(s)

3. Update: `target/ast10x0/peripherals/smc/mod.rs`
- re-export new block facade type(s) if needed by existing import patterns

4. Optional test touch:
- add host-side unit tests under existing host test arrangement only if required
- keep test additions local to peripheral/device layer first

5. Optional integration touch:
- add a focused QEMU test for `from_jedec_id` construction + smoke read/write/erase behavior

## 7. JEDEC Mapping Strategy (Contained)

1. Start with explicit allowlist for currently used devices (for example Winbond profile[s]).
2. Return `SmcError::DeviceNotSupported` for unknown JEDEC IDs.
3. Keep mapping table local to `block_device.rs` to avoid cross-module coupling.

## 8. Validation Plan

Host/build gates:
1. `bazelisk test //target/ast10x0/peripherals:smc_flash_encoding_test`
2. `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:smc`

If integration test added:
3. `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_program_erase_test`
4. plus dedicated new block-device target if created

## 9. Acceptance Criteria

1. New facade is additive and contained to `smc/device`.
2. Facade reuses `SpiNorFlash` operations; no new transport logic.
3. JEDEC-driven construction exists with clear unsupported-device behavior.
4. Existing tests remain green.
5. At least one new test validates constructor + basic block-style operation path.

## 10. Risks and Mitigations

1. Risk: facade duplicates existing backend adapter semantics.
- Mitigation: keep API minimal and delegate deeply; avoid parallel policy logic.

2. Risk: JEDEC mapping drift.
- Mitigation: explicit local mapping table + test per known ID.

3. Risk: accidental API surface bloat.
- Mitigation: start with minimal methods only; defer expansion until consumers require it.

## 11. Execution Order

1. Add contained type + `from_flash` constructor.
2. Add `from_jedec_id` with minimal known-ID mapping.
3. Add minimal read/write/erase forwarding methods with alignment checks.
4. Add unit test(s) for constructor and unsupported JEDEC path.
5. Add optional integration smoke test.
6. Update parity report/checkpoint after validation.
