# 4-Byte Command Capability Plan

Date: 2026-05-03
Owner: SMC device layer
Scope: `target/ast10x0/peripherals/smc/device/flash.rs` and adjacent integration/tests

Status: In progress (implementation largely complete; docs/status closeout pending)

## 1. Objective

Close the 4-byte command parity gap against spitest behavior for SPI controller flows while preserving existing 3-byte behavior for FMC and small-capacity devices.

Target parity points from spitest:
- 4-byte fast read command path (`READ_FAST_4B` semantics).
- 4-byte page program command path (`PP_4B` semantics).
- 4-byte erase command path for large-address devices.
- Address-width policy that is explicit, testable, and not implicit in call sites.

## 2. Current State Snapshot

Implemented now:
- `AddressWidth` includes `FourByte` in `smc/types.rs`.
- Command encoding helper supports 4-byte address serialization (`encode_addr_cmd`).
- Addressing policy and command profile are implemented:
  - `FlashAddressingPolicy`
  - `FlashCommandProfile`
- `erase_sector` and `program_page` dispatch opcode/address width via policy/profile.
- Explicit policy override is available through `with_addressing_policy(...)`.
- Device facade read path uses memory-mapped reads, not explicit `READ_FAST_4B` user command transactions.

Primary remaining gap in this phase:
- Planning/status artifacts need final closeout updates and evidence roll-up.

## 3. Design Decisions

1. Keep command addressing explicit at the device facade level.
2. Avoid hidden mode transitions (for example, enter/exit 4-byte global mode) in this phase.
3. Select 3-byte vs 4-byte command family by policy derived from `FlashConfig` and optional explicit override.
4. Preserve current public behavior by default: existing callers continue to work without source changes.

## 4. Extension Points (Next-Level Structure)

### EP-1: Addressing policy as a type

Add a dedicated policy type to avoid scattered conditional logic.

Proposed type:
- `FlashAddressingPolicy`
  - `ThreeByteOnly`
  - `FourByteCommands`
  - Optional future: `AutoByCapacity` (if explicit runtime policy is needed later)

Why:
- Centralizes command/address-width decisions.
- Makes test matrix compact and auditable.

### EP-2: Command set profile as a type

Add a profile that maps operation -> opcode by addressing mode.

Proposed type:
- `FlashCommandProfile`
  - `read_fast`
  - `page_program`
  - `erase_sector_4k`
  - `read_status`
  - `write_enable`

Proposed construction:
- `FlashCommandProfile::for_addressing(policy)`

Why:
- Cleanly decouples operation logic from opcode constants.
- Enables vendor/family extension without touching erase/program core loops.

### EP-3: Builder-style facade configuration

Add explicit policy setter on `SpiNorFlash`.

Proposed API:
- `with_addressing_policy(self, policy: FlashAddressingPolicy) -> Self`

Default behavior:
- Derive from capacity at constructor time:
  - `capacity_mb <= 16` -> 3-byte.
  - `capacity_mb > 16` -> 4-byte command family.

Why:
- Keeps existing call sites simple.
- Allows deterministic override in tests and bring-up.

## 5. Planned Change Map (Files, Types, Refactor Points)

## A. Core device logic

1. File: `target/ast10x0/peripherals/smc/device/flash.rs`
- Add types:
  - `FlashAddressingPolicy`
  - `FlashCommandProfile`
- Add state fields to `SpiNorFlash`:
  - `addressing_policy`
  - `command_profile`
- Refactor methods:
  - `erase_sector` uses policy-selected width/opcode.
  - `program_page` uses policy-selected width/opcode.
  - `issue_command` stays transport-only.
- Add helper methods:
  - `fn default_addressing_for_cfg(cfg: FlashConfig) -> FlashAddressingPolicy`
  - `fn addr_width(&self) -> AddressWidth`
  - `fn command_profile(&self) -> &FlashCommandProfile`

Refactor boundary:
- No changes to `SpiNorFlashDevice` trait signatures in this phase.

2. File: `target/ast10x0/peripherals/smc/device/mod.rs`
- Re-export newly introduced public types if exposed by API.

3. File: `target/ast10x0/peripherals/smc/mod.rs`
- Re-export policy/profile types if public surface requires top-level visibility.

## B. Shared types and error model

4. File: `target/ast10x0/peripherals/smc/types.rs`
- Keep `AddressWidth` as canonical low-level width type.
- Optional: add small helper mapping policy -> `AddressWidth` only if needed by more than one module.

Refactor boundary:
- Avoid adding 4-byte-specific errors unless a concrete hardware distinction is needed.

## C. Host/unit test coverage

5. File: `target/ast10x0/peripherals/smc/device/flash.rs` (unit tests module)
- Add tests for policy/opcode mapping:
  - 3-byte policy selects legacy opcodes and 3-byte width.
  - 4-byte policy selects 4-byte opcodes and 4-byte width.
- Add tests for default policy derivation from capacity.
- Add tests that builder override supersedes default policy.

6. File: `target/ast10x0/peripherals/smc/host_flash_mod.rs`
- Update stubs if new public methods/fields affect host test compilation.

7. File: `target/ast10x0/peripherals/BUILD.bazel`
- Keep existing host tests; add sources only if new helper files are created.

## D. QEMU integration coverage

8. File: `target/ast10x0/tests/smc/target_device_qemu_program_erase.rs`
- Add scenario that instantiates flash with 4-byte-capable config and validates program/erase/readback loop.
- Ensure assertions distinguish policy selection from generic write success.

9. File: `target/ast10x0/tests/smc/BUILD.bazel`
- Extend existing test target or add dedicated 4-byte integration target if split is cleaner.

## E. Planning/documentation traceability

10. File: `target/ast10x0/peripherals/smc/planning/device/spitest-behavior-support-report.md`
- Update parity status after implementation and tests.

11. File: `target/ast10x0/peripherals/smc/planning/checkpoint.md`
- Mark completion status for the May 3 4-byte workstream.

## 6. Refactoring Strategy (Safe Sequence)

1. Refactor-1 (no behavior change): introduce policy/profile types with current behavior wired to 3-byte defaults.
2. Refactor-2 (behavioral): switch erase/program opcode+width selection to policy-driven logic.
3. Refactor-3 (API extension): add builder override for addressing policy.
4. Refactor-4 (cleanup): remove duplicated opcode/width constants from method bodies.
5. Refactor-5 (optional): move command profile definitions into a small `commands` submodule if file growth exceeds readability threshold.

## 7. Work Packages and Commit Slices

### WP-4B-001: Introduce policy/profile scaffolding
- Files: `smc/device/flash.rs`, optional re-export files.
- Expected tests:
  - `//target/ast10x0/peripherals:smc_flash_encoding_test`
- Exit:
  - Zero behavior change, tests pass.
- Status: Completed.

### WP-4B-002: Policy-driven program/erase selection
- Files: `smc/device/flash.rs`.
- Expected tests:
  - host unit tests for opcode/width mapping.
  - `//target/ast10x0/peripherals:smc_flash_encoding_test`
- Exit:
  - 3-byte behavior unchanged, 4-byte policy path covered by unit tests.
- Status: Completed.

### WP-4B-003: Integration validation in QEMU target
- Files: `tests/smc/target_device_qemu_program_erase.rs`, `tests/smc/BUILD.bazel`.
- Expected tests:
  - `//target/ast10x0/tests/smc:smc_device_qemu_program_erase_test`
- Exit:
  - End-to-end pass with explicit 4-byte policy/config path.
- Status: Completed.

### WP-4B-004: Docs parity update
- Files: planning docs listed above.
- Exit:
  - parity report and checkpoint reflect measured status.
- Status: Pending closeout.

## 8. Acceptance Criteria

1. Device facade supports explicit 4-byte command selection for erase/program flows.
2. Existing 3-byte tests remain green.
3. At least one host test proves opcode/address-width dispatch for both policy branches.
4. At least one integration test exercises the 4-byte path intent and passes.
5. Planning artifacts updated with evidence links and residual risk notes.

## 9. Risks and Mitigations

1. Risk: QEMU model limitations may not fully distinguish 3-byte vs 4-byte wire behavior.
- Mitigation: assert command construction/mapping in host tests; treat QEMU test as integration smoke, not wire-encoding proof.

2. Risk: introducing too many public types may increase API churn.
- Mitigation: keep public surface stable now that `FlashAddressingPolicy` and `FlashCommandProfile` are exported; defer further public-type expansion without a second concrete consumer.

3. Risk: accidental behavior regressions in existing program/erase loops.
- Mitigation: preserve method contracts; run existing multi-CS and program/erase suites before merge.

## 10. Validation Command Set

Build baseline:
- `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:smc`

Host/device-layer tests:
- `bazelisk test //target/ast10x0/peripherals:smc_flash_encoding_test`

QEMU integration tests:
- `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_program_erase_test`
- Optional regression sweep:
  - `//target/ast10x0/tests/smc:smc_device_qemu_multi_cs_test`
  - `//target/ast10x0/tests/smc:smc_device_qemu_multi_cs_offsets_test`
  - `//target/ast10x0/tests/smc:smc_device_qemu_multi_cs_capacity_test`

## 11. Completion Snapshot (2026-05-03)

Completed:
- Policy/profile scaffolding landed.
- Program/erase opcode + address-width dispatch is policy-driven.
- QEMU integration includes explicit 4-byte policy extension-point assertions.
- Host/unit + target build gates pass for this phase.

Remaining for full document closeout:
- Update `spitest-behavior-support-report.md` with final parity wording for 4-byte path status.
- Update `checkpoint.md` done/not-done checklist for May 3.
- Optionally add commit hashes and command output links to this file for audit traceability.
