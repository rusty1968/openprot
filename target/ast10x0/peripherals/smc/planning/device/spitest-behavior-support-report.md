# spitest Behavioral Support Report (Replacement SMC)

Date: 2026-05-03

## Goal

Treat `aspeed-rust/src/spi/spitest.rs` as a behavioral specification and assess whether the replacement under `reference/target/ast10x0/peripherals/smc` supports the exercised flows.

## Verdict Summary

- Supported: JEDEC read, CS0/CS1 routing checks, mapped reads, sector erase + page program + verify loop, SPI1/SPI2 controller wrappers, raw user-mode transfer path, DMA read primitive + IRQ/status handling, and policy-driven 3-byte/4-byte program+erase command dispatch in the device facade.
- Partially supported: explicit 4-byte command parity is implemented at policy/command-construction level, but current QEMU integration evidence is extension-point/configuration-oriented rather than full wire-level differentiation for all spitest SPI0/SPI1 scenarios; auto-DMA behavior tied to transfer length remains caller-driven.
- Not supported in this module: block-device facade equivalent to `NorFlashBlockDevice::from_jedec_id(...)`.
- Extracted outside SMC: SPIM monitor policy/control and SCU route/passthrough controls exist under `peripherals/spimonitor` and `peripherals/scu`; SMC wrappers do not own this behavior directly.

## Flow Matrix (spitest -> replacement)

1. JEDEC ID read (`test_read_jedec`, and raw `0x9F` transfer)
- spitest behavior: `nor_read_jedec_id()` and raw `transfer` read-id path.
- Status: Supported.
- Evidence:
  - spitest anchors: `test_read_jedec` and raw transfer path in `aspeed-rust/src/spi/spitest.rs` lines 193 and 669.
  - replacement supports JEDEC at device facade (`jedec_id`, `jedec`, `expect_jedec`) in `target/ast10x0/peripherals/smc/device/flash.rs` lines 208, 213, 394, 469.
  - exercised end-to-end in QEMU integration test `target/ast10x0/tests/smc/target_device_qemu_program_erase.rs` lines 154 and 162.

2. Controller bring-up for FMC/SPI wrappers
- spitest behavior: initialize FMC/SPI controllers, then execute read/write flows.
- Status: Supported.
- Evidence:
  - FMC wrapper create/init and ops in `target/ast10x0/peripherals/smc/fmc.rs` lines 25, 33, 42, 47, 90.
  - SPI wrapper create/init and ops in `target/ast10x0/peripherals/smc/spi.rs` lines 25, 38, 47, 52, 95.
  - backend uses FMC/SPI1/SPI2 constructors in `target/ast10x0/backend/flash/src/lib.rs` lines 52, 56, 60, 89, 96.

3. Read path via mapped flash window
- spitest behavior: `nor_read_data`/`nor_read_fast_4b_data` and compare with expected buffers.
- Status: Supported for mapped read semantics.
- Evidence:
  - mapped read implementation in `target/ast10x0/peripherals/smc/controller.rs` line 181.
  - per-CS translated read in device facade (`device_to_controller_offset` + `read`) in `target/ast10x0/peripherals/smc/device/flash.rs` lines 321 and 418.

4. Sector erase + page program + verify (write/readback)
- spitest behavior: erase sector, page program, wait-ready, readback compare.
- Status: Supported (policy-driven 3-byte/4-byte command addressing path for erase/program).
- Evidence:
  - erase/program/status polling and command dispatch in `target/ast10x0/peripherals/smc/device/flash.rs`.
  - command profile wiring supports both 3-byte and 4-byte opcode families (`ERASE_SECTOR_4K`/`ERASE_SECTOR_4K_4B`, `PAGE_PROGRAM`/`PAGE_PROGRAM_4B`) in `target/ast10x0/peripherals/smc/device/flash.rs`.
  - end-to-end program/erase + verify in `target/ast10x0/tests/smc/target_device_qemu_program_erase.rs` lines 166 and 173.

5. Multi-CS routing and CS validity handling
- spitest behavior: exercise CS0/CS1 across FMC and SPI paths.
- Status: Supported at transport/routing layer.
- Evidence:
  - `ChipSelect::Cs1` routing + invalid-CS guard in `target/ast10x0/peripherals/smc/controller.rs` lines 332-333.
  - dedicated multi-CS routing test in `target/ast10x0/tests/smc/target_device_qemu_multi_cs.rs` lines 69, 79, 98.
  - CS-local bounds checks via device facade in `target/ast10x0/tests/smc/target_device_qemu_multi_cs_offsets.rs` lines 121 and 127.

6. DMA read path and completion/error handling
- spitest behavior: comments indicate DMA path for larger reads, no IRQ handling in test.
- Status: Partially supported relative to spitest behavior style.
- Evidence:
  - replacement exposes explicit DMA read + status + clear + IRQ handler (`dma_read`, `dma_status`, `clear_dma_status`, `handle_dma_irq`) in `target/ast10x0/peripherals/smc/controller.rs` lines 201, 226, 233, 242 and wrappers in `fmc.rs`/`spi.rs` lines 47-67 and 52-67.
  - no automatic length-threshold routing (spitest uses threshold `DMA_MIN_LENGTH = 128` in `aspeed-rust/src/spi/spitest.rs` line 67); replacement requires caller to invoke DMA API directly.

7. 4-byte command addressing flow (SPI0/SPI1 in spitest)
- spitest behavior: uses explicit 4-byte read/program commands (`READ_FAST_4B`, `PP_4B`) and `AddressWidth::FourByte`.
- Status: Partially supported.
- Evidence:
  - spitest anchors in `aspeed-rust/src/spi/spitest.rs` lines 156, 173, 160, 177, 296.
  - replacement includes `FlashAddressingPolicy` + `FlashCommandProfile` and routes erase/program opcode+address-width through policy selection in `target/ast10x0/peripherals/smc/device/flash.rs`.
  - explicit override path (`with_addressing_policy(FourByteCommands)`) is integration-tested in `target/ast10x0/tests/smc/target_device_qemu_program_erase.rs`.
  - residual caveat: current QEMU model/testing does not provide exhaustive wire-level distinction for every spitest SPI0/SPI1 path variant, so parity is strong at API/command-construction level but still documented as partial at full behavioral evidence level.

4-byte wire-level evidence gap detail (why this remains partial):
- Proven today:
  - command-selection intent is correct at facade level (policy -> opcode + address-width mapping),
  - override path is plumbed through integration and exercised without regressions.
- Not yet proven end-to-end:
  - on-wire opcode differentiation for all targeted operations across SPI0/SPI1-style scenarios (`0x0C`/`0x12`/`0x21` vs 3-byte counterparts),
  - read-path parity for explicit command-driven `READ_FAST_4B` behavior (current replacement read path is primarily mapped-window based),
  - behavior at addresses that require 4-byte encoding (>16 MiB boundary semantics),
  - equivalence of completion/error behavior between 3-byte and 4-byte paths under stress/failure cases.
- Why current evidence is insufficient:
  - existing QEMU coverage validates integration and control-path selection but is not a full bus-level oracle for every SPI0/SPI1 variant used by spitest,
  - current test shape emphasizes facade behavior and success outcomes more than exhaustive transport capture/assertion.
- Closeout evidence required to mark this "Supported":
  - at least one test/harness that captures or deterministically asserts emitted opcode/address-width per operation,
  - directed tests for >16 MiB addressing semantics on a model/platform that distinguishes 3-byte vs 4-byte behavior,
  - read/write/erase parity matrix for SPI-like paths using 4-byte commands with explicit pass/fail assertions,
  - retained regression proof that 3-byte behavior remains unchanged.

8. Block-device facade from JEDEC
- spitest behavior: creates `NorFlashBlockDevice::from_jedec_id(...)` then runs block read/program/erase test.
- Status: Not supported in replacement SMC module.
- Evidence:
  - no `NorFlashBlockDevice` / `from_jedec_id` equivalent under `target/ast10x0/peripherals/smc`.
  - replacement backend path is `FlashBackend` + `SpiNorFlash` in `target/ast10x0/backend/flash/src/lib.rs` lines 110 and 143.

9. SPIM monitor route / pinmux-specific dual target path in spitest (`SPI2@0` and `SPI2@1`)
- spitest behavior: exercises monitor selection (`spim`) and pinctrl-dependent routing.
- Status: Supported via extracted modules (outside SMC), with integration required at call sites.
- Evidence:
  - SPI monitor controller abstraction is implemented in `target/ast10x0/peripherals/spimonitor/controller.rs` (`SpiMonitorController` + typestate facade).
  - SPIM instance mapping (`Spim0..Spim3`) and SPIPF register blocks are implemented in `target/ast10x0/peripherals/spimonitor/registers.rs`.
  - SCU routing APIs for internal master detour, passthrough, and mux control are implemented in `target/ast10x0/peripherals/scu/routing.rs` (`set_spim_internal_master_route`, `set_spim_passthrough`, `set_spim_ext_mux`, `set_spim_miso_multi_func`) with public enums in `target/ast10x0/peripherals/scu/types.rs`.
  - SMC wrappers remain transport-only (`target/ast10x0/peripherals/smc/fmc.rs`, `target/ast10x0/peripherals/smc/spi.rs`), so spimonitor/scu setup must be performed by higher-level board/app code.

## Overall Conclusion

Using spitest as behavior spec, the replacement SMC module is functionally close for core transport and basic NOR operation flows (JEDEC, read, policy-driven erase/program, CS routing, DMA primitives). The primary parity deltas are:
- remaining evidence gap for full wire-level 4-byte behavioral parity across all spitest-style SPI0/SPI1 scenarios (despite implemented policy-level support),
- no block-device construction path mirroring `NorFlashBlockDevice::from_jedec_id`,
- SPIM monitor/pinmux-routing behavior is split into `spimonitor` + `scu` modules and must be composed by integration code,
- and no implicit auto-DMA threshold behavior (caller-driven DMA only).

## Evidence Closeout Criteria for 4-byte Parity

To move flow #7 from "Partially supported" to "Supported", the following must be satisfied:

1. Wire-level command evidence
- Demonstrate operation-level emission/selection of 4-byte command family where policy requires it (`READ_FAST_4B`, `PP_4B`, 4-byte sector erase opcode path).

2. Address-boundary behavioral evidence
- Demonstrate correct behavior for offsets requiring 4-byte addresses (including boundary and negative cases), not only constructor/policy assertions.

3. Transport-path parity evidence
- Show equivalent completion/error semantics for 4-byte command flow relative to existing 3-byte path on representative SPI0/SPI1-style scenarios.

4. Regression guard
- Preserve and re-run existing 3-byte path tests to prove no regression while adding 4-byte parity evidence.
