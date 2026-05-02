# SMC Flash Command Transport Status

Date: 2026-05-02 (updated 2026-05-02, all three simplifications resolved)

## Summary

The SMC flash command transport is implemented for the current Phase 3B scope
and wired end-to-end:
- controller raw user-mode transport exists with per-phase IO mode register writes
- `TransferMode` enum added; `transceive_user` now accepts a mode parameter
- `normal_read_ctrl` stored at init time and restored after every transaction
- `AddressWidth` enum guards address byte emission in the device layer
- FMC/SPI wrappers expose the transport API with mode parameter
- device-layer status/read/program/erase paths use the transport
- QEMU integration coverage includes program+verify+erase+verify with WEL/WIP assertions
- All three aspeed-rust simplifications resolved

## Current Implementation Status

### Completed

1. Controller-level transport primitive implemented.
- `ReadySmc::transceive_user(cmd, tx_payload, rx, mode: TransferMode)` in
	`target/ast10x0/peripherals/smc/controller.rs`.
- Guards runtime state (`SmcState::Ready`), toggles user mode, and now writes
	the CS control register once per phase (cmd / addr+TX / RX) with the IO mode
	bits derived from `TransferMode`, matching aspeed-rust `spi_nor_transceive_user()`.
- Restores normal-read control register value after each transaction.

2. Wrapper exposure completed.
- `FmcReady::transceive_user(..., mode: TransferMode)` forwards to `ReadySmc`.
- `SpiReady::transceive_user(..., mode: TransferMode)` forwards to `ReadySmc`.
- `TransferMode` is re-exported from `mod.rs` as part of the public HAL surface.

3. Device-layer integration completed.
- `SpiNorFlash::issue_command()` uses wrapper `transceive_user` for write-path
	command dispatch.
- `SpiNorFlash::read_status_impl()` uses `RDSR` over `transceive_user`.
- `wait_write_complete()` now polls real status transport and returns timeout on
	exhausted poll budget.
- `erase_sector()` and `program_page()` perform `WREN` sequencing and WIP polling
	through the real command path.

4. Test coverage extended for transport-dependent behavior.
- Added QEMU integration test:
	`target/ast10x0/tests/smc/target_device_qemu_program_erase.rs`
	(program page, verify data, erase sector, verify erased bytes).
- Raw transport status-bit assertions: WIP=0 at idle and post-op; WEL=1 after
	each `WREN` command via raw `transceive_user` call.
- Bazel target added:
	`//target/ast10x0/tests/smc:smc_device_qemu_program_erase_test`.

## Validation Snapshot

The following tests were run and passed on QEMU (`--config=virt_ast10x0`):

1. `//target/ast10x0/tests/smc:smc_device_qemu_program_erase_test`
2. `//target/ast10x0/tests/smc:smc_device_qemu_erase_state_test`
3. `//target/ast10x0/tests/smc:smc_device_test`

## Scope Notes / Limitations

1. Transport is currently CS0-oriented in practice.
- This matches the current facade constructors and test usage.

2. Error signaling — COMPLETED.
- `SmcError::ControllerNotReady` variant added; returned by `transceive_user`
  and `dma_read` when state is not `Ready`.
- Negative test `smc_error_granularity_test` verifies the variant is emitted
  correctly after a DMA kick-off leaves the controller non-ready.

3. QEMU model limits still apply.
- Functional command/data sequencing is validated, but timing-sensitive and
	silicon-specific effects are not fully represented by QEMU.

## Known Simplifications Versus aspeed-rust

These are intentional scope reductions, not bugs. Each has a concrete upgrade
path.

### 1. Per-phase IO mode register writes — COMPLETED

aspeed-rust's `spi_nor_transceive_user()` rewrites the CS control register once
per phase to embed bus-width bits. This pattern is now implemented in the
reference HAL.

`transceive_user` now accepts `mode: TransferMode` and writes the CS control
register three times inside the active user-mode window:

```
write cs_ctrl(user_base | mode.cmd_io_bits())   ; cmd phase
write cs_ctrl(user_base | mode.addr_io_bits())  ; addr/TX phase
write cs_ctrl(user_base | mode.data_io_bits())  ; RX phase
```

`TransferMode` covers `Mode111`, `Mode112`, `Mode122`, `Mode114`, `Mode144`.
All current call sites pass `TransferMode::Mode111`. Dual/Quad paths can now
be enabled by passing the appropriate variant without further transport changes.

### 2. Normal-read restore value — COMPLETED

aspeed-rust stores a pre-negotiated `normal_read` register word and restores
it unconditionally on `deactivate_user`. The reference now matches this.

`Smc<Ready>` carries a `normal_read_ctrl: u32` field populated by `init()`
immediately after all timing and config writes. `transceive_user` derives
`user_base` from this field and restores it on deactivate instead of using a
per-call read-back snapshot.

### 3. Address width is implicit, not guarded — COMPLETED

The device layer previously assembled 3-byte address commands by slicing
`offset.to_be_bytes()[1..]` with no explicit width selection. This is now
resolved.

`AddressWidth` enum (`None` / `ThreeByte` / `FourByte`) added to `types.rs`
and re-exported from `mod.rs`. A private `encode_addr_cmd(opcode, offset,
width)` helper in `device/flash.rs` builds the exact command byte slice.
`erase_sector` and `program_page` both call it with `AddressWidth::ThreeByte`
explicitly, matching aspeed-rust's selection pattern.

## Next Recommended Step

All three aspeed-rust faithfulness gaps are resolved. Remaining transport
gaps from the `Scope Notes / Limitations` section:

1. **Multi-CS command routing** — transport is currently CS0-only. Once
   multi-CS facade constructors are added, `transceive_user` will need a `cs`
   parameter and matching CS control register selection.

3. **Silicon validation** — QEMU confirms functional command sequencing but
   does not model timing, frequency, or pin-mux constraints.
