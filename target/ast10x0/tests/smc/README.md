# AST10x0 SMC QEMU Test Suite

This package provides a minimal bare-metal scaffold for exercising the SMC
peripheral crate under the `virt_ast10x0` QEMU configuration (QEMU
`ast1030-evb` machine + ARMv7-M semihosting).

## Current coverage

Controller-level coverage:
- Construct an FMC-backed `Smc<Uninitialized>` controller
- Run the SMC init sequence against the AST10x0 MMIO region
  (`0x7E620000`) and validate state transitions
- Verify invalid DMA requests are rejected on the runtime path

Device-level coverage (`SpiNorFlash` facade):
- Build facade from both FMC and SPI wrappers
- Verify `status()` command transport succeeds on both constructor paths
- Verify read path and range rejection (`SmcError::InvalidCapacity`)
- QEMU erase-state check (`0xFF`) through facade read path
- Program one page, verify programmed bytes, erase sector, verify erased bytes

## Build and run

Build only:

```console
bazelisk build //target/ast10x0/tests/smc:smc
```

Run baseline controller smoke test under QEMU:

```console
bazelisk test --config=virt_ast10x0 //target/ast10x0/tests/smc:smc_test
```

Streamed output (shows kernel boot log):

```console
bazelisk test --config=virt_ast10x0 //target/ast10x0/tests/smc:smc_test \
  --test_output=streamed
```

Run device-level portable smoke test:

```console
bazelisk test --config=virt_ast10x0 //target/ast10x0/tests/smc:smc_device_test
```

Run QEMU-only device integration tests (explicitly include `integration`):

```console
bazelisk test --config=virt_ast10x0 --test_tag_filters= \
  //target/ast10x0/tests/smc:smc_device_qemu_erase_state_test \
  //target/ast10x0/tests/smc:smc_device_qemu_program_erase_test
```

## What QEMU validates faithfully

Based on the QEMU model for `ast1030-evb` (see
[`smc-qemu-test.md`](../../../../../smc-qemu-test.md) for full analysis):

- **Register layout** — every register write reaches the modelled FMC
  peripheral at `0x7E620000`; offset bugs are caught immediately.
- **SMC init sequence** — config, segment, and SPI-mode register
  programming reaches the peripheral model.
- **DMA register dance** — `R_DMA_ADDR`, `R_DMA_LEN`, `R_DMA_CTRL`
  writes, and the `DMA_STATUS` / `INTR_CTRL` bit paths are all modelled.
- **IRQ wiring** — NVIC vector 39 (FMC), 65 (SPI1), 66 (SPI2) are
  correctly wired in the ARMv7-M NVIC model; handler registration and
  masking can be validated here.
- **PIO flash-window reads** — `m25p80` chips (one per CS) are
  instantiated by `aspeed_board_init_flashes`; reads from the FMC flash
  window (`0x80000000`) hit a real emulated chip.
- **Input validation** — argument-rejection paths (`SmcError::*`) are
  pure Rust logic and execute identically on QEMU and silicon.

Default QEMU flash chips on `ast1030-evb`:

| Controller | Chip | Size | CS count |
|---|---|---|---|
| FMC | `w25q80bl` | 1 MB | 2 |
| SPI1 | `w25q256` | 32 MB | 2 |
| SPI2 | `w25q256` | 32 MB | 2 |

## What QEMU does NOT validate (silicon-only)

A green QEMU run does **not** cover:

- **Timing/frequency registers** — `R_TIMINGS`, `R_TIMINGS2`,
  per-CS clock-divisor and dummy-cycle fields are stored but never
  consulted by the QEMU transfer path. Wrong divisor math passes
  silently.
- **Pin multiplexing** — SCU `0xF0` QSPI mux, `PINCTRL_FMC_QUAD`,
  `PINCTRL_SPIM*` tables. Writes update register state; they do not
  gate flash access in QEMU.
- **DMA realism** — DMA completes synchronously inside the MMIO write
  that enables it. DMA-vs-CPU races, mid-transfer aborts, throughput
  budgets, and watchdog-pet-during-DMA scenarios are unmodelled.
- **Timing calibration** — the `R_DMA_CHECKSUM` sweep always finds a
  passing window because `m25p80` has no setup/hold violations.
- **SPIM routing constraint** (SPI2 CS0/SPIM3) — the SPI Pin Filter
  blocks (`0x7E791000–0x7E794000`) are not in the AST1030 memmap.
- **Flash chip persistence** — `m25p80` instances are volatile by
  default; read-only test variants need `-drive` backing to see
  meaningful contents.
- **IRQ real-time latency, tail-chaining, dual-image boot switch.**

## Future extensions

Targeted next steps in priority order:

1. **WIP/WEL semantics checks** — assert expected status-bit transitions
  around `WREN`, `PP`, and erase operations.
2. **Cross-page and partial-page programming checks** — validate policy and
  error mapping for invalid page boundaries and lengths.
3. **DMA register dance** — program and trigger a DMA transfer; verify
  `DMA_STATUS` is set afterwards (QEMU DMA is synchronous, so no
  polling needed).
4. **IRQ handler wiring** — register an FMC IRQ 39 handler, trigger DMA,
  assert the ISR fires.
5. **Backed flash image** — pass a known binary via `-drive` and assert
  on specific byte values from the PIO read path.

## Test tagging convention

| Target | Tags | Runs with `--config=virt_ast10x0` | Explicit invocation |
|---|---|---|---|
| `smc_test` | `kernel` | ✅ automatically | `bazelisk test --config=virt_ast10x0 ...` |
| `smc_qemu_erase_state_test` | `kernel`, `integration` | ❌ excluded by k_common | `bazelisk test --config=virt_ast10x0 --test_tag_filters= ...` |
| `smc_device_test` | `kernel` | ✅ automatically | `bazelisk test --config=virt_ast10x0 //target/ast10x0/tests/smc:smc_device_test` |
| `smc_device_qemu_erase_state_test` | `kernel`, `integration` | ❌ excluded by k_common | `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_erase_state_test` |
| `smc_device_qemu_program_erase_test` | `kernel`, `integration` | ❌ excluded by k_common | `bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/smc:smc_device_qemu_program_erase_test` |

The `integration` tag marks tests whose assertions are QEMU-specific and
would fail on silicon (e.g. asserting erased-state `0xFF` byte values).
When a future silicon runner config is added, silicon-only tests will get
the same tag and be excluded from QEMU runs symmetrically.

To pre-load flash content for backed tests, pass `-drive` arguments to
the QEMU invocation:

```
-drive file=fmc_cs0.bin,if=mtd,unit=0,format=raw   # FMC CS0
-drive file=fmc_cs1.bin,if=mtd,unit=1,format=raw   # FMC CS1
-drive file=spi1_cs0.bin,if=mtd,unit=2,format=raw  # SPI1 CS0
-drive file=spi1_cs1.bin,if=mtd,unit=3,format=raw  # SPI1 CS1
-drive file=spi2_cs0.bin,if=mtd,unit=4,format=raw  # SPI2 CS0
-drive file=spi2_cs1.bin,if=mtd,unit=5,format=raw  # SPI2 CS1
```

Unit numbering follows the order of `aspeed_board_init_flashes` calls in
`aspeed_minibmc_machine_init`.