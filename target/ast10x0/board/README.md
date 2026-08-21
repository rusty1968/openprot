# ast10x0_board

Board-level integration crate for AST10x0 platforms.

This crate owns board-level hardware initialization and board-selected SGPIOM
configuration for AST10x0 targets.

## Responsibilities

- Apply board pinctrl groups via SCU
- Gate/reset board-level peripherals needed at boot (currently I2C flow)
- Export board descriptor metadata used by runtime init
- Consume generated SGPIOM configuration selected at board build time

SGPIOM policy is board-owned here, while SGPIOM execution remains in
`//target/ast10x0/peripherals`.

## BMC reset & external-flash bus gate

Source: `src/bmc_reset.rs`.

### Problem

The RoT and the host (BMC/AP) share the host SPI flash bus. The RoT may only
master that bus while the host is held in reset; otherwise both sides could
drive it. The external-flash server must therefore refuse every bus operation
unless the host is currently in reset.

This is enforced by two cooperating roles that touch the *same* two active-low
reset lines but never share a Rust object — in the OpenPRoT microkernel they run
in different processes over shared MMIO:

- **Actuator** — `BmcReset<S, E>`: owns and drives the reset lines. The
  orchestrator asserts reset before handing the bus to the RoT and releases it
  afterwards.
- **Observer** — `BmcResetGate<S, E>`: read-only. `GatedFlash` calls
  `BusAccessGate::ensure_open` before each read/erase/program and refuses the op
  (`FLASH_AST10X0_GATE_CLOSED`) unless both lines read back asserted. `geometry`
  is not gated (static config, no bus access).

### The portability seam

Each reset line is expressed as an `embedded-hal` `StatefulOutputPin`. Both the
actuator and the gate are generic over the two pin types, so board support is
"supply two pins," not "fork the reset logic." This mirrors Zephyr's
`gpio_dt_spec`: the pin is the board-selectable seam.

Active-low mapping and sequencing (encoded once, in `BmcReset`):

- assert  = drive low; `EXTRST` low first, then `SRST`, each with a settle delay
- release = drive high; `SRST` high first, then `EXTRST`
- `asserted` = both lines read back low (`is_set_low`); readback confirms the
  transition rather than assuming it

`StatefulOutputPin` readback reflects the driven output latch, not an input
level — it reports what the RoT is holding.

### Board variants

| Board            | eRoT SoC | SRST        | EXTRST      | Constructor      |
| ---------------- | -------- | ----------- | ----------- | ---------------- |
| prot             | AST1060  | SGPIO A–D 8 | SGPIO A–D 9 | `prot()`         |
| ast1060_dcscm    | AST1060  | GPIO_M5     | GPIO_H2     | `dcscm()`        |
| ast2700_dcscm    | AST1060  | GPIO_M5     | GPIO_H2     | `dcscm()`        |

The reset lines are pins on the **eRoT** SoC (AST1060 = the `ast10x0`/pwkernel
target). The host on the other end (e.g. AST2700) does not change this code, so
`ast2700_dcscm` — whose reference DTS routes the same eRoT `GPIO_M5`/`GPIO_H2` —
is already covered by `dcscm()`.

Not covered: the `ast2700_dcscm` *venice* variant routes a single line
(`EXTRST` on `GPIO_M5`, no `SRST`) and is flagged unfinished in the ASPEED
reference DTS; it needs its own single-line constructor once finalized.

### Two pin backends

- **SGPIO** (`prot`): `SgpioOutputPin` adapts an SGPIO master A–D output-latch
  bit to `StatefulOutputPin`. `set_*` write `gpio500`; `is_set_*` read `gpio570`.
  Construction has no register side effects, so the observer can build one
  without disturbing the line.
- **Native GPIO** (`dcscm`): the peripheral pins already implement
  `StatefulOutputPin`. The gate obtains them via `PXi::steal()` (a no-write
  handle) so observing never re-drives the line; the actuator uses
  `into_push_pull_output()` to configure and drive.

`BusAccessGate::ensure_open` takes `&mut self` because `StatefulOutputPin`
readback requires mutable access to the pin.

## SGPIOM JSON Pipeline (Bazel)

The board package wires SGPIOM JSON tooling into Bazel with these targets:

- `//target/ast10x0/board:sgpiom_validate`
- `//target/ast10x0/board:sgpiom_merged`
- `//target/ast10x0/board:sgpiom_generate`
- `//target/ast10x0/board:sgpiom_check`
- `//target/ast10x0/board:sgpiom_report`

Generated artifact compiled into this crate:

- `sgpiom_config_generated.rs`

See `//tools/sgpiom/README.md` for CLI details and manifest format.

## Build

```
bazelisk build --config=virt_ast10x0 //target/ast10x0/board:ast10x0_board
```

For AST1060 hardware config and SGPIOM pipeline artifacts:

```bash
bazelisk build --config=k_ast1060_evb \
	//target/ast10x0/board:ast10x0_board \
	//target/ast10x0/board:sgpiom_validate \
	//target/ast10x0/board:sgpiom_merged \
	//target/ast10x0/board:sgpiom_generate \
	//target/ast10x0/board:sgpiom_check \
	//target/ast10x0/board:sgpiom_report
```
