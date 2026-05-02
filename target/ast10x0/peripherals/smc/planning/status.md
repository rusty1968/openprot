# AST10x0 Peripherals SMC Status

Date: 2026-05-02

## Scope

This status reflects:
- Review baseline from [SPI_REVIEW.md](SPI_REVIEW.md)
- Implementation under `target/ast10x0/peripherals/smc/` (present in both
  `openprot` and `reference` repos — the two are byte-for-byte identical as of
  this date)
- QEMU bare-metal test scaffold under `target/ast10x0/tests/smc/` (`reference`
  repo only)

## Build Status

`reference` repo: **✅ passing**
```
bazelisk build --platforms=//target/ast10x0 //target/ast10x0/peripherals:peripherals
```
- `embedded-storage = "0.3"` added to `reference/third_party/crates_io/Cargo.toml`
  to satisfy the crate label used by the SMC BUILD target.

## SPI_REVIEW Alignment

Reference sections:
- [SPI_REVIEW.md](SPI_REVIEW.md#L830) (Recommended Fixes)
- [SPI_REVIEW.md](SPI_REVIEW.md#L810) (Summary Table)

### P1 Critical — ✅ Done

1. Unsafe constructor with ownership contract — `Smc::new()` + `SmcRegisters::new()`
2. Consolidated unsafe perimeter — single `regs()` method in `registers.rs`
3. Safety comments on unsafe operations — constructor, register access, PIO read

### P2 High — ✅ Done

4. Initialization guarantees — type-state `Smc<Uninitialized>` → `Smc<Ready>`;
   I/O APIs unavailable before `init()`
5. Safe interrupt wrapper — `SmcInterruptDecoder` in `interrupts.rs`
6. Input validation — ✅ bounds-checked: `validate_mapped_range` on PIO read,
   `validate_dma_read` on DMA start; both exercised by QEMU tests

### P3 Medium — Partial

7. Register side-effect docs — wrappers named by offset; higher-level sequencing
   docs still needed
8. Error model granularity — `NorFlashError` implemented; kind still collapses to
   `Other`; retryable `nb::WouldBlock` path exists

## Functional Coverage

### Implemented
- Base address/window mapping per controller (FMC / SPI1 / SPI2)
- Type-state init path: construction → init → ready
- Init: config register, segment registers, optional interrupt-enable bit
- CS timing programming in `configure_timing` via CS control register divider
   field (`SPI_CTRL_FREQ_MASK`)
- PIO read via mapped flash window with bounds validation
- DMA read start sequence with DRAM alignment and range validation
- Interrupt decoder (`SmcInterruptDecoder`)
- Host-side unit tests: segment encode, clock divisor, capacity arithmetic,
   DMA argument validation (`//target/ast10x0/peripherals:smc_helpers_test`)

### Not yet implemented
- DMA completion polling / status-clear API in controller
- DMA write path
- Full flash operation layer (erase / program / verify)
- `dma_enabled` field used for runtime gating
- `SmcError` kind variants more specific than `Other`

## QEMU Test Scaffold (reference repo)

Added `target/ast10x0/tests/smc/` with two test targets:

| Target | Tag | Runs with `--config=virt_ast10x0` | Explicit invocation |
|---|---|---|---|
| `smc_test` | `kernel` | ✅ automatic | `bazelisk test --config=virt_ast10x0 //target/ast10x0/tests/smc:smc_test` |
| `smc_qemu_erase_state_test` | `kernel`, `integration` | ❌ excluded by k_common | `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_qemu_erase_state_test` |

`smc_test` (portable — QEMU + silicon safe):
- Init and Ready-state assertion
- PIO read success path (byte count check; content not asserted)
- PIO read bounds rejection (`SmcError::InvalidCapacity`)
- DMA unaligned-DRAM rejection (`SmcError::InvalidCapacity`)

`smc_qemu_erase_state_test` (QEMU-only — tagged `integration`):
- All of the above, plus:
- PIO read erase-state check: asserts every byte is `0xFF`, confirming
  segment register encoding → flash window → `m25p80` model → buffer

See `target/ast10x0/tests/smc/README.md` for full QEMU-vs-silicon coverage
analysis and future extension plan.

## Remaining Risks

1. `Error` variant in `SmcState` is dead code — add a path that sets it or
   remove it.
2. QEMU does not model flash timing fidelity, so divider correctness is
   validated by algorithm parity and silicon-tested mapping rather than QEMU
   timing behavior.

## Next Actions

1. Add DMA completion / status-clear API to `Smc<Ready>` and cover it in the
   QEMU test scaffold.
2. Consider more specific `NorFlashErrorKind` mappings for `SmcError` variants.
3. Use [SMC_FLASH_COMMAND_TRANSPORT_PLAN.md](SMC_FLASH_COMMAND_TRANSPORT_PLAN.md)
   as the implementation plan for Phase 3B command transport wiring.
