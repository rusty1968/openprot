# Device Layer Auditable TDD Implementation Plan

Date: 2026-05-02
Scope: `target/ast10x0/peripherals/smc/device/flash.rs`

## 1. Objective

Close remaining device-layer parity gaps against aspeed-rust behavior using a
strict, auditable TDD workflow where each requirement is linked to tests,
implementation changes, and verifiable evidence.

## 2. TDD Governance Rules

1. RED first: every requirement starts with a failing test.
2. GREEN minimal: implement only what is needed to pass that test.
3. REFACTOR safe: improve structure only with full test pass.
4. One requirement per commit where practical.
5. No commit unless required build/test gates pass.

## 3. Requirement Set (Auditable IDs)

### DEV-PAR-001: Per-CS device capacity correctness
Requirement:
- `SpiNorFlash::{from_fmc_cs,from_spi_cs}` must validate `FlashConfig`
  against selected-CS capacity, not controller total capacity.

Acceptance Criteria:
- CS0 and CS1 constructors accept matching per-CS config.
- Mismatched per-CS capacity returns `SmcError::InvalidCapacity`.

### DEV-PAR-002: CS-relative read offset semantics
Requirement:
- `FlashDevice::read(offset, ...)` treats `offset` as device-local and
  applies selected CS base translation.

Acceptance Criteria:
- CS-local reads do not alias across CS windows.
- Bounds checks are enforced against selected-CS capacity.

### DEV-PAR-003: CS-relative command addressing
Requirement:
- Program/erase command address encoding uses device-local offsets with
  selected CS base semantics consistent with read path.

Acceptance Criteria:
- `program_page` and `erase_sector` target correct CS-local address space.
- No cross-CS address bleed.

### DEV-PAR-004: Verify large-buffer behavior
Requirement:
- `verify()` supports buffers larger than 256 bytes via chunked comparison.

Acceptance Criteria:
- Verify succeeds for multi-page inputs.
- Verify remains bounded and deterministic.

### DEV-PAR-005: Transfer mode policy explicitness
Requirement:
- Device layer transfer mode policy is explicit: either configurable or
  intentionally fixed to `Mode111` with documented rationale.

Acceptance Criteria:
- Public API/docs make policy explicit.
- Tests cover chosen behavior.

### DEV-PAR-006: Polling/timeout policy fidelity
Requirement:
- `wait_write_complete()` timeout/poll policy is documented and tested.

Acceptance Criteria:
- Timeout path deterministic and test-covered.
- Success path deterministic and test-covered.

## 4. Traceability Matrix

| Req ID | RED Test Target | Implementation File(s) | Validation Command(s) | Evidence Artifact |
|---|---|---|---|---|
| DEV-PAR-001 | `target_device_qemu_multi_cs.rs` (new constructor checks) | `smc/device/flash.rs` | `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_multi_cs_test` | test output + commit hash |
| DEV-PAR-002 | `target_device_qemu_multi_cs.rs` (CS-local read behavior) | `smc/device/flash.rs` | same as above | test output + commit hash |
| DEV-PAR-003 | `target_device_qemu_program_erase.rs` (CS-local address ops) | `smc/device/flash.rs` | `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_program_erase_test` | test output + commit hash |
| DEV-PAR-004 | host unit test in `smc/device/flash.rs` | `smc/device/flash.rs`, `peripherals/BUILD.bazel` | `bazelisk test //target/ast10x0/peripherals:smc_flash_encoding_test` (+ new verify test target if added) | test output + commit hash |
| DEV-PAR-005 | unit/integration test per selected policy | `smc/device/flash.rs`, docs | depends on selected policy | test output + commit hash |
| DEV-PAR-006 | timeout/success tests in device integration path | `smc/device/flash.rs` + test files | relevant QEMU test target(s) | test output + commit hash |

## 5. Work Packages (Execution Order)

### WP-1 (DEV-PAR-001)
RED:
- Add failing constructor tests for per-CS capacity mismatch.
GREEN:
- Add selected-CS capacity resolution in constructor validation.
REFACTOR:
- Centralize CS capacity lookup helper.

### WP-2 (DEV-PAR-002 + DEV-PAR-003)
RED:
- Add failing CS-local read and command-address tests.
GREEN:
- Add CS base translation model and use it in read/program/erase.
REFACTOR:
- Isolate `device_to_controller_offset()` helper.

### WP-3 (DEV-PAR-004)
RED:
- Add failing verify test >256 bytes.
GREEN:
- Implement chunked verify.
REFACTOR:
- Reuse chunk helper for future read/compare paths.

### WP-4 (DEV-PAR-005)
RED:
- Add failing tests for selected transfer mode policy.
GREEN:
- Implement policy (configurable mode or explicit fixed mode).
REFACTOR:
- Minimize mode plumbing complexity.

### WP-5 (DEV-PAR-006)
RED:
- Add deterministic timeout/success tests around WIP polling.
GREEN:
- Align timeout semantics and status interpretation.
REFACTOR:
- Extract poll-loop policy constants.

## 6. Mandatory Pre-Commit Gates

For each WP commit:
1. `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:smc`
2. Run relevant RED/GREEN target from the traceability row.
3. If runtime path touched, run corresponding QEMU test target.
4. Do not commit if any required target fails.

## 7. Audit Evidence Log Template

Use this section as an append-only log.

- Date:
- Req ID(s):
- RED test added:
- Commit hash:
- Build command + result:
- Test command + result:
- Notes/risk:

---

- Date: 2026-05-02
- Req ID(s): DEV-PAR-001
- RED test added: `target/ast10x0/tests/smc/target_device_qemu_multi_cs_capacity.rs`
  (target `//target/ast10x0/tests/smc:smc_device_qemu_multi_cs_capacity_test`,
  tag `integration`).
- Commit hashes:
  - RED: `f6025a9` — `test(smc/device): add RED test for DEV-PAR-001 per-CS capacity`
  - GREEN: `f38f5e8` — `feat(smc/device): per-CS capacity validation (DEV-PAR-001)`
  - REFACTOR: skipped — `cs_config` has only two call sites
    (`SpiNorFlash::from_fmc_cs`, `SpiNorFlash::from_spi_cs`); the plan's
    "more than two callers" trigger for hoisting `cs_capacity_bytes` was
    not met.
- Build command + result:
  - `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:smc`
    → `Build completed successfully`.
- Test command + result:
  - `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_multi_cs_capacity_test //target/ast10x0/tests/smc:smc_device_qemu_multi_cs_test //target/ast10x0/tests/smc:smc_device_qemu_program_erase_test //target/ast10x0/tests/smc:smc_device_test`
    → all 4 PASS.
  - Pre-GREEN run of the RED target alone: `FAILED in 0.5s` (asserted
    failure on assertions 1 & 3 of the new test, where the constructor
    rejects a correctly-sized per-CS config because it compares
    against CS0+CS1 total).
  - Post-GREEN sanity: `bazelisk test //target/ast10x0/peripherals:smc_flash_encoding_test`
    → PASS (host stub `host_flash_mod.rs` updated to expose `cs_config`).
- Notes/risk:
  - Deviation from plan §3.1.5: `cs_capacity_bytes` helper in
    `helpers.rs` was *not* added in WP-1 because no GREEN call site
    needs it — `validate_capacity_cfg` uses the controller's
    `cs_config(cs)` accessor and compares full `FlashConfig` equality
    via the new `PartialEq`/`Eq` derive. WP-2 (DEV-PAR-002/003) will
    add `cs_capacity_bytes` when its `device_to_controller_offset`
    helper introduces a real caller.
  - `validate_capacity_cfg` now compares whole-`FlashConfig` rather
    than `capacity_mb` alone (plan suggested either is fine). Stricter:
    catches drift between init-time `SmcConfig.cs*` and facade-time
    `FlashConfig` in page/sector/clock fields too.
  - Status: PASS.

## 8. Exit Criteria

Plan is complete when:
1. DEV-PAR-001..006 are all marked PASS.
2. Each requirement has linked tests and passing evidence.
3. No device-layer parity gaps remain open in `parity-gap-status.md`.
