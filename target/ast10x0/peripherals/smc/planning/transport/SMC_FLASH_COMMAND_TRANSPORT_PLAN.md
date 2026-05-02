# SMC Flash Command Transport Implementation Plan

Date: 2026-05-02

## Goal

Implement the actual command transport behind the device-layer write/erase/status
APIs in `target/ast10x0/peripherals/smc/device/flash.rs` using the AST10x0 SMC
user-mode command path, not a separate software SPI engine.

This plan is intentionally transport-focused. It does not redefine the Phase 3B
device API; it defines how command bytes and payloads should reach the flash
through the existing controller/wrapper stack.

## Current State In This Repo

Implemented today:
- Device-layer API scaffolding exists in `device/flash.rs`
- `issue_command()` and `read_status_impl()` are the transport seam
- Read path already works through the mapped flash aperture
- Wrapper/controller layering is stable (`FmcReady`, `SpiReady`, `ReadySmc`)

Missing today:
- No explicit register-layer or controller-layer abstraction for user-mode SPI
  command transactions
- No documented CS control mode transitions for command phase vs data phase
- No test that proves write-enable, read-status, erase, or page-program through
  the in-repo transport path

## Evidence Lifted From `aspeed-rust`

The nearest verified implementation surface in this workspace is not in the
current `reference` SMC HAL; it is in `aspeed-rust`, specifically the SPI/FMC
controller stack under `aspeed-rust/src/spi/`.

/home/rusty1968/work/storage/aspeed-rust

### 1. User-mode command transport is performed through the flash aperture

Evidence:
- `aspeed-rust/src/spi/fmccontroller.rs`
- `spi_nor_transceive_user()`

Observed behavior:
- Enter user mode on the selected CS
- Write opcode bytes through the mapped AHB flash window
- Write address bytes through the same window
- Write dummy bytes if required
- Then either write payload bytes or read RX bytes through that same aperture
- Exit user mode and restore normal read mode

This is the key conclusion: the transport is not modeled as a separate command
FIFO in the lifted code. It is serialized by toggling controller mode and using
the mapped flash aperture as the byte stream port.

### 2. User mode is controlled by CS control register values

Evidence:
- `aspeed-rust/src/spi/consts.rs`
- `ASPEED_SPI_USER = 0x3`
- `ASPEED_SPI_USER_INACTIVE = 0x4`
- `aspeed-rust/src/spi/fmccontroller.rs`
- `activate_user()` / `deactivate_user()`

Observed behavior:
- `activate_user()` writes `user_reg | ASPEED_SPI_USER_INACTIVE`, then `user_reg`
- `deactivate_user()` writes `user_reg | ASPEED_SPI_USER_INACTIVE`, then restores
  `normal_read`

This gives the minimal control-state sequence we should reproduce in the HAL.

### 3. Raw byte movement is done with volatile reads/writes on the aperture

Evidence:
- `aspeed-rust/src/spi/util.rs`
- `spi_write_data()`
- `spi_read_data()`

Observed behavior:
- Full 32-bit words are transferred when possible
- Tail bytes are transferred with byte-wide volatile access
- No extra protocol wrapper is needed at the byte-stream layer

This is sufficient to implement a local, auditable transport in the controller
layer without importing the full `aspeed-rust` SPI object model.

### 4. SPI NOR operations are already expressed as transportable command objects

Evidence:
- `aspeed-rust/src/spi/norflash.rs`
- `nor_write_enable()`
- `nor_sector_erase()`
- `nor_page_program()`
- `nor_wait_until_ready()`

Observed behavior:
- `WREN` is a command with no address and no data payload
- `RDSR` is a command with 1-byte read payload
- `SE`/`PP` are command + address (+ data for program)
- WIP polling loops on `RDSR` until bit 0 clears

This confirms the device-layer APIs already match the transport shape needed by
the hardware path.

## Design Decision

Use a narrow controller-level raw transaction API as the transport primitive:

```rust
fn transceive_user(&self, cmd: &[u8], tx_payload: &[u8], rx: &mut [u8])
    -> Result<(), SmcError>;
```

Why this shape:
- Small enough to keep the transport generic
- Matches the proven `opcode + optional address + optional data` flow from
  `aspeed-rust`
- Avoids importing `Jesd216Mode`, `SpiNorCommand`, or the broader SPI stack into
  this HAL prematurely
- Keeps policy in `device/flash.rs` and mechanism in controller/wrapper layers

## Implementation Plan

### Phase T1: Register and Controller Transport Primitive

Implement a controller-level user transaction path on top of existing CS0
control and flash window access.

Tasks:
- Add a controller helper that:
  - captures current CS0 control register value
  - derives a user-mode control value while preserving frequency bits
  - activates user mode
  - streams command bytes to the flash aperture
  - streams TX payload bytes, if any
  - reads RX payload bytes, if any
  - deactivates user mode and restores normal-read control value
- Keep the first implementation scoped to CS0 only
- Return `SmcError::HardwareError` if controller is not ready

Acceptance criteria:
- `status()` can execute `RDSR` successfully through the transport
- Existing read-path behavior remains unchanged

### Phase T2: Wrapper Exposure

Expose the controller primitive through both wrapper types.

Tasks:
- Add `transceive_user()` on `FmcReady`
- Add `transceive_user()` on `SpiReady`
- Keep wrappers thin; no transport policy belongs here

Acceptance criteria:
- Device layer does not need direct register access
- Same device-layer transport code works for FMC and SPI wrappers

### Phase T3: Device-Layer Integration

Replace placeholder transport with wrapper-backed command execution.

Tasks:
- Wire `issue_command()` to `transceive_user(cmd, payload, &mut [])`
- Wire `read_status_impl()` to `transceive_user(&[RDSR], &[], &mut [status])`
- Keep WIP polling in device layer

Acceptance criteria:
- `status()` returns the flash status byte
- `wait_write_complete()` uses real transport, not a placeholder error

### Phase T4: Write/Erase Enablement

Once transport works for status reads, enable write sequence tests.

Tasks:
- `erase_sector()` issues `WREN`, then `SE`, then polls WIP
- `program_page()` issues `WREN`, then `PP`, then polls WIP
- `verify()` remains read-back comparison through the existing read facade

Acceptance criteria:
- Device-layer write/erase methods no longer return transport placeholders
- Deterministic timeout/error mapping remains intact

## Constraints And Guardrails

1. Keep transport below the device layer.
- `device/flash.rs` should assemble commands and interpret status.
- `controller.rs` should only perform byte transport.

2. Do not introduce duplicate command state machines.
- Avoid adding a second SPI framework in the peripheral HAL.
- Reuse the mapped-aperture + user-mode pattern proven in `aspeed-rust`.

3. Start with CS0 only.
- Current device facade constructors are CS0-oriented.
- Multi-CS command routing can be added after single-CS write path is stable.

4. Preserve normal-read configuration.
- Entering and exiting user mode must restore the controller’s normal-read path
  so PIO reads continue to behave exactly as before.

## Test Plan For Transport

### Immediate transport smoke

Extend existing device smoke tests to assert:
- `status()` succeeds on FMC facade
- `status()` succeeds on SPI facade

### Write-sequence validation

After transport is live:
- QEMU or silicon test for `WREN` followed by `RDSR`
- Sector erase test with WIP polling
- Page program test with read-back verification

### Regression protection

Continue running:
- `//target/ast10x0/tests/smc:smc_device_test`
- `//target/ast10x0/tests/smc:smc_device_qemu_erase_state_test`

## Risks

1. User-mode control bits are currently derived from lifted code, not from an
   in-repo datasheet-backed register definition.
2. CS0-only transport is sufficient for current facade constructors but not for
   future multi-CS device routing.
3. QEMU may not model all command-side effects, so final erase/program proof may
   require silicon validation.

## Recommended Sequence

1. Implement controller `transceive_user()`
2. Expose it through `FmcReady` and `SpiReady`
3. Wire `issue_command()` and `read_status_impl()`
4. Add `status()` smoke assertions to existing device tests
5. Enable erase/program tests only after transport is verified