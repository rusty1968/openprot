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

### SPIM Wiring Requirements by Controller Type

- **FMC (Flash Memory Controller)**: Does NOT route through SPIPF monitor. Requires
  `spim_wiring == None`. The FMC connects directly to flash without any SPI-monitor
  interception. If you attempt to call `apply_spim_wiring()` on an FMC descriptor,
  it will return `SpimWiringError::InvalidController` because monitoring is not
  supported on FMC paths.

- **SPI1 / SPI2 (Application SPI Controllers)**: Route through SPIPF monitor (SPIM0–3).
  Require `spim_wiring == Some(_)`. The `SpimWiring` struct defines which SPIM instance
  and mux configuration applies. If the wiring source disagrees with the controller
  (e.g., you pass Spi1 with wiring configured for Spi2), `apply_spim_wiring()` returns
  `SpimWiringError::RouteMismatch`.

### MonitorPolicy Equality

`Ast10x0BoardDescriptor` is not `Eq`/`PartialEq` because the embedded `MonitorPolicy`
is not yet comparable (its allowlist and region tables require custom equality logic).
For now, compare individual fields (`controller`, `cs0`, `cs1`, `spim_wiring`, etc.)
if descriptor equality is needed.

### SPIM Wiring & Locking Semantics

The SPIM wiring path follows a strict "configure early, validate, lock, and operate"
model:

1. **Configure**: Set up SCU0F0 routing fields (mux source, passthrough mode, external
   mux select, MISO multi-func pin).
2. **Validate**: Ensure the route is legal and consistent with the flash descriptor.
3. **Lock**: Call `lock_policy()` on the monitor to set the SPIPF write-disable bit.
   Once locked, the policy is immutable until reset.
4. **Operate**: All subsequent SPI transactions must comply with the locked policy.

**Per-transaction reroutes are out of scope.** The design does not support changing the
mux, passthrough mode, or policy between transactions. Such dynamic reconfiguration would
require pausing the monitor, reprogramming registers, and revalidating—significantly
increasing complexity and risk. For details on this design rationale, see
`peripherals/spimonitor/planning/overview-and-usage-model.md`.
