# SMC Transport TDD Plan (aspeed-rust Parity)

Date: 2026-05-02

## Goal

Preserve functional parity with aspeed-rust for SMC transport behavior by
making each parity claim executable via tests before implementation changes.

## TDD Rule

For every behavior slice:
1. Write a failing test (RED).
2. Implement the minimum code change (GREEN).
3. Refactor while keeping tests green (REFACTOR).
4. Commit test + code together.

No transport behavior change is complete without a test asserting it.

## Parity Contract (Spec)

The following externally-visible behavior is the parity spec:
1. User-mode lifecycle: CS assert, per-phase ctrl updates, CS deassert,
   normal-read restore.
2. Per-phase IO mode mapping: cmd/addr/data IO bits from `TransferMode`.
3. Address encoding: `AddressWidth::{None,ThreeByte,FourByte}`.
4. Error semantics and precedence:
   - `ControllerNotReady` for non-ready state
   - `InvalidChipSelect` for CS1 when `cs1` not configured
   - precedence rules are explicit and tested.
5. Device command flow: WREN, RDSR polling, program, erase, verify.
6. Multi-CS routing: CS0/CS1 select correct ctrl register + AHB window.

## Existing Coverage (Baseline)

Already in tree:
- `target/ast10x0/tests/smc/target_device_qemu_program_erase.rs`
- `target/ast10x0/tests/smc/target_error_granularity.rs`
- `target/ast10x0/tests/smc/target_device_qemu_multi_cs.rs`

These provide baseline confidence for program/erase flow, error granularity,
and CS routing.

## RED Test Backlog (Ordered)

### 1) Error precedence test

Status:
- COMPLETED in commit `01b47ca`.
- Validated by `//target/ast10x0/tests/smc:smc_error_granularity_test` on QEMU.

Intent:
- Prove state guard is checked before chip-select validation and arg checks.

RED test:
- Force controller non-ready (DMA in-flight), then call
  `transceive_user(ChipSelect::Cs1, ...)` on a CS1-unconfigured instance.
- Expected: `Err(SmcError::ControllerNotReady)`.

File target:
- `target/ast10x0/tests/smc/target_error_granularity.rs`

### 2) TransferMode differential test

Status:
- COMPLETED in the multi-CS test update series.
- Validated by `//target/ast10x0/tests/smc:smc_device_qemu_multi_cs_test` on QEMU.

Intent:
- Keep cmd/addr/data IO-bit mapping stable for all supported modes.

RED test:
- For each mode in `{111,112,122,114,144}`, run a small raw transaction
  and assert transport completes and returns deterministic status path.

File target:
- `target/ast10x0/tests/smc/target_device_qemu_multi_cs.rs`

### 3) AddressWidth byte-stream test

Status:
- COMPLETED in commit `7f03da6`.
- Validated by `//target/ast10x0/peripherals:smc_flash_encoding_test` (host test).

Intent:
- Ensure opcode+address assembly never regresses.

RED test:
- Unit tests on command builder for None/3-byte/4-byte with known offsets.
- Assert exact emitted byte slices.

File target:
- `target/ast10x0/peripherals/smc/device/flash.rs` (unit test module)

### 4) Normal-read restore invariant test

Status:
- COMPLETED in commit `1516a49`.
- Validated by `//target/ast10x0/tests/smc:smc_device_qemu_program_erase_test` on QEMU.

Intent:
- Ensure user-mode transaction restore path remains intact.

RED test:
- Run user transaction then mapped read; assert read path behavior remains
  valid (no stuck user-mode behavior).

File target:
- `target/ast10x0/tests/smc/target_device_qemu_program_erase.rs`

## Differential Parity Harness (Phase 2)

Add a small host-side parity runner that executes equivalent operation scripts
against reference HAL behavior and captured aspeed-rust expectations.

Compare:
1. Operation outcome (ok/error)
2. Status-byte outcomes at checkpoints
3. Programmed/erased data checks
4. Error variant identity

This catches semantic drift even when both sides still "work".

## CI Gate

A change touching transport is mergeable only if all pass:
1. `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/peripherals:peripherals`
2. `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:smc`
3. `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:target_device_qemu_program_erase`
4. `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:target_error_granularity`
5. `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:target_device_qemu_multi_cs`

Optional execution gate (when QEMU runtime is enabled):
- `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_program_erase_test`
- `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_error_granularity_test`
- `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_multi_cs_test`

## Commit Discipline

Each commit should follow:
1. Add/adjust failing test.
2. Add minimum production change to pass.
3. Keep commit narrow (one behavior slice).
4. Include test name(s) in commit message body.

Mandatory pre-commit gate:
1. Do not commit unless the relevant targets for the touched behavior pass.
2. For transport-path changes, this means at least:
  - `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/tests/smc:smc`
  - matching QEMU `bazelisk test` target(s) for the modified path when available.
3. If no runtime test exists yet for the touched behavior, add one first,
  then require it to pass before committing.

Example message:
- `ast10x0/smc: enforce not-ready precedence over cs validation`
- body: `Covers target_error_granularity.rs precedence case.`

## Exit Criteria

Transport parity is considered stable when:
1. All parity-contract behaviors have dedicated tests.
2. Every transport bug fix includes a regression test.
3. CI gate is green for transport-related changes.
4. No untested behavior changes in `controller.rs`, `fmc.rs`, `spi.rs`,
   `device/flash.rs`, or transport-facing tests.
