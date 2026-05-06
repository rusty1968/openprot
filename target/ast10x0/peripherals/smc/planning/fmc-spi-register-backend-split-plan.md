# FMC/SPI Register Backend Split Plan (TDD)

## Goal

Split SMC register access so FMC and SPI1/SPI2 use their own register layouts,
remove accidental FMC assumptions from shared code, and keep behavior stable for
existing tests.

## Baseline Facts (Current State)

1. Shared register wrapper is FMC-typed today in
   `target/ast10x0/peripherals/smc/registers.rs`.
2. Generic controller is shared across FMC and SPI in
   `target/ast10x0/peripherals/smc/controller.rs`.
3. FMC and SPI facade types both delegate to the same generic controller in
   `target/ast10x0/peripherals/smc/fmc.rs` and
   `target/ast10x0/peripherals/smc/spi.rs`.
4. SMC tests are under `target/ast10x0/tests/smc`.

## Constraints

1. Preserve current public behavior for common operations
   (read, DMA path, CS routing, segment setup).
2. Keep API churn low for callers that use `FmcReady` / `SpiReady` wrappers.
3. Add compile-time structure that prevents layout mixing in future changes.
4. Drive all changes with TDD: write failing tests first, then implement.

## Incremental Implementation Plan

### Phase 0: Baseline lock (tests only)

1. Add tests capturing currently used shared-register behavior.
2. Add one negative test documenting that offset `0x6c` semantics are
   controller-specific and must not be treated as shared.
3. Run existing SMC tests and record pass/fail baseline.

Exit criteria:

1. Baseline tests pass on current code.
2. Existing SMC test suite remains green.

### Phase 1: Introduce shared register contract (no behavior change)

1. Add a small trait for operations used by shared controller flows:
   - config read/write/modify
   - CS control read/write
   - segment read/write
   - DMA status/control/address/length/checksum
   - calibration status
2. Implement the trait for the current register wrapper.
3. Convert generic controller to depend on that trait instead of concrete
   FMC register type.
4. Keep constructor wiring unchanged so runtime behavior is stable.

TDD loop:

1. Red: add fake-backend unit tests for controller behavior.
2. Green: trait + implementation + controller wiring.
3. Refactor: remove duplicated assumptions from controller internals.

Exit criteria:

1. Controller compiles and runs against trait-based backend.
2. No behavior changes in integration tests.

### Phase 2: Split concrete register backends

1. Create FMC-specific backend around `ast1060_pac::fmc::RegisterBlock`.
2. Create SPI-specific backend around `ast1060_pac::spi::RegisterBlock`.
3. Keep only truly shared operations in common trait.
4. Move controller-specific-only register access out of shared path.

TDD loop:

1. Red: backend mapping tests for FMC and SPI.
2. Red: tests proving unsupported cross-layout operations are not exposed.
3. Green: implement both backends and trait impls.
4. Refactor: trim shared trait to minimum common surface.

Exit criteria:

1. Backend split compiles cleanly.
2. Shared controller still passes all existing SMC tests.

### Phase 3: Wire wrappers to correct backend

1. Update FMC construction path to instantiate FMC backend.
2. Update SPI construction path to instantiate SPI backend.
3. Preserve external wrapper method signatures where practical.
4. Reduce runtime branching where type system can enforce correctness.

TDD loop:

1. Red: compile-time pairing tests (`Fmc` with FMC backend, `Spi` with SPI backend).
2. Green: wrapper constructor updates.
3. Refactor: simplify controller ID checks where redundant.

Exit criteria:

1. FMC and SPI wrappers each bind to correct backend type.
2. Caller-facing API impact is minimal and documented.

### Phase 4: Backend-specific feature surfacing

1. Make SPI-only registers/methods available only on SPI backend.
2. Make FMC-only registers/methods available only on FMC backend.
3. Keep shared controller limited to common operations.

TDD loop:

1. Red: tests asserting unsupported operations fail or are uncallable.
2. Green: backend-specific API partitioning.
3. Refactor: clean method naming and docs.

Exit criteria:

1. No ambiguous shared method names for non-shared semantics.
2. Accidental misuse blocked by type system.

### Phase 5: Regression and cleanup

1. Run all SMC-related test targets in:
   - `target/ast10x0/tests/smc`
   - `target/ast10x0/tests/smc_listener`
2. Run multi-CS, capacity, offsets, SPI-focused tests.
3. Remove temporary shims if no longer needed.
4. Update docs and module exports.

Exit criteria:

1. Full SMC suite passes.
2. No lingering compatibility glue unless explicitly retained.

## TDD Test Matrix

### Unit tests

1. Shared trait contract tests with fake backend.
2. FMC backend register mapping tests.
3. SPI backend register mapping tests.
4. Compile-time separation tests for backend/controller pairing.

### Integration tests

1. FMC DMA read path.
2. SPI DMA read path.
3. Multi-CS capacity and offset behavior.
4. Error granularity and IRQ tests.
5. SPI-focused transfer behavior tests.

### Regression criteria

1. Existing SMC tests remain green.
2. Shared operation behavior remains unchanged.
3. New tests prove backend separation and block layout drift.

## Expected Tree Changes (End State)

### Files likely modified

1. `target/ast10x0/peripherals/smc/controller.rs`
2. `target/ast10x0/peripherals/smc/fmc.rs`
3. `target/ast10x0/peripherals/smc/spi.rs`
4. `target/ast10x0/peripherals/smc/mod.rs`
5. `target/ast10x0/peripherals/smc/registers.rs` (reduced, repurposed, or removed)
6. `target/ast10x0/tests/smc/BUILD.bazel`
7. `target/ast10x0/tests/smc/README.md`

### Files likely added

1. Shared register trait module under `target/ast10x0/peripherals/smc/`
2. FMC register backend module under `target/ast10x0/peripherals/smc/`
3. SPI register backend module under `target/ast10x0/peripherals/smc/`
4. Unit test modules for backend contract/mapping
5. Optional compile-fail test harness (if adopted)

## Impact Assessment

### Public API impact

1. Minimal if wrapper signatures stay stable.
2. Internal controller type parameters become explicit and safer.

### Behavioral impact

1. No intentional behavior change in shared operations.
2. Reduced risk of subtle bugs from offset-semantic mismatch.

### Test/build impact

1. New unit test targets for backend separation.
2. Existing SMC integration targets should continue to run.

### Maintenance impact

1. Easier to add SPI-only or FMC-only features safely.
2. Stronger type boundaries prevent accidental cross-layout register usage.

## PR Strategy

1. PR 1: Phase 0 + Phase 1 (baseline tests + trait abstraction, no split yet).
2. PR 2: Phase 2 + Phase 3 (backend split + wrapper wiring).
3. PR 3: Phase 4 + Phase 5 (feature partitioning + regression cleanup).

## Final Target Outcome

1. Shared controller logic is backend-agnostic over a strict common trait.
2. FMC and SPI wrappers are each bound to their own PAC register layout.
3. Non-shared register semantics are not represented as shared APIs.
4. Test suite enforces separation and prevents regressions.
