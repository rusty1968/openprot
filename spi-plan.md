# SPI Topology Modeling Plan

## Goal

Model the AST10x0 SPI controller topology that aspeed-rust carries as
`ctrl_type` and `master_idx`, and thread that information into the reference
project in the right layer.

The immediate objective is to support the behavior differences that depend on
controller role without stuffing that behavior into the thin SPI wrapper.

## Problem Statement

Today the reference SMC stack models:

- which controller is being used (`Fmc`, `Spi1`, `Spi2`)
- which chip selects are configured (`cs0`, `cs1`)
- whether DMA and interrupts are enabled

It does **not** model the controller topology that aspeed-rust uses to control
several SPI behaviors:

- `ctrl_type`:
  - `BootSpi`
  - `HostSpi`
  - `NormalSpi`
- `master_idx`:
  - `0` for FMC / SPI1 cases in the current aspeed-rust usage
  - `2` for SPI2 in the current aspeed-rust usage

Those fields are not decorative in aspeed-rust. They influence runtime
behavior in the SPI controller implementation, especially:

1. decode-window sizing and pre-initialization behavior
2. calibration skip rules
3. some controller-specific SPI register programming paths
4. multi-CS behavior under nonzero `master_idx`

The reference project currently documents this as a gap in the board crate, but
it does not carry the data into the peripheral configuration model.

## Non-Goal

This plan is **not** a broad rewrite of the SPI stack.

It should not:

- redesign the public SPI facade from scratch
- move all board logic into peripherals
- duplicate aspeed-rust line-for-line
- mix HostSpi and NormalSpi policy decisions into raw register backends
- make [target/ast10x0/peripherals/smc/spi.rs](target/ast10x0/peripherals/smc/spi.rs)
  responsible for topology behavior

## Current State Summary

### Reference project

- [target/ast10x0/peripherals/smc/types.rs](target/ast10x0/peripherals/smc/types.rs)
  defines `SmcController` and `SmcConfig`
- [target/ast10x0/peripherals/smc/spi.rs](target/ast10x0/peripherals/smc/spi.rs)
  is only a thin lifecycle wrapper around `UninitSpiSmc` and `ReadySpiSmc`
- [target/ast10x0/peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs)
  owns the shared SMC behavior
- [target/ast10x0/board/src/lib.rs](target/ast10x0/board/src/lib.rs)
  already documents that `master_idx / ctrl_type` are not modeled
- [target/ast10x0/board/src/spim_wiring.rs](target/ast10x0/board/src/spim_wiring.rs)
  documents the implied routing role, but does not pass that role into SMC

### aspeed-rust

- [../aspeed-rust/src/spi/types.rs](../aspeed-rust/src/spi/types.rs)
  defines `CtrlType` and `master_idx` in `SpiConfig`
- [../aspeed-rust/src/spi/spicontroller.rs](../aspeed-rust/src/spi/spicontroller.rs)
  uses those fields to control decode-range and calibration behavior
- [../aspeed-rust/src/spi/spitest.rs](../aspeed-rust/src/spi/spitest.rs)
  shows the board-level mappings currently relied on:
  - FMC -> `BootSpi`, `master_idx = 0`
  - SPI1 -> `HostSpi`, `master_idx = 0`
  - SPI2 -> `NormalSpi`, `master_idx = 2`

## Design Principles

1. **Model the concept once**.
   The topology should be carried in configuration data, not inferred from ad
   hoc conditionals spread through the code.

2. **Keep wrappers thin**.
   The SPI facade in [target/ast10x0/peripherals/smc/spi.rs](target/ast10x0/peripherals/smc/spi.rs)
   should remain a construction/lifecycle layer, not a policy engine.

3. **Keep raw register backends dumb**.
   Backends should expose register access. Topology-specific decisions belong in
   controller logic, not the backend itself.

4. **Source board-specific topology from the board crate**.
   The board layer already knows the intended role of each controller preset.
   That is where the semantic mapping should originate.

5. **Port semantics, not syntax**.
   We want behavioral parity with aspeed-rust where it matters, not a mechanical
   copy of its structure.

## Proposed Data Model

Add an explicit topology enum in
[target/ast10x0/peripherals/smc/types.rs](target/ast10x0/peripherals/smc/types.rs).

Example shape:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcTopology {
    BootSpi { master_idx: u8 },
    HostSpi { master_idx: u8 },
    NormalSpi { master_idx: u8 },
}
```

Then extend `SmcConfig`:

```rust
pub struct SmcConfig {
    pub controller_id: SmcController,
    pub cs0: Option<FlashConfig>,
    pub cs1: Option<FlashConfig>,
    pub dma_enabled: bool,
    pub enable_interrupts: bool,
    pub topology: SmcTopology,
}
```

### Why an enum instead of only `host_id` or only `master_idx`

A raw host/master identifier is not enough by itself.

The aspeed-rust logic depends on two dimensions:

- role: `BootSpi`, `HostSpi`, `NormalSpi`
- mux/master identity: `master_idx`

An enum makes the controller role explicit and prevents ambiguous states such as
“FMC with HostSpi behavior” or “SPI2 pretending to be BootSpi” unless a caller
intentionally constructs that topology.

## Ownership and Layering

### Layer responsibilities after the change

#### Board crate

Responsible for declaring board intent:

- which controller this descriptor uses
- what SPIM wiring applies
- what monitor policy applies
- what pinctrl groups apply
- what SPI topology applies

#### SMC config/types

Responsible for carrying that intent as structured configuration into the
peripheral layer.

#### SMC controller

Responsible for acting on the topology:

- decode-window rules
- calibration skip rules
- CS routing restrictions
- any role-dependent control register programming

#### SPI wrapper

Responsible only for constructing and exposing the controller.

#### Register backends

Responsible only for exposing low-level accessors.

## Detailed Implementation Steps

### Phase 1: Add topology to the type system

Files:

- [target/ast10x0/peripherals/smc/types.rs](target/ast10x0/peripherals/smc/types.rs)

Changes:

1. Add `SmcTopology` enum.
2. Add helper methods if useful, for example:
   - `master_idx()`
   - `is_host_spi()`
   - `is_boot_spi()`
   - `is_normal_spi()`
3. Extend `SmcConfig` with a `topology` field.
4. Update all config construction sites so the build stays green.

Exit criteria:

- all `SmcConfig` construction sites compile
- no behavior changes yet

### Phase 2: Source topology from the board crate

Files:

- [target/ast10x0/board/src/lib.rs](target/ast10x0/board/src/lib.rs)

Changes:

1. Update `Ast10x0BoardDescriptor::smc_config()` to populate `topology`.
2. Encode the current intended mappings:
   - FMC descriptors -> `SmcTopology::BootSpi { master_idx: 0 }`
   - SPI1 descriptors -> `SmcTopology::HostSpi { master_idx: 0 }`
   - SPI2 descriptors -> `SmcTopology::NormalSpi { master_idx: 2 }`
3. Keep the existing controller and flash geometry handling unchanged.

Exit criteria:

- all board presets emit explicit topology
- the board layer becomes the single source of truth for controller role

### Phase 3: Consume topology in controller logic

Files:

- [target/ast10x0/peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs)

Changes:

1. Find the reference equivalents of the aspeed-rust behaviors tied to
   `master_idx` and `ctrl_type`.
2. Port only the semantics that matter in the reference implementation.
3. Replace implicit controller-role assumptions with checks against
   `config.topology`.

The first-pass targets are:

- decode-range sizing or pre-init behavior when `master_idx != 0`
- calibration skip logic for nonzero `master_idx` on nonzero chip select
- controller-role gating for SPI-only behavior currently associated with
  HostSpi semantics

Exit criteria:

- controller decisions that depend on topology now use `SmcTopology`
- no topology decisions are hidden in `spi.rs`

### Phase 4: Keep the SPI wrapper thin

Files:

- [target/ast10x0/peripherals/smc/spi.rs](target/ast10x0/peripherals/smc/spi.rs)

Changes:

1. Do **not** add new policy logic here.
2. Only ensure that the richer `SmcConfig` is passed through correctly.
3. If documentation is needed, add a short note that topology behavior is
   handled in the shared controller, not in the wrapper.

Exit criteria:

- wrapper remains a construction API
- no duplicate role logic appears here

### Phase 5: Leave raw backends as register transport

Files:

- [target/ast10x0/peripherals/smc/spi_backend.rs](target/ast10x0/peripherals/smc/spi_backend.rs)
- [target/ast10x0/peripherals/smc/fmc_backend.rs](target/ast10x0/peripherals/smc/fmc_backend.rs)

Changes:

1. Avoid pushing HostSpi vs NormalSpi decisions into backend types.
2. Continue to expose SPI-only accessors such as SPI06C/SPI074 only where they
   belong structurally.
3. Let controller logic decide when to call them.

Exit criteria:

- register backends stay simple
- topology logic remains centralized in the controller layer

### Phase 6: Update documentation and comments

Files:

- [target/ast10x0/board/src/lib.rs](target/ast10x0/board/src/lib.rs)
- [target/ast10x0/board/src/spim_wiring.rs](target/ast10x0/board/src/spim_wiring.rs)
- optionally SMC README/planning docs if needed

Changes:

1. Remove or narrow comments saying `master_idx / ctrl_type` are not modeled.
2. Replace them with a more precise statement of what is now modeled and what is
   still intentionally not carried.
3. Keep the docs aligned with the actual topology mappings.

Exit criteria:

- docs describe current reality instead of a stale gap
- `SmcTopology` enum variant docs include behavioral summary (decode-range, calibration, SPIM bracketing gating per variant)

## Mapping Table

The first-pass mapping should match the current board intent:

| Controller | Board Meaning | Topology | master_idx |
|---|---|---|---|
| FMC | Boot flash path | `BootSpi` | `0` |
| SPI1 | Host/BMC-style SPI path | `HostSpi` | `0` |
| SPI2 | Normal SPI path | `NormalSpi` | `2` |

This table should be treated as code-generation truth for board presets, not
just documentation.

## Behavioral Parity Targets from aspeed-rust

The following semantics are worth porting explicitly.

### 1. Decode-range pre-init behavior

In aspeed-rust, `master_idx != 0` changes decode sizing behavior.

Reference should decide whether the equivalent logic belongs in the existing
window/layout setup. If yes, gate it via `SmcTopology::master_idx()`.

### 2. Calibration skip behavior

In aspeed-rust, nonzero `master_idx` changes whether calibration runs on CS1.

Reference should port this only if the corresponding calibration path exists and
matters to current hardware behavior.

### 3. HostSpi-specific control programming

aspeed-rust treats some SPI06C/SPI074 register programming as HostSpi-specific.

Reference should model that at the controller layer using `matches!(topology,
SmcTopology::HostSpi { .. })` rather than controller-id heuristics.

## Risks

1. **Over-modeling too early**
   If we add a large topology surface before confirming the controller sites
   that need it, the code may become more complicated without immediate value.

2. **Putting logic in the wrong layer**
   Adding the field in `SmcConfig` but then implementing the behavior in
   [target/ast10x0/peripherals/smc/spi.rs](target/ast10x0/peripherals/smc/spi.rs)
   would be the wrong architecture.

3. **Copying aspeed-rust too mechanically**
   Some aspeed-rust conditionals may reflect implementation quirks rather than
   essential design. We should port the intent, not every branch.

4. **Board/peripheral drift**
   If board presets and controller semantics are updated separately, the mapping
   can drift. The board crate must be the single source of truth for topology.

## Testing Strategy

### Narrow validation first

Run these after the first substantive edits:

```bash
cd /home/rusty1968/work/storage/reference
bazelisk build --config=virt_ast10x0 //target/ast10x0/peripherals:peripherals
bazelisk build --config=virt_ast10x0 //target/ast10x0/board:ast10x0_board
```

### Focused behavior checks

If there is an existing SPI or SPIM test surface, add the narrowest possible
coverage around:

- SPI1 host-topology initialization
- SPI2 normal-topology initialization
- any CS1 behavior that depends on nonzero `master_idx`

If no tight unit-test surface exists for the controller logic, use the nearest
AST10x0 integration target rather than forcing `#[cfg(test)]` modules into the
peripheral crate.

## Recommended Implementation Order

1. Update
   [target/ast10x0/peripherals/smc/types.rs](target/ast10x0/peripherals/smc/types.rs)
2. Update
   [target/ast10x0/board/src/lib.rs](target/ast10x0/board/src/lib.rs)
3. Update
   [target/ast10x0/peripherals/smc/controller.rs](target/ast10x0/peripherals/smc/controller.rs)
4. Touch
   [target/ast10x0/peripherals/smc/spi.rs](target/ast10x0/peripherals/smc/spi.rs)
   only if needed to pass through config or adjust docs
5. Update stale comments/docs
6. Run narrow builds
7. Add focused behavior coverage if needed

## Minimal Viable Slice

If we want the smallest useful first change, do only this:

1. Add `SmcTopology` to `SmcConfig`
2. Populate it from board presets
3. Use it in the controller for the first behavior that demonstrably needs
   parity with aspeed-rust
4. Stop and validate

That keeps the first patch small and falsifiable.

## Definition of Done

This work is done when:

1. topology is modeled explicitly in `SmcConfig`
2. board presets supply the intended topology values
3. controller logic consumes topology where behavior actually depends on it
4. the SPI wrapper remains thin
5. stale “not modeled” comments are updated
6. the narrowed Bazel builds pass
7. at least one focused behavior path is validated against the intended role
   differences
