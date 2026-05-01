# AST10x0 Peripherals SMC Status

Date: 2026-04-30

## Scope

This status reflects:
- Review baseline from [SPI_REVIEW.md](SPI_REVIEW.md)
- Current implementation under [openprot/target/ast10x0/peripherals/smc](openprot/target/ast10x0/peripherals/smc)

## Current Implementation Snapshot

Implemented modules:
- [openprot/target/ast10x0/peripherals/smc/mod.rs](openprot/target/ast10x0/peripherals/smc/mod.rs)
- [openprot/target/ast10x0/peripherals/smc/registers.rs](openprot/target/ast10x0/peripherals/smc/registers.rs)
- [openprot/target/ast10x0/peripherals/smc/types.rs](openprot/target/ast10x0/peripherals/smc/types.rs)
- [openprot/target/ast10x0/peripherals/smc/controller.rs](openprot/target/ast10x0/peripherals/smc/controller.rs)
- [openprot/target/ast10x0/peripherals/smc/interrupts.rs](openprot/target/ast10x0/peripherals/smc/interrupts.rs)

Supporting crate files:
- [openprot/target/ast10x0/peripherals/BUILD.bazel](openprot/target/ast10x0/peripherals/BUILD.bazel)
- [openprot/target/ast10x0/peripherals/lib.rs](openprot/target/ast10x0/peripherals/lib.rs)
- [openprot/target/ast10x0/peripherals/README.md](openprot/target/ast10x0/peripherals/README.md)

## Build Status

Current state: Blocked by dependency generation/auth during analysis, not by SMC Rust compile errors.

Observed failure while evaluating crate universe:
- Cargo/Bazel attempts to update a git dependency for tock registers via SSH
- Fails with authentication/revision retrieval error
- Build stops at analysis stage

Relevant files involved in dependency resolution:
- [openprot/third_party/crates_io/Cargo.toml](openprot/third_party/crates_io/Cargo.toml)
- [openprot/third_party/crates_io/Cargo.lock](openprot/third_party/crates_io/Cargo.lock)

Note:
- SMC types were migrated to embedded-storage error trait in [openprot/target/ast10x0/peripherals/smc/types.rs](openprot/target/ast10x0/peripherals/smc/types.rs)
- embedded-storage was added to [openprot/target/ast10x0/peripherals/BUILD.bazel](openprot/target/ast10x0/peripherals/BUILD.bazel) and [openprot/third_party/crates_io/Cargo.toml](openprot/third_party/crates_io/Cargo.toml)
- Full validation is pending successful dependency resolution

## SPI_REVIEW Alignment

Reference sections:
- [SPI_REVIEW.md](SPI_REVIEW.md#L830) (Recommended Fixes)
- [SPI_REVIEW.md](SPI_REVIEW.md#L810) (Summary Table)

### P1 Critical Items

1. Unsafe constructor with ownership contract: Implemented
- [openprot/target/ast10x0/peripherals/smc/controller.rs](openprot/target/ast10x0/peripherals/smc/controller.rs#L35)
- [openprot/target/ast10x0/peripherals/smc/registers.rs](openprot/target/ast10x0/peripherals/smc/registers.rs#L33)

2. Consolidated unsafe access perimeter: Implemented
- Centralized pointer dereference in regs accessor
- [openprot/target/ast10x0/peripherals/smc/registers.rs](openprot/target/ast10x0/peripherals/smc/registers.rs#L45)

3. Safety comments on unsafe operations: Mostly implemented
- Constructor and register access have safety rationale
- Remaining opportunity: tighten comments around memory window read in controller

### P2 High Items

4. Initialization guarantees: Partially implemented
- init is explicit and required by runtime state checks
- Still deferred (not enforced by type-state/builder)

5. Safe interrupt wrapper: Implemented
- [openprot/target/ast10x0/peripherals/smc/interrupts.rs](openprot/target/ast10x0/peripherals/smc/interrupts.rs)

6. Input validation: Partially implemented
- Config presence check and segment size overflow checks exist
- Missing checks for read bounds and some DMA argument edge cases

### P3 Medium Items

7. Register side-effect docs: Partial
- Register wrappers are named and documented by offset
- Higher-level side effects and operational sequencing need deeper documentation

8. Error model granularity: Partial
- Migrated from embedded-hal SPI error to embedded-storage NorFlashError
- Current mapping still collapses to Other kind
- Retryable path exists via nb WouldBlock conversion

## Functional Coverage

Implemented:
- Base address/window mapping by controller
- Init path for config, segment setup, and optional interrupt bit enable
- PIO read via mapped flash window
- DMA read start sequence
- Interrupt decode helper
- Unit tests for segment encode and clock divisor helpers

Not yet implemented or incomplete:
- Timing register programming in configure_timing (currently TODO)
- DMA completion polling/wait API and status-clear integration in controller
- DMA write path
- Strong bounds checks for read offset and length
- Full flash operations (erase/program/verify) layer
- Use of dma_enabled field for behavior gating

## Risks and Blockers

1. Dependency pipeline blocker
- Build analysis currently blocked by cargo git fetch/auth issue for tock source

2. Runtime correctness risks
- read currently has no explicit capacity-window bounds guard
- dma_read does not validate all argument corner cases

3. Completeness risks
- timing config is not yet applied to hardware control registers
- controller state includes Error variant that is currently unused

## Recommended Next Actions

1. Unblock dependency resolution
- Ensure cargo can fetch locked git dependencies in bazel environment
- Re-run full build for peripherals target after lock/dependency resolution

2. Finish controller correctness items
- Implement timing writes in configure_timing
- Add read and DMA bounds/argument validation
- Add DMA completion/status API in controller

3. Improve error and API semantics
- Map SmcError variants to more specific NorFlashErrorKind values where possible
- Decide and document blocking vs non-blocking DMA contract for callers

4. Prepare integration progression
- Add minimal flash operation facade (read/erase/program placeholders or real ops)
- Add integration tests for init plus read plus DMA round-trip
