# AST10x0 Host Flash Backend — Driver-Level Gaps

Scope: gaps *inside* `Ast10x0SpiHostFlashDriver`
([host.rs](../src/host.rs)) that stand between today's driver and full parity
with the aspeed-zephyr BMC/PCH update path. Higher-layer concerns (image
staging, signature/manifest verification, host-reset and SPI-monitor
sequencing, active/recovery rollback policy) live above this crate and are out
of scope here.

For the overall design, see [host-flash-design.md](host-flash-design.md).

## What the driver already provides

The three primitives an update needs are present and build for
`--config=virt_ast10x0`:

- **Read** — arbitrary length, single-lane.
- **Erase** — 4 KiB sector.
- **Program** — 256 B page (`BlockingFlash` splits larger writes).
- **4-byte addressing** — automatic above 16 MiB, so a 64/128 MiB part is fully
  addressable.
- **Bus routing** — per-operation RoT-internal-master detour with guaranteed
  passthrough restore.

For a **single-chip** BMC/PCH flash, the mechanics of erase → program →
read-back are therefore complete.

## Gap 1 — Dual-flash spanning (blocker for dual-chip layouts)

The reference BMC layout is a 128 MiB image presented as **two** physical SPI
chips spanned into one logical address space (aspeed's `flash_aspeed.c`;
`BMC_FLASH_CONFIG` capacity = 128 MiB). Each `Ast10x0SpiHostFlashDriver`
instance targets **one** chip select, so a single instance cannot address an
image that spills onto a second chip past the first chip's capacity.

Impact: on a dual-chip BMC, addresses beyond the first chip are unreachable —
this is a correctness blocker, not a performance issue.

Possible directions:
- A spanning wrapper that owns two chip selects and routes each
  read/erase/program to the correct chip by offset, splitting operations that
  straddle the boundary.
- Or a higher-level composite `FlashDriver` that stacks two single-chip drivers
  behind one linear address space.

## Gap 2 — Erase granularity (performance)

Only 4 KiB sector erase is exposed (`erasable_sizes_bitmap` reports a single
size). A full-image update erases tens of MiB one sector at a time.

Impact: functional but slow; not a correctness issue.

Possible direction: expose 64 KiB block erase (and optionally chip erase) once
the SMC peripheral layer supports them, and report the additional sizes in
`erasable_sizes_bitmap`.

## Gap 3 — Program throughput (performance)

Reads and programs are single-lane; programs move 256 B per page command.

Impact: correct but slow for a full-image write; not a correctness issue.

Possible direction: dual/quad I/O for read and page program, gated on device
capability probing.

## Summary

| Gap | Kind | Blocks BMC update? |
|-----|------|--------------------|
| Dual-flash spanning | Correctness | Yes, on dual-chip layouts |
| Erase granularity (block/chip) | Performance | No |
| Program throughput (dual/quad I/O) | Performance | No |

Single-chip BMC/PCH updates are achievable with the current driver plus an
external orchestrator. Reaching parity with the aspeed dual-chip BMC layout
requires closing Gap 1.
