# configure_timing Implementation Plan

Date: 2026-05-02

## Goal

Implement `configure_timing` faithfully against the silicon-tested behavior in
`aspeed-rust`, while keeping this SMC crate's structure and safety model.

## Baseline

Current behavior:
- `configure_timing` computes a divisor placeholder and returns success.
- It does not write any timing/divider bits to CS control registers.

Reference behavior (silicon-tested):
- `aspeed-rust/src/spi/spicontroller.rs::apply_clock_settings`
- Uses `aspeed_get_spi_freq_div` (from `aspeed-rust/src/spi/util.rs`) to build
  the AST divider field encoding.
- Reads CS control register, clears `SPI_CTRL_FREQ_MASK` (`0x0F00_0F00`), ORs
  in encoded divider, writes back.

## Step-by-Step Plan

### 1) Port the divider encoding logic to SMC helpers

File:
- `target/ast10x0/peripherals/smc/helpers.rs`

Actions:
- Add a constant:
  - `pub(crate) const SPI_CTRL_FREQ_MASK: u32 = 0x0F00_0F00;`
- Replace/retire the current shift-based `calculate_clock_divisor` helper.
- Add a faithful port function (same algorithm as `aspeed_get_spi_freq_div`):
  - Input: `sysclk_mhz`, `max_freq_mhz`
  - Output: encoded field for CS control registers (already aligned for OR)
  - Error on `max_freq_mhz == 0` (return `SmcError::HardwareError`)
- Keep this helper pure and side-effect free.

Notes:
- Use the same `div_arr` ordering as silicon-tested code:
  `[15, 7, 14, 6, 13, 5, 12, 4, 11, 3, 10, 2, 9, 1, 8, 0]`.
- Return value shape must match AST CS control register encoding:
  `(i << 24) | (div_val << 8)`.

### 2) Implement register programming in `configure_timing`

File:
- `target/ast10x0/peripherals/smc/controller.rs`

Actions:
- Replace TODO body in `configure_timing` with:
  - Calculate divider field using new helper (`sysclk_mhz = 200` for now).
  - Select CS register accessor by `cs`:
    - `0` -> `read_cs0_ctrl` / `write_cs0_ctrl`
    - `1` -> `read_cs1_ctrl` / `write_cs1_ctrl`
    - otherwise return `SmcError::HardwareError`
  - Preserve non-frequency bits:
    - `new = (current & !SPI_CTRL_FREQ_MASK) | encoded_div`
  - Write updated value to selected CS control register.

Constraints:
- Keep all MMIO unsafe access confined to `SmcRegisters`.
- `configure_timing` remains safe API surface.

### 3) Add/adjust unit tests for divider behavior

File:
- `target/ast10x0/peripherals/smc/helpers.rs` (`#[cfg(test)]` block)

Actions:
- Replace old tests that expected small shift counts.
- Add tests validating encoded output semantics:
  - `spi_freq_div(200, 25)` gives <= 25 MHz effective clock.
  - `spi_freq_div(200, 50)` gives <= 50 MHz effective clock.
  - `spi_freq_div(200, 0)` returns error.
- Add a tiny decode helper in tests only (if needed) to reconstruct divisor and
  verify resulting frequency bound (`effective <= requested`).

### 4) Keep QEMU expectations realistic

Files:
- Existing QEMU tests under `target/ast10x0/tests/smc/`

Actions:
- Do not add assertions that CS control readback reflects timing bits unless
  verified supported by the QEMU model.
- Rely on:
  - Unit tests for encoding correctness.
  - Existing init success path to confirm programming path executes.

Rationale:
- QEMU model does not necessarily emulate timing side effects/register fidelity.

### 5) Update status tracking

Files:
- `target/ast10x0/peripherals/smc/status.md` (reference copy)
- (optional mirror later in openprot if still maintaining identical docs)

Actions:
- Move "Timing register writes in `configure_timing`" from "Not yet
  implemented" to "Implemented" after code lands and tests pass.
- Remove/adjust the corresponding risk item if no longer applicable.

## Validation Plan

Build:
- `bazelisk build --platforms=//target/ast10x0 //target/ast10x0/peripherals:peripherals`

Unit tests:
- `bazelisk test --platforms=//target/ast10x0 //target/ast10x0/peripherals:smc_helpers_test`

QEMU smoke tests:
- `bazelisk test --config=virt_ast10x0 //target/ast10x0/tests/smc:smc_test`
- `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_qemu_erase_state_test`

## Out of Scope (this change)

- Timing calibration sweep logic.
- Dynamic HCLK probing/plumbing from clock subsystem.
- DMA completion API, DMA write, erase/program/verify flash ops.
- Richer `NorFlashErrorKind` mapping.

## Expected Outcome

After implementation:
- `configure_timing` will program CS0/CS1 frequency bits using AST-compatible
  divider encoding derived from silicon-tested logic.
- Existing safety boundary is preserved.
- Behavior remains QEMU-safe while improving silicon correctness.