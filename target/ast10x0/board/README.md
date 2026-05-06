# AST10x0 Board Descriptors

Board-level descriptors that bind the AST10x0 SMC peripheral, SCU SPI-monitor
routing, and SPIPF policy into a single value backends can apply at init time.

The peripheral crate (`//target/ast10x0/peripherals`) exposes register-level
controllers that are intentionally board-agnostic. This crate is where
per-board topology lives: which SMC controller is wired up, which flash sits
on each chip-select, how the SPI master is routed through a SPI-monitor
instance, and what opcode policy is programmed and locked on that monitor.

## What it provides

- `Ast10x0BoardDescriptor` — controller choice, CS0/CS1 `FlashConfig`s,
  unknown-JEDEC handling policy, optional SPIM wiring, and the
  `MonitorPolicy` to lock when SPIM is in use.
- `UnknownJedecPolicy` — board-integration choice between strict reject and
  conservative fallback to configured geometry.
- `SpimWiring` — the four SCU0F0 fields (instance, source, passthrough,
  external mux) plus the MISO multi-function pin choice that together
  determine the monitor route for a given SPI master.
- `apply_spim_wiring()` — single-shot helper that validates the route,
  programs SCU routing, applies the monitor policy, and **locks** the SPIPF
  block. The lock is one-way; an empty policy combined with a lock will
  brick the SPI bus until reset, so callers must pass a vetted preset.
- `presets::bmc_default_policy()` — opcode allow-list covering the BMC's
  normal flash opcodes (READ/FAST_READ, PP, SE_4K, RDSR/WREN/WRDI, RDID,
  RSTEN/RST) in both 3-byte and 4-byte addressing variants.

## Built-in descriptors

| Constructor | Controller | Flash | SPIM route |
|---|---|---|---|
| `ast10x0_qemu_default()` | FMC | 1 MiB on CS0 | none (FMC has no SPIM path) |
| `ast10x0_qemu_default_spi1()` | SPI1 | `winbond_w25q256` on CS0 | SPIM0, BMC default policy |
| `ast10x0_qemu_default_spi2()` | SPI2 | `winbond_w25q256` on CS0 | SPIM2, BMC default policy |

## Building

```console
bazel build //target/ast10x0/board_descriptors:ast10x0_board_descriptors
```

## Design notes

- FMC descriptors must have `spim_wiring == None`; Spi1/Spi2 descriptors
  must have `spim_wiring == Some(_)`. `apply_spim_wiring` returns
  `SpimWiringError::InvalidController` on FMC and `RouteMismatch` if the
  wiring source disagrees with the controller being initialized.
- `Ast10x0BoardDescriptor` is not `Eq`/`PartialEq` because the embedded
  `MonitorPolicy` is not (yet) — compare individual fields if needed.
- The SPIM wiring path is "configure early, validate, lock, and operate
  under that locked policy." Per-transaction reroutes are out of scope; see
  `peripherals/spimonitor/planning/overview-and-usage-model.md`.
