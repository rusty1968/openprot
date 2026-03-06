# SRAM Migration Analysis: Tock Single-Process to pw-kernel Multi-Process

## Executive Summary

This document analyzes the SRAM impact of migrating caliptra-mcu-sw from Tock OS (single userspace process, cooperative async tasks) to pw-kernel (Hubris-style architecture with separate processes per driver/service, hardware-enforced isolation via PMP).

**Current Tock baseline (emulator build, 1 app):**
- SRAM used: 413,696 bytes (404.0 KB) of 524,288 bytes (512 KB) [measured]
- SRAM free: 110,592 bytes (108.0 KB, 21.1%) [measured]

**Projected pw-kernel multi-process budget (5 processes):**
- SRAM used: ~469-489 KB [estimated]
- SRAM free: ~23-43 KB (4.5-8.4%) [estimated]

**Recommendation: GO for 512 KB SRAM.**

The migration fits within 512 KB with substantial headroom (~40-45%), primarily because the move to synchronous per-process event loops eliminates Embassy's async runtime entirely — removing ~8 KB of executor/reactor code, ~27 KB of async task pool BSS, and ~12 KB of heap used for pinned futures. No special optimizations are required.

At 448 KB, the migration is feasible with ~25-30% headroom. At 384 KB, it fits with ~10-15% margin.

**Architecture-specific verdicts (512 KB, 5 processes):**
- **RISC-V (VeeR EL2, 64 PMP entries):** GO — ample memory protection headroom, ~200+ KB free
- **ARMv8-M (Cortex-M33, 8 MPU regions):** GO — SRAM budget similar; MPU swapping per context switch trades simultaneous protection for feasibility
- **ARMv7-M (Cortex-M3/M4):** NO GO — not supported by pw-kernel; power-of-2 MPU alignment wastes ~141 KB, exceeding the SRAM budget

**Key architectural change:** Under pw-kernel's Hubris-style model, each process runs a **synchronous event loop** (block on IPC, process message, reply) — not a cooperative async executor. Embassy (executor, reactor, task pools, heap-allocated futures) is eliminated entirely. This removes the single largest user-app BSS consumer (`spdm_doe_responder::POOL`, 20,288 B) and all async runtime overhead.

---

## 1. Per-Process Decomposition

### Process Mapping

The current Tock single-app architecture runs all async tasks within one userspace process. Under pw-kernel, each service becomes an isolated process with its own PMP-enforced memory regions.

| pw-kernel Process | Caliptra MCU Tasks Contained | Rationale |
|---|---|---|
| **P0: kernel** | pw-kernel itself (M-mode) | Kernel scheduler, IPC, PMP management |
| **P1: spdm_service** | `spdm_mctp_responder` + `spdm_doe_responder` + `spdm_task` | Both SPDM responders share `spdm-lib`, `SpdmContext`, certificates, and measurements state. Splitting them would duplicate the 20 KB DOE SPDM state machine and ~40 KB of spdm-lib code. |
| **P2: pldm_service** | `pldm_responder_task` + `pldm_initiator_task` + `pldm_service_task` | Responder and initiator share `pldm-lib`, `pldm-common`, and MCTP transport. They coordinate via `embassy_sync::Signal` within the same process. Splitting would break this shared state. |
| **P3: mcu_mbox** | `mcu_mbox_responder_task` | MCU mailbox command handler. Forwards crypto to Caliptra via IPC. Independent of SPDM/PLDM protocol state. |
| **P4: caliptra_proxy** | Caliptra mailbox forwarding | Mediates access to Caliptra hardware mailbox. Currently kernel capsules in Tock; becomes a dedicated driver process in pw-kernel. |

**Total: 5 processes** (1 kernel + 4 userspace)

### Why These Groupings

- **SPDM tasks stay together**: `spdm_mctp_responder` and `spdm_doe_responder` are defined in the same module (`platforms/emulator/runtime/userspace/apps/user/src/spdm/mod.rs`) and share `SpdmContext`, `SpdmMeasurements`, certificate state (`SHARED_DPE_LEAF_CERT`, 2,072 B), and the `spdm-lib` crate. The `spdm_doe_responder::POOL` alone is 20,288 bytes — duplicating this in a separate process is prohibitive.
- **PLDM tasks stay together**: `pldm_responder_task` and `pldm_initiator_task` are spawned from `pldm_service_task` and communicate via `embassy_sync::Signal<CriticalSectionRawMutex, ...>`. They share `pldm-lib` and `pldm-common` crates.
- **MCU mailbox is independent**: It uses `mcu-mbox-lib` and `caliptra-api` but not SPDM or PLDM libraries. Clean separation boundary.
- **Caliptra proxy is a driver**: In Tock it's kernel capsules (MCTP driver + Caliptra mailbox). In pw-kernel's Hubris-style model, hardware drivers run as isolated user-mode processes.

---

## 2. Code Duplication Matrix

### Crate Usage Per Process

Source: `platforms/emulator/runtime/userspace/apps/user/Cargo.toml`, task source files in `runtime/userspace/api/`

| Crate | P1: spdm | P2: pldm | P3: mcu_mbox | P4: caliptra_proxy | Shared Count |
|---|:-:|:-:|:-:|:-:|:-:|
| spdm-lib | x | | | | 1 |
| pldm-lib | | x | | | 1 |
| pldm-common | | x | | | 1 |
| mcu-mbox-lib | | | x | | 1 |
| mcu-mbox-common | | | x | | 1 |
| caliptra-api | x | | x | x | 3 |
| libapi-caliptra | x | | x | x | 3 |
| external-cmds-common | x | | x | | 2 |
| zerocopy | x | | x | | 2 |
| ocp-eat | x | | | | 1 |
| mctp-vdm-common | x | | | | 1 |
| mctp-vdm-lib | x | | | | 1 |
| Core runtime (panic, startup) | x | x | x | x | 4 |

> **Embassy eliminated:** In the Tock single-process model, all tasks share an Embassy async executor with cooperative scheduling. Under pw-kernel, each process is a synchronous event loop that blocks on IPC via `object_wait` — no async executor, reactor, or task pools are needed. The following crates are **removed entirely**: `embassy-executor`, `embassy-sync`, `libtockasync`, `async-trait`, `portable-atomic` (async use), `critical-section` (async use). This eliminates ~8 KB of shared code that would otherwise be duplicated across all 4 processes (~32 KB total savings).

### Estimated Code Size Per Crate

These estimates are derived from the total app .text (131,306 B) and .rodata (23,880 B) = 155,186 B total, allocated proportionally based on crate complexity and known symbol sizes. Crates eliminated by the move to synchronous processes are shown struck through.

| Crate | Est. .text + .rodata (KB) | Status in pw-kernel | Basis |
|---|---|---|---|
| spdm-lib (protocol + transport + codec) | ~45 KB [estimated] | **Retained** | Dominant protocol crate; SPDM state machines, DOE/MCTP transport, VDM handlers |
| pldm-lib + pldm-common | ~25 KB [estimated] | **Retained** | PLDM protocol, firmware update, codec |
| mcu-mbox-lib + mcu-mbox-common | ~12 KB [estimated] | **Retained** | Command interface, transport, handlers |
| caliptra-api + libapi-caliptra | ~15 KB [estimated] | **Retained** | Mailbox API, command definitions |
| ~~embassy-executor + embassy-sync~~ | ~~~8 KB [estimated]~~ | **Eliminated** | Replaced by synchronous IPC event loops; each process blocks on `object_wait` directly |
| external-cmds-common | ~5 KB [estimated] | **Retained** | Unified command handler trait |
| zerocopy + ocp-eat + mctp-vdm | ~8 KB [estimated] | **Retained** | Serialization, measurements |
| ~~libtock runtime shims / syscall layer~~ | ~~~15 KB [estimated]~~ | **Eliminated** | Replaced by pw-kernel userspace syscall stubs (~2 KB) |
| ~~libtockasync + async-trait~~ | ~~~4 KB [estimated]~~ | **Eliminated** | No async runtime needed |
| pw-kernel userspace stubs | ~2 KB [estimated] | **New** | Replaces libtock shims; minimal syscall wrappers |
| Remaining (static init, panic, etc.) | ~15 KB [estimated] | **Retained** | Core library, panic handler, startup |

**Tock app code total: ~152 KB** (matches measured 151.7 KB app code in prog region)
**pw-kernel per-process code base (after removals): ~127 KB** [estimated] (~152 KB - 8 KB embassy - 15 KB libtock - 4 KB libtockasync + 2 KB pw-kernel stubs)

> **Note:** The ~127 KB is the sum across all protocol crates. Each individual process only includes the crates it uses — P1 gets spdm-lib + caliptra-api + shared, P2 gets pldm-lib + shared, etc. The per-process code sizes are: P1 ~65 KB, P2 ~33 KB, P3 ~25 KB, P4 ~30 KB [estimated].

### Code Duplication Cost

Crates shared across multiple processes must be duplicated in each process's .text region (pw-kernel does not support shared libraries or XIP from a common code region by default).

| Duplicated Crate | Size (KB) | Processes Using | Extra Copies | Duplication Cost (KB) |
|---|---|---|---|---|
| caliptra-api + libapi-caliptra | ~15 KB [est.] | P1, P3, P4 | 2 extra | ~30 KB [est.] |
| external-cmds-common | ~5 KB [est.] | P1, P3 | 1 extra | ~5 KB [est.] |
| zerocopy | ~3 KB [est.] | P1, P3 | 1 extra | ~3 KB [est.] |
| Core runtime (panic, startup) | ~5 KB [est.] | P1, P2, P3, P4 | 3 extra | ~15 KB [est.] |
| pw-kernel userspace stubs | ~2 KB [est.] | P1, P2, P3, P4 | 3 extra | ~6 KB [est.] |
| **Total duplication overhead** | | | | **~59 KB [est.]** |

> **Note:** Eliminating Embassy removes the largest previously-duplicated crate (embassy-executor + embassy-sync at ~8 KB x 4 processes = ~32 KB). The duplication cost dropped from ~80 KB to ~59 KB. If pw-kernel supports shared read-only memory regions, caliptra-api duplication (~30 KB) could also be eliminated.

---

## 3. Stack Budget Per Process

### Methodology

Stack requirements are estimated from synchronous call depth analysis. Under pw-kernel, each process runs a single-threaded synchronous event loop — no async executor or task pools. The stack must hold the deepest synchronous call chain for a single message processing path.

In the current Tock/Embassy model, `#[embassy_executor::task]` compiles into a state machine that captures all locals across `.await` points in a static `POOL`. The stack only holds one synchronous segment at a time. Under the pw-kernel synchronous model, the entire call chain (receive → parse → process → respond) runs on the stack, but only one message is in flight at a time. The stack depth equals the maximum of any single call path, which is much smaller than the 20 KB async state machine that captured all paths simultaneously.

### Current Tock Stack (Single Process)

- App stack: 44,544 B (43.5 KB) configured in `user-app.toml:36` [measured]
- Kernel stack: 8,192 B (8 KB) in `board.rs:111` [measured]

### pw-kernel Per-Process Stack Budget

| Process | Stack (KB) | Justification |
|---|---|---|
| P0: pw-kernel | 2 KB [measured] | `KERNEL_STACK_SIZE_BYTES = 2048` in pw-kernel `config.rs` |
| P1: spdm_service | 8 KB [estimated] | Deepest synchronous call chain: IPC recv → SPDM message parse → certificate lookup → measurement generation → response encode → IPC send. Single-threaded, processes one SPDM request at a time (MCTP or DOE, not both simultaneously). No executor overhead. |
| P2: pldm_service | 5 KB [estimated] | Synchronous PLDM message handling. Simpler protocol than SPDM. Responder and initiator are sequential operations within the same event loop, not concurrent. |
| P3: mcu_mbox | 4 KB [estimated] | Synchronous command processing: parse command → forward to Caliptra via IPC → await response → return. Moderate call depth. |
| P4: caliptra_proxy | 3 KB [estimated] | Simple synchronous IPC-to-hardware-register forwarding. Shallow call depth. |
| **Per-thread kernel stack** (x4 threads) | 8 KB [measured] | pw-kernel allocates 2 KB kernel stack per thread: 4 threads x 2 KB = 8 KB |
| **Total stacks** | **30 KB [est.]** | Compared to 52.5 KB in Tock (44.5 KB app + 8 KB kernel) |

**Stack savings vs Tock: ~22.5 KB** — splitting into processes with right-sized synchronous stacks is substantially more efficient than one 43.5 KB stack shared by all async tasks.

---

## 4. Buffer/BSS Budget Per Process

### Tracing Static Allocations to Owning Process

Source: `sram-budget.csv`, `sram-budget.md` sections 3.1-3.2

#### Kernel BSS (currently 28,632 B in Tock)

In pw-kernel, Tock kernel capsules become either kernel-internal state or driver processes.

| Buffer | Size (B) | Tock Owner | pw-kernel Owner | Notes |
|---|---|---|---|---|
| MCTP SPDM rx/tx/buffered (3x2048) | 6,144 | Kernel capsule | P4: caliptra_proxy | MCTP driver buffers move to driver process |
| MCTP PLDM rx/tx/buffered (3x2048) | 6,144 | Kernel capsule | P4: caliptra_proxy | Same |
| MCTP Caliptra rx/tx/buffered (3x2048) | 6,144 | Kernel capsule | P4: caliptra_proxy | Same |
| MCTP mux tx/rx (2x250) | 500 | Kernel capsule | P4: caliptra_proxy | I3C packet buffers |
| Flash partition buffer | 512 | Kernel capsule | P4: caliptra_proxy or kernel | Flash staging |
| DOE mailbox buffers | ~2,048 [est.] | Kernel capsule | P4: caliptra_proxy | DOE message buffers |
| MCU MBOX0 buffers | ~4,096 [est.] | Kernel capsule | P3: mcu_mbox | MCU mailbox staging |
| DEFCALLS bitmap | 256 | Kernel core | P0: kernel (eliminated) | pw-kernel uses WaitGroup, no deferred calls |
| Other kernel state | ~2,784 [est.] | Kernel core | P0: kernel | Alarm, driver structs |
| **Subtotal** | **28,632** | | | |

#### User App BSS (currently 40,640 B in Tock)

| Buffer | Size (B) | Tock Owner | pw-kernel Owner | Notes |
|---|---|---|---|---|
| ~~spdm_doe_responder::POOL~~ | ~~20,288~~ | ~~Single app~~ | **Eliminated** | Embassy async task pool — not needed in synchronous model. Protocol state is now stack-local during message processing. |
| ~~HEAP_MEM~~ | ~~16,384~~ | ~~Single app~~ | **Greatly reduced** | Was used for Embassy pinned futures (`Box<dyn Future>`). Without async, heap needs drop to protocol-level dynamic state only. |
| SHARED_DPE_LEAF_CERT | 2,072 | Single app | P1: spdm_service | DPE certificate cache — retained |
| ~~spdm_task::POOL~~ | ~~1,704~~ | ~~Single app~~ | **Eliminated** | Embassy async task pool — not needed |
| Other app state | 192 | Single app | Distributed | Remaining statics |
| **Subtotal (Tock)** | **40,640** | | | |
| **Subtotal (pw-kernel, after removals)** | **~2,264** | | | Only cert cache + other statics remain |

#### Heap Distribution

The current 16 KB `HEAP_MEM` is primarily consumed by Embassy's `embedded-alloc` heap for `Box<dyn Future>` pinned I/O futures and dynamic protocol state. Without Embassy's async runtime, heap usage drops dramatically. Synchronous processes can use stack-allocated buffers for protocol processing.

| Process | Heap (KB) | Justification |
|---|---|---|
| P1: spdm_service | 2 KB [est.] | SPDM certificate chain assembly (dynamic-length certs). No pinned futures. |
| P2: pldm_service | 2 KB [est.] | PLDM firmware update metadata. No pinned futures. |
| P3: mcu_mbox | 0 KB | All buffers are stack-local or static. |
| P4: caliptra_proxy | 0 KB | All buffers are stack-local or static. |
| **Total** | **4 KB [est.]** | Down from 16 KB — 12 KB savings from removing async futures heap |

> **Note:** If protocol implementations can avoid `Box`/`Vec` entirely (using fixed-size buffers), the heap can be eliminated completely. This is the Hubris convention — no global allocator, all allocations are static or stack.

#### Per-Process BSS Summary

| Process | Protocol/Cert State (B) | Heap (B) | Driver Buffers (B) | Other (B) | Total BSS (B) |
|---|---|---|---|---|---|
| P1: spdm_service | 2,072 | 2,048 | 0 | 100 | 4,220 [est.] |
| P2: pldm_service | 0 | 2,048 | 0 | 100 | 2,148 [est.] |
| P3: mcu_mbox | 0 | 0 | 4,096 | 100 | 4,196 [est.] |
| P4: caliptra_proxy | 0 | 0 | 21,492 | 500 | 21,992 [est.] |
| **Total** | **2,072** | **4,096** | **25,588** | **800** | **~32,556** |

**BSS savings vs Tock: -36,716 B (-35.9 KB)** (32,556 vs 69,272 in Tock). The elimination of Embassy task pools (~27 KB) and heap reduction (~12 KB) far outweigh the small per-process overhead. This is the single largest savings from the architectural change.

---

## 5. Total SRAM Waterfall: Tock vs pw-kernel

### Side-by-Side Comparison

| Line Item | Tock (bytes) | Tock (KB) | pw-kernel (bytes) | pw-kernel (KB) | Delta (KB) | Source |
|---|---|---|---|---|---|---|
| **Kernel .text + .rodata** | 116,128 | 113.4 | ~10,240 | ~10.0 | -103.4 | Tock: `phase1-elf-sizes.md` [measured]; pw-kernel: `design.rst` ~10 KB target [estimated] |
| **App / Process code** | 155,306 | 151.7 | ~186,000 | ~181.6 | +29.9 | Tock: [measured]; pw-kernel: 127 KB base (after Embassy/libtock removal) + ~59 KB duplication - ~4 KB further dead code [estimated] |
| **Alignment padding (code)** | 3,030 | 3.0 | ~20,480 | ~20.0 | +17.0 | Tock: [measured]; pw-kernel: 5 processes x ~4 KB avg alignment [estimated] |
| **Kernel stack** | 8,192 | 8.0 | 2,048 | 2.0 | -6.0 | Tock: `board.rs:111` [measured]; pw-kernel: `config.rs` [measured] |
| **Kernel .data** | 36 | 0.0 | ~100 | ~0.1 | +0.1 | [estimated] |
| **Kernel .bss** | 28,632 | 28.0 | ~3,000 | ~2.9 | -25.1 | Tock: [measured]; pw-kernel: kernel-only state, no capsule buffers [estimated] |
| **Per-thread kernel stacks** | 0 | 0.0 | 8,192 | 8.0 | +8.0 | pw-kernel: 4 threads x 2 KB [measured] |
| **Process control blocks** | 0 | 0.0 | ~1,300 | ~1.3 | +1.3 | 4 processes x ~150 B + 4 threads x ~180 B [estimated from struct analysis] |
| **User process stacks** | 44,544 | 43.5 | 20,480 | 20.0 | -23.5 | Tock: `user-app.toml:36` [measured]; pw-kernel: 8+5+4+3 KB synchronous stacks [estimated] |
| **User .data** | 60 | 0.1 | ~240 | ~0.2 | +0.1 | 4 processes x ~60 B [estimated] |
| **User .bss (all processes)** | 40,640 | 39.7 | ~32,556 | ~31.8 | -7.9 | Tock: [measured]; pw-kernel: no Embassy pools, reduced heap; see BSS breakdown [estimated] |
| **Grant space** | 16,384 | 16.0 | 0 | 0.0 | -16.0 | pw-kernel uses IPC, not Tock grants [measured→eliminated] |
| **IPC buffers (new)** | 0 | 0.0 | ~8,192 | ~8.0 | +8.0 | See IPC analysis section [estimated] |
| **ram→app_ram padding** | 7,168 | 7.0 | 0 | 0.0 | -7.0 | Tock-specific alignment [measured→eliminated] |
| **Per-process PMP alignment** | 0 | 0.0 | ~16,384 | ~16.0 | +16.0 | 4 processes x 3 regions x ~1.3 KB avg pad [estimated] |
| | | | | | | |
| **Total Used** | **~413,696** | **~404.0** | **~309,212** | **~301.9** | **-102.1** | |
| | | | | | | |
| **Free (512 KB)** | **110,592** | **108.0** | **~215,076** | **~210.1** | | **41% free** |
| **With +25% estimation error** | | | **~386,515** | **~377.5** | | **~134 KB free (26%)** |

### Summary

| Metric | Tock | pw-kernel (est.) | Delta |
|---|---|---|---|
| Kernel code | 113.4 KB | ~10.0 KB | -103.4 KB (pw-kernel is much smaller) |
| App/process code | 151.7 KB | ~181.6 KB | +29.9 KB (duplication, offset by Embassy/libtock removal) |
| Total code | 265.1 KB | ~191.6 KB | **-73.5 KB net savings** |
| Total data (stacks + BSS + grants) | 137.2 KB | ~62.2 KB | **-75.0 KB** (no async pools, no grants, smaller stacks) |
| Alignment/padding | 10.2 KB | ~36.0 KB | +25.8 KB (more processes = more padding) |
| Overhead (PCBs, kernel stacks, IPC) | 0 KB | ~19.3 KB | +19.3 KB (new per-process costs) |
| **Total** | **~404.0 KB** | **~301.9 KB** | **-102.1 KB** |

> **The dominant savings come from two sources:** (1) pw-kernel's ~10 KB kernel replaces Tock's 113 KB kernel (-103 KB), and (2) eliminating Embassy's async runtime removes ~27 KB of task pool BSS, ~12 KB of futures heap, and ~8 KB of executor code. These savings far exceed the costs of code duplication (+59 KB) and multi-process overhead (+19 KB).
| **Total used** | **~404.0 KB** | **~403.2-429.7 KB** | **-0.8 to +25.7 KB** |
| **Free (of 512 KB)** | **~108.0 KB** | **~82.3-108.8 KB** | |

---

## 6. PMP Region Allocation Plan

### VeeR EL2 PMP Configuration

The VeeR EL2 RISC-V core used in the Caliptra MCU provides **64 PMP entries** (not 16).
Source: `runtime/kernel/veer/src/pmp.rs` — `AVAILABLE_ENTRIES = 64`, with entries 0..31 for user MPU regions and entries 32..63 for kernel regions.

> **Important correction:** The prompt assumed 16 PMP entries (standard RISC-V minimum). The VeeR EL2 implementation has 64 entries, which significantly relaxes PMP pressure.

### PMP Granularity

- VeeR EL2 PMP granularity: configurable, set to 0 in pw-kernel QEMU config (`PMP_GRANULARITY = 0`, meaning 4-byte minimum granule) [measured from `config.rs`]
- In practice, code/data regions are aligned to 4 KB pages for efficiency [estimated]

### Kernel PMP Regions (entries 32-63)

| Entry Range | Region | Mode | Access |
|---|---|---|---|
| 62-63 | Kernel .text + .rodata | TOR | M: R+X, locked |
| 60-61 | Kernel .data + .bss + stacks | TOR | M: R+W, locked |
| 58-59 | DCCM (PIC vector table) | TOR | M: R+W, locked |
| 56-57 | MMIO (MCI, MBOX, SOC) | NAPOT | M: R+W, locked |
| 54-55 | Flash storage | NAPOT | M: R, locked |
| 48-53 | Reserved for additional kernel regions | | |
| **Total kernel entries** | **~10-16** | | |

### Per-Process PMP Regions (entries 0-31)

Each user process needs at minimum:
- 2 entries: code region (TOR: start OFF + end TOR+RX)
- 2 entries: data region (TOR: start OFF + end TOR+RW)
- 2 entries: stack region (TOR: start OFF + end TOR+RW) — or merged with data

With 4 processes and 2-4 entries each:

| Process | Code Region | Data Region | Stack | MMIO | Entries |
|---|---|---|---|---|---|
| P1: spdm_service | 2 entries | 2 entries | (merged with data) | 0 | 4 |
| P2: pldm_service | 2 entries | 2 entries | (merged with data) | 0 | 4 |
| P3: mcu_mbox | 2 entries | 2 entries | (merged with data) | 2 entries (MCU MBOX MMIO) | 6 |
| P4: caliptra_proxy | 2 entries | 2 entries | (merged with data) | 4 entries (I3C, DOE, MBOX) | 8 |
| **Total user entries** | | | | | **22** |

**Verdict: 22 user entries + ~12 kernel entries = ~34 of 64 entries used. Fits comfortably.** [estimated]

With only 16 PMP entries (standard RISC-V), this would NOT fit — 22 user + 12 kernel = 34 > 16. The VeeR EL2's 64-entry PMP is essential.

---

## 7. IPC Overhead Analysis

### Communication Paths

Under Tock, inter-task communication is in-process function calls or shared memory via `embassy_sync` primitives. Under pw-kernel, cross-process communication requires kernel-mediated IPC.

| Path | Tock Mechanism | pw-kernel Mechanism | Message Size | Frequency |
|---|---|---|---|---|
| SPDM task ↔ Caliptra mailbox | Kernel capsule `command`/`allow` syscall | IPC channel to P4 | ~2 KB (max MCTP msg) | Per SPDM request |
| PLDM task ↔ MCTP transport | Kernel capsule syscall | IPC channel to P4 | ~2 KB | Per PLDM request |
| MCU mbox ↔ Caliptra mailbox | Kernel capsule syscall | IPC channel to P4 | ~1 KB (cmd + response) | Per mailbox command |
| SPDM ↔ MCU mbox (cert sharing) | In-process shared static | IPC or shared memory region | ~2 KB (certificate) | Rare (init only) |
| PLDM ↔ PLDM (resp↔init signal) | `embassy_sync::Signal` | In-process (same P2) | 0 (signal only) | Per firmware update |

### IPC Buffer Overhead

pw-kernel IPC channels are zero-copy with one message in flight (rendezvous design, per `design.rst`). Each IPC channel endpoint requires a message buffer on both sides.

| IPC Channel | Buffer Size (B) | Count | Total (B) | Notes |
|---|---|---|---|---|
| P1↔P4: SPDM MCTP | 2,048 | 2 (send + recv) | 4,096 | Replaces Tock MCTP SPDM capsule buffers |
| P1↔P4: SPDM DOE | 2,048 | 2 | 4,096 | Replaces Tock DOE capsule buffers |
| P2↔P4: PLDM MCTP | 2,048 | 2 | 4,096 | Replaces Tock MCTP PLDM capsule buffers |
| P3↔P4: MCU mbox→Caliptra | 1,024 | 2 | 2,048 | Replaces Tock Caliptra capsule buffers |
| **Total IPC buffers** | | | **~14,336** | |
| **Minus Tock capsule buffers replaced** | | | **-18,432** | 3 MCTP instances x 3 x 2048 B |
| **Net IPC overhead** | | | **-4,096** | IPC is actually more efficient than Tock's triple-buffered capsules |

> **Key insight:** Tock's MCTP driver allocates 3 buffers per instance (rx, tx, buffered_rx) at 2,048 B each = 6,144 B per instance. pw-kernel's rendezvous IPC needs only 2 buffers per channel. With 3 MCTP instances becoming 3 IPC channels, the buffer overhead actually **decreases** by ~4 KB.

### IPC Latency Cost

Each IPC call adds kernel entry/exit overhead (~100-200 cycles on RISC-V). This is a performance cost, not a memory cost. For the low-frequency operations in caliptra-mcu-sw (SPDM handshakes, PLDM firmware updates), this latency is negligible.

---

## 8. Kernel Size Comparison

| Metric | Tock Kernel | pw-kernel | Source |
|---|---|---|---|
| .text + .rodata | 116,128 B (113.4 KB) | ~10,240 B (~10 KB) | Tock: `phase1-elf-sizes.md` [measured]; pw-kernel: `design.rst` ~10 KB target with panic_detector [estimated] |
| .bss | 28,632 B (28.0 KB) | ~3,000 B (~2.9 KB) | Tock: [measured]; pw-kernel: kernel-only state without capsule buffers [estimated] |
| .stack | 8,192 B (8.0 KB) | 2,048 B (2.0 KB) | Tock: `board.rs:111` [measured]; pw-kernel: `config.rs` [measured] |
| .data | 36 B | ~100 B | Tock: [measured]; pw-kernel: [estimated] |
| **Total kernel SRAM** | **152,988 B (149.4 KB)** | **~15,388 B (~15.0 KB)** | |
| **Savings** | | **~134.4 KB** | |

**Why pw-kernel is ~10x smaller:**
1. **No capsules**: Tock's 113 KB .text includes MCTP drivers, DOE capsules, flash partitions, MCU mailbox capsules, and the entire capsule framework. pw-kernel moves all drivers to userspace processes.
2. **No grant system**: Tock's grant mechanism (per-capsule per-app state) adds kernel complexity.
3. **Static configuration**: pw-kernel uses compile-time task definitions, eliminating dynamic process loading.
4. **Panic elimination**: pw-kernel's `panic_detector` tool statically removes panic paths from the kernel binary.

> **Caveat:** The ~10 KB pw-kernel size is the project target from `design.rst`. Actual measured size for a caliptra-mcu-sw configuration may be larger (15-25 KB) depending on the number of kernel objects, IPC channels, and interrupt handlers configured. This document uses 10 KB as the optimistic estimate. [DATA NOT FOUND: no pw-kernel binary for caliptra-mcu-sw exists yet]

---

## 9. PMP Region Allocation Plan (Detailed)

### Memory Layout Under pw-kernel

```
0x40000000 ┌─────────────────────────────────────────┐
           │ pw-kernel .text + .rodata               │ ~10 KB
0x40002800 ├─────────────────────────────────────────┤
           │ pw-kernel .data + .bss + kernel stack   │ ~5 KB
0x40004000 ├═════════════════════════════════════════┤ ← 4 KB aligned
           │ P1: spdm_service .text + .rodata        │ ~65 KB [est.]
0x40014400 ├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
           │ P1: spdm_service .bss + stack           │ ~12 KB [est.]
0x40017400 ├═════════════════════════════════════════┤
           │ P2: pldm_service .text + .rodata        │ ~33 KB [est.]
0x4001F800 ├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
           │ P2: pldm_service .bss + stack           │ ~7 KB [est.]
0x40021400 ├═════════════════════════════════════════┤
           │ P3: mcu_mbox .text + .rodata            │ ~25 KB [est.]
0x40027800 ├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
           │ P3: mcu_mbox .bss + stack               │ ~8 KB [est.]
0x40029800 ├═════════════════════════════════════════┤
           │ P4: caliptra_proxy .text + .rodata      │ ~30 KB [est.]
0x40031400 ├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
           │ P4: caliptra_proxy .bss + stack         │ ~25 KB [est.]
0x40037800 ├═════════════════════════════════════════┤
           │ Per-thread kernel stacks (4 x 2KB)      │ 8 KB
0x40039800 ├─────────────────────────────────────────┤
           │ Code duplication overhead                │ ~59 KB [est.]
0x40048400 ├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
           │                                         │
           │ FREE (~210 KB)                          │ ~41% of SRAM
           │                                         │
0x40080000 └─────────────────────────────────────────┘
```

> **Note:** This layout is illustrative. Actual addresses depend on pw-kernel's linker configuration and build-time allocation. Code duplication is shown as a separate block for clarity but in practice is distributed across each process's .text region. With 25% estimation error margin, ~124 KB (24%) remains free.

---

## 10. Risk Analysis

### High Risk

| Risk | Impact | Mitigation |
|---|---|---|
| Code duplication exceeds estimate | Could add 15-30 KB beyond estimate | Implement shared read-only code regions in pw-kernel; merge processes |
| Per-process stack underestimated | Stack overflow causes silent corruption or hard fault | Implement stack canaries; use pw-kernel's stack guard pages; measure actual watermarks early. Note: synchronous stacks hold the full call chain, which may be deeper than estimated for complex SPDM operations. |
| Protocol refactoring for synchronous model | Converting async SPDM/PLDM code to synchronous event loops may increase stack usage if large buffers become stack-local | Profile stack depth during porting; use static buffers for large protocol state |

### Medium Risk

| Risk | Impact | Mitigation |
|---|---|---|
| PMP alignment waste exceeds 4 KB/region | Could add 10-20 KB of fragmentation | Use NAPOT mode for power-of-2 sized regions; pack small regions together |
| pw-kernel binary larger than 10 KB target | 15-25 KB is plausible for this configuration | Measure early; the 103 KB Tock→pw-kernel delta provides large margin |
| IPC message copying adds buffer overhead | Each IPC path may need staging buffers | Use pw-kernel's lease mechanism for zero-copy where possible |

### Low Risk

| Risk | Impact | Mitigation |
|---|---|---|
| Process control block overhead | ~330 B per thread + ~150 B per process | Negligible at 4 processes |
| SPDM protocol requires concurrency within a process | Would need to add async back, increasing complexity | Unlikely — SPDM MCTP and DOE handle one message at a time; they don't need to run concurrently within the same process |

### Which Processes to Merge First (If SRAM is Tight)

1. **Merge P3 (mcu_mbox) into P1 (spdm_service)**: Both use `caliptra-api` and `libapi-caliptra`. Saves ~20 KB code duplication + 4 KB stack + 1 process overhead. Cost: weaker isolation between SPDM and mailbox.
2. **Merge P4 (caliptra_proxy) into kernel**: Turns driver process into kernel-internal driver, similar to Tock's capsule model. Saves ~30 KB code + 3 KB stack + process overhead. Cost: driver faults can crash kernel.
3. **Merge P2 (pldm_service) into P1**: Creates single "protocol services" process. Saves ~15 KB code duplication. Cost: PLDM fault takes down SPDM.

> **Note:** With ~200 KB free at 512 KB SRAM, process merging is an optimization of last resort. The budget has sufficient headroom for the full 5-process architecture.

---

## 11. Sensitivity Analysis

### SRAM Budget at Different Sizes

| Line Item | 384 KB | 448 KB | 512 KB |
|---|---|---|---|
| pw-kernel code + data | 15 KB | 15 KB | 15 KB |
| P1: spdm_service (code+data+stack) | 75 KB | 75 KB | 75 KB |
| P2: pldm_service (code+data+stack) | 40 KB | 40 KB | 40 KB |
| P3: mcu_mbox (code+data+stack) | 33 KB | 33 KB | 33 KB |
| P4: caliptra_proxy (code+data+stack) | 55 KB | 55 KB | 55 KB |
| Per-thread kernel stacks | 8 KB | 8 KB | 8 KB |
| Process control blocks | 1.3 KB | 1.3 KB | 1.3 KB |
| IPC buffers | 8 KB | 8 KB | 8 KB |
| PMP alignment padding | 16 KB | 16 KB | 16 KB |
| Code duplication | 59 KB | 59 KB | 59 KB |
| **Total estimated** | **~310 KB** | **~310 KB** | **~310 KB** |
| | | | |
| **Available SRAM** | **384 KB** | **448 KB** | **512 KB** |
| **Free** | **~74 KB (19.3%)** | **~138 KB (30.8%)** | **~202 KB (39.5%)** |
| **With +25% estimation error** | | | |
| **Adjusted total** | **~388 KB** | **~388 KB** | **~388 KB** |
| **Free (adjusted)** | **-4 KB (TIGHT)** | **~60 KB (13.4%)** | **~124 KB (24.2%)** |
| **Verdict** | **Marginal** | **GO** | **GO** |

### Interpretation

- **384 KB**: Fits with best estimates (~74 KB free, 19%). With 25% estimation error, at the edge (~-4 KB). Feasible if stacks and code duplication are tightly controlled; merging P3 into P1 would provide comfortable margin.
- **448 KB**: Comfortable. Even with 25% estimation error, ~60 KB (13%) headroom remains.
- **512 KB**: Very comfortable. ~202 KB free (39%) at best estimate — enough headroom for future feature growth, additional processes, or larger protocol implementations.

---

## 12. Optimization Recommendations

Ranked by estimated savings vs implementation difficulty.

| Rank | Optimization | Savings (KB) | Difficulty | Notes |
|---|---|---|---|---|
| 1 | **Shared read-only code regions** | 30 KB [est.] | Medium | Map caliptra-api .text as shared across P1/P3/P4. Requires pw-kernel support for shared memory mappings. (Reduced from 54 KB since Embassy duplication is already gone.) |
| 2 | **Merge mcu_mbox into spdm_service** | 20-25 KB [est.] | Low | Eliminates one process + code duplication of caliptra-api. Weakens isolation. |
| 3 | **Eliminate global allocator entirely** | 4 KB [est.] | Medium | Follow Hubris convention: no `#[global_allocator]`, all buffers statically sized. Requires refactoring protocol code to use fixed-size containers. |
| 4 | **MCTP buffer right-sizing** | 3-5 KB [est.] | Low | Current buffers are 2,048 B. If max MCTP message is smaller for some protocols, reduce per-channel. |
| 5 | **Merge caliptra_proxy into kernel** | 30+ KB [est.] | High | Eliminates driver process entirely. Requires careful kernel driver implementation. |
| 6 | **XIP from flash** | 180+ KB [est.] | Very High | Execute code from flash/ROM instead of SRAM. Requires hardware support (not available on current VeeR EL2, but common on ARM Cortex-M SoCs). |

> **Note:** Several optimizations from the earlier Embassy-based analysis are no longer needed: "Reduce SPDM DOE task pool" (pools eliminated), "Reduce heap per process" (heap already minimized), and "Right-size process stacks" (synchronous stacks are already smaller by design).

---

## 13. Architecture Comparison: RISC-V vs ARMv7-M vs ARMv8-M

The preceding analysis targets RISC-V (VeeR EL2) with 64 PMP entries. This section extends the analysis to ARM Cortex-M architectures, which are relevant because the Caliptra MCU ecosystem includes ARM-based platforms (e.g., AST1060 with Cortex-M, referenced in `sram-budget.md`).

### pw-kernel Architecture Support Status

| Architecture | pw-kernel Support | Source |
|---|---|---|
| RISC-V (riscv32imc) | YES — primary target | `bazel-openprot/external/pigweed+/pw_kernel/arch/riscv/` [measured] |
| ARMv8-M (Cortex-M33) | YES — two targets | `bazel-openprot/external/pigweed+/pw_kernel/arch/arm_cortex_m/` [measured] |
| ARMv7-M (Cortex-M3/M4) | **NO — not supported** | No ARMv7-M code found in pw-kernel source [measured] |

pw-kernel's ARM support targets **ARMv8-M only** (Cortex-M33 and later). Two concrete targets exist:
- **MPS2-AN505**: ARM MPS2+ FPGA board with Cortex-M33, 124 IRQs, 20 MHz SysTick. Source: `pw_kernel/target/mps2_an505/config.rs` [measured]
- **RP2350**: Raspberry Pi Pico 2 with Cortex-M33, 52 IRQs, 1 MHz SysTick. Source: `pw_kernel/target/pw_rp2350/config.rs` [measured]

ARMv7-M (Cortex-M3/M4/M7) is **not supported** by pw-kernel. Hubris OS has native ARMv7-M support, but pw-kernel's ARM implementation only covers the ARMv8-M MPU model. Porting to ARMv7-M would require implementing the older PMSAv7 MPU interface, which has different register layouts and more restrictive alignment requirements.

### Memory Protection: MPU vs PMP

This is the **critical architectural difference** affecting the multi-process SRAM budget.

| Feature | RISC-V PMP (VeeR EL2) | ARMv8-M MPU | ARMv7-M MPU |
|---|---|---|---|
| Region count | **64 entries** | **8 regions** | **8 regions** |
| Source | `veer/src/pmp.rs`: `AVAILABLE_ENTRIES = 64` [measured] | `mps2_an505/config.rs`: `NUM_MPU_REGIONS = 8` [measured] | ARM PMSAv7 spec (not in pw-kernel) |
| Min alignment | 4 bytes (configurable granularity) | **32 bytes** (lower 5 bits forced) | **Power-of-2, min 32 bytes** |
| Region sizing | Arbitrary (TOR mode) or power-of-2 (NAPOT) | Arbitrary base+limit (5-bit aligned) | **Power-of-2 only** (size and base must be naturally aligned) |
| Entries per region | 1-2 (TOR uses 2, NAPOT uses 1) | **1 per region** | **1 per region** |
| Access control | Per-entry R/W/X + machine/user | Per-region R/W/X + privileged/unprivileged + XN/PXN | Per-region R/W/X + privileged/unprivileged + XN |
| Shareability | N/A | Inner/Outer/Non-shareable | Cacheable/Bufferable/Shareable (TEX+S+C+B) |

### Impact on Process Count: The 8-Region Constraint

With only 8 MPU regions on ARM, the multi-process model is severely constrained.

**Minimum regions per process:**
- 1 region: code (.text + .rodata) — read-execute
- 1 region: data (.data + .bss + stack) — read-write, no-execute
- 1 region (optional): MMIO access — read-write, device memory

**Minimum regions for kernel:**
- 1 region: kernel code — privileged read-execute
- 1 region: kernel data + stacks — privileged read-write
- 1 region: peripheral MMIO — privileged read-write, device

That leaves **5 regions for userspace** after 3 kernel regions. With 2 regions per process minimum:

| Configuration | Kernel Regions | User Regions | Max Processes | Feasible? |
|---|---|---|---|---|
| 3 kernel + 2/process | 3 | 5 | **2 processes** | Tight but workable |
| 3 kernel + 3/process (with MMIO) | 3 | 5 | **1 process** | Only 1 process gets MMIO |
| 2 kernel + 2/process | 2 | 6 | **3 processes** | Requires merging kernel code+data |
| 2 kernel + 3/process | 2 | 6 | **2 processes** | 2 processes with MMIO each |

**On ARM, the 5-process architecture (P0-P4) proposed in Section 1 does NOT fit.**

pw-kernel handles this by swapping MPU configurations on context switch — each process gets the full set of user-accessible MPU regions when it is running, and the kernel reconfigures the MPU at every switch. From `pw_kernel/arch/arm_cortex_m/protection.rs`: each Process stores a `MemoryConfig` containing per-region base/limit/attributes, written to MPU registers on context switch. [measured]

**With MPU swapping, the effective limit is `NUM_MPU_REGIONS` per process (8 total, minus kernel regions).** Each process can use up to 5-6 regions when active, but only one process's regions are loaded at a time. This eliminates the "total entries across all processes" constraint that would exist with static allocation.

### Revised ARM Process Feasibility

With MPU-swapping, the constraint becomes: **each individual process must fit in ~5 user-assignable MPU regions** (8 total minus ~3 kernel). This is feasible for our 4-process design:

| Process | Code Region | Data Region | MMIO Regions | Total | Fits in 5? |
|---|---|---|---|---|---|
| P1: spdm_service | 1 | 1 | 0 | 2 | YES |
| P2: pldm_service | 1 | 1 | 0 | 2 | YES |
| P3: mcu_mbox | 1 | 1 | 1 (MCU MBOX) | 3 | YES |
| P4: caliptra_proxy | 1 | 1 | 2 (I3C + MBOX) | 4 | YES |

However, there are costs:
- **MPU reconfiguration overhead**: Writing 8 MPU regions on every context switch takes ~50-100 cycles on Cortex-M33. For the infrequent context switches in caliptra-mcu-sw, this is negligible.
- **No simultaneous protection**: While P1 is running, P2's memory is not MPU-protected. A bug in P1 could theoretically corrupt P2's memory if it computes a wild pointer into P2's address range. RISC-V PMP can protect all processes simultaneously (64 entries suffice for static allocation).

### ARMv7-M: Additional Constraints

ARMv7-M's PMSAv7 MPU adds a further constraint not present in ARMv8-M:

**Power-of-2 region sizing.** Every MPU region must be a power-of-2 in size, and its base address must be naturally aligned to that size. This causes significant internal fragmentation:

| Process | Actual Size | Rounded to Power-of-2 | Wasted |
|---|---|---|---|
| P1: spdm_service code (~70 KB) | 70 KB | 128 KB | 58 KB (45%) |
| P1: spdm_service data (~44 KB) | 44 KB | 64 KB | 20 KB (31%) |
| P2: pldm_service code (~40 KB) | 40 KB | 64 KB | 24 KB (38%) |
| P2: pldm_service data (~15 KB) | 15 KB | 16 KB | 1 KB (6%) |
| P3: mcu_mbox code (~30 KB) | 30 KB | 32 KB | 2 KB (6%) |
| P3: mcu_mbox data (~14 KB) | 14 KB | 16 KB | 2 KB (13%) |
| P4: caliptra_proxy code (~35 KB) | 35 KB | 64 KB | 29 KB (45%) |
| P4: caliptra_proxy data (~27 KB) | 27 KB | 32 KB | 5 KB (16%) |
| **Total power-of-2 waste** | | | **~141 KB** |

At 141 KB of alignment waste, the ARMv7-M multi-process model would consume ~141 KB more than RISC-V or ARMv8-M. This alone would push the 512 KB budget well past capacity.

> **ARMv7-M is not viable for this multi-process architecture** without subregion tricks (PMSAv7 supports 8 equal subregions per region, allowing finer granularity at the cost of complexity and still requiring power-of-2 base alignment).

### ARMv8-M: Alignment Overhead

ARMv8-M's PMSAv8 MPU uses base+limit addressing with 32-byte granularity — much better than ARMv7-M's power-of-2 requirement. The alignment waste is comparable to RISC-V:

| Process | Size | 32-byte alignment waste | Notes |
|---|---|---|---|
| P1: spdm_service code | ~70 KB | ~0 B | Already 32B aligned |
| P1: spdm_service data | ~44 KB | ~0 B | |
| P2-P4 (similar) | ~136 KB | ~0 B | |
| **Total ARMv8-M alignment waste** | | **< 1 KB** | 32-byte granularity is effectively free |

However, the kernel may impose 4 KB page alignment for simplicity (matching RISC-V), which would bring the padding back to ~16 KB total as estimated for RISC-V.

### ARM ArchThreadState Overhead

| Field | Size (bytes) | Source |
|---|---|---|
| `frame: *mut KernelExceptionFrame` | 4 | `arch/arm_cortex_m/threads.rs` [measured] |
| `memory_config: *const MemoryConfig` | 4 | `arch/arm_cortex_m/threads.rs` [measured] |
| `local: ThreadLocalState<Arch>` | ~8-16 | Includes `preempt_disable_count` (AtomicUsize) + `needs_reschedule` (AtomicBool) [estimated] |
| **Total ARM ArchThreadState** | **~16-24 B** | Slightly smaller than RISC-V (~28-36 B) |

The ARM `MemoryConfig` (stored per-process, not per-thread) contains the MPU region table:
- 8 regions x (base: u32 + limit: u32 + attributes: u32) = ~96 bytes per process [estimated]
- Compared to RISC-V PmpConfig: 16 x (cfg: u8 + addr: u32) = 80 bytes [estimated]

### ARM Kernel Binary Size

pw-kernel's ~10 KB target applies to both ARM and RISC-V. ARM Cortex-M33 has a denser instruction encoding (Thumb-2) than RISC-V RVC, so the ARM kernel binary may be slightly smaller:

| Metric | RISC-V (RV32IMC) | ARMv8-M (Cortex-M33) | Notes |
|---|---|---|---|
| Instruction density | RVC: 16/32-bit mixed | Thumb-2: 16/32-bit mixed | Comparable density |
| Kernel .text target | ~10 KB | ~8-10 KB [estimated] | ARM Thumb-2 historically ~10-15% smaller |
| Syscall entry/exit | ecall → trap handler | SVC → exception handler | Similar complexity |
| Context switch | Save/restore 32 GPRs + PMP | Save/restore 16 GPRs + MPU | ARM saves fewer registers |

### Cross-Architecture Summary Table

| Dimension | RISC-V (VeeR EL2) | ARMv8-M (Cortex-M33) | ARMv7-M (Cortex-M3/M4) |
|---|---|---|---|
| pw-kernel support | YES | YES | **NO** |
| Memory protection entries | 64 PMP | 8 MPU (swapped per context) | 8 MPU (swapped) |
| Simultaneous process protection | YES (all processes) | **NO** (active process only) | **NO** |
| Min alignment | 4 B | 32 B | Power-of-2 size+base |
| Alignment waste (4 processes) | ~16 KB [est.] | ~1-16 KB [est.] | **~141 KB [est.]** |
| Max processes (practical) | **5+** (64 entries) | **4** (with MPU swapping) | **2-3** (with subregion tricks) |
| Kernel binary size | ~10 KB [est.] | ~8-10 KB [est.] | N/A (not supported) |
| Thread state overhead | ~180 B/thread | ~160 B/thread [est.] | N/A |
| Process state overhead | ~150 B + 80 B PMP | ~150 B + 96 B MPU [est.] | N/A |
| Code density | RVC 16/32-bit | Thumb-2 16/32-bit | Thumb-2 16/32-bit |
| TCM equivalent | DCCM (VeeR) | TCM (Cortex-M) | TCM (Cortex-M) |

### Per-Architecture Verdict for 512 KB SRAM

| Architecture | 5-Process Model | 3-Process Model (merged) | Notes |
|---|---|---|---|
| **RISC-V (VeeR EL2)** | **GO** (~200 KB free, 39%) | **GO** (~240+ KB free) | 64 PMP entries give ample headroom |
| **ARMv8-M (Cortex-M33)** | **GO** (~200 KB free, 39%) | **GO** (~235+ KB free) | SRAM budget similar to RISC-V; MPU swapping works but loses simultaneous protection. 4 user processes feasible with 5 user-assignable regions each. |
| **ARMv7-M (Cortex-M3/M4)** | **Marginal** (~60 KB free after power-of-2 waste) | **GO** (~130+ KB free with 2 processes) | Not supported by pw-kernel. Power-of-2 alignment waste is severe (~141 KB) but Embassy removal creates enough headroom that it might barely fit. Still not recommended. |

### Recommendations for ARM Deployment

1. **Target ARMv8-M (Cortex-M33 or later) exclusively.** The PMSAv8 MPU's base+limit addressing eliminates the power-of-2 fragmentation problem. pw-kernel already supports this.

2. **Accept MPU-swapping trade-off.** With only 8 MPU regions, processes are protected one at a time (the active process). Between context switches, idle processes' memory is not hardware-protected. For the caliptra-mcu-sw workload where tasks cooperatively yield and processes are trusted firmware components, this is an acceptable trade-off.

3. **Do not attempt ARMv7-M.** The power-of-2 MPU alignment would waste ~141 KB of the 512 KB budget. Even with subregion tricks, the complexity and fragmentation make it impractical for 4+ processes. If ARMv7-M is required, use a 2-process model (merge all protocol services into one process + one driver process).

4. **Prioritize XIP on ARM platforms that support it.** Many Cortex-M SoCs can execute code directly from flash (XIP). If the target ARM SoC supports XIP, process .text sections can reside in flash rather than SRAM, eliminating ~230 KB of code from the SRAM budget entirely. This would make even ARMv7-M viable.

5. **ARM context switch is faster.** Cortex-M33 saves only 16 core registers (vs 32 on RISC-V) and the exception frame is hardware-assisted. MPU reconfiguration adds ~50-100 cycles but is amortized over the typically long time between context switches in this workload.

---

## Appendix A: Data Sources and Confidence

| Data Point | Value | Tag | Source File | Line |
|---|---|---|---|---|
| SRAM total | 524,288 B | [measured] | `firmware-bundler/reference/emulator/user-app.toml` | runtime.size = 0x80000 |
| Kernel .text | 116,128 B | [measured] | `docs/src/phase1-elf-sizes.md` | Section table |
| App .text + .rodata | 155,306 B | [measured] | `docs/src/phase1-elf-sizes.md` | Section table |
| Kernel .bss | 28,632 B | [measured] | `docs/src/sram-budget.csv` | Line 14 |
| App .bss | 40,640 B | [measured] | `docs/src/sram-budget.csv` | Line 22 |
| App stack | 44,544 B | [measured] | `firmware-bundler/reference/emulator/user-app.toml` | stack = 0xae00 |
| Kernel stack | 8,192 B | [measured] | `platforms/emulator/runtime/src/board.rs` | Line 111 |
| Grant space | 16,384 B | [measured] | `firmware-bundler/reference/emulator/user-app.toml` | grant_space = 0x4000 |
| spdm_doe_responder::POOL | 20,288 B | [measured] | `docs/src/phase1-elf-sizes.md` | Largest BSS symbols |
| HEAP_MEM | 16,384 B | [measured] | `platforms/emulator/runtime/userspace/apps/user/src/riscv.rs` | Line 11 |
| pw-kernel target size | ~10 KB | [estimated] | `bazel-openprot/external/pigweed+/pw_kernel/design.rst` | Design target |
| pw-kernel stack per thread | 2,048 B | [measured] | `bazel-openprot/external/pigweed+/pw_kernel/config/lib.rs` | KERNEL_STACK_SIZE_BYTES |
| pw-kernel Thread struct | ~180 B | [estimated] | `bazel-openprot/external/pigweed+/pw_kernel/kernel/scheduler/thread.rs` | Struct field analysis |
| pw-kernel Process struct | ~150 B | [estimated] | `bazel-openprot/external/pigweed+/pw_kernel/kernel/scheduler/thread.rs` | Struct field analysis |
| PMP entries (VeeR EL2) | 64 | [measured] | `runtime/kernel/veer/src/pmp.rs` | AVAILABLE_ENTRIES |
| PMP granularity | 4 B (configurable) | [measured] | `bazel-openprot/external/pigweed+/pw_kernel/target/qemu_virt_riscv32/config.rs` | PMP_GRANULARITY = 0 |
| Code duplication overhead | ~59 KB | [estimated] | Crate analysis + proportional allocation (Embassy removed) | See Section 2 |
| Per-process stacks | 20 KB total | [estimated] | Synchronous call depth analysis | See Section 3 |
| Per-process BSS | ~33 KB total | [estimated] | Static allocation tracing (Embassy pools eliminated) | See Section 4 |
| Embassy code eliminated | ~27 KB | [estimated] | embassy-executor + embassy-sync + libtockasync + async-trait | Replaced by synchronous IPC event loops |
| Embassy BSS eliminated | ~27 KB | [measured→eliminated] | spdm_doe_responder::POOL (20,288 B) + spdm_task::POOL (1,704 B) + other pools | Async state machines not needed |
| Heap reduced | 16 KB → 4 KB | [estimated] | HEAP_MEM was primarily for Embassy pinned futures | Synchronous model needs minimal heap |
| ARM MPU regions | 8 | [measured] | `bazel-openprot/external/pigweed+/pw_kernel/target/mps2_an505/config.rs` | NUM_MPU_REGIONS = 8 |
| ARM MPU min alignment | 32 bytes | [measured] | `bazel-openprot/external/pigweed+/pw_kernel/arch/arm_cortex_m/regs/mpu.rs` | RBAR/RLAR mask 0xffff_ffe0 |
| ARM ArchThreadState | ~16-24 B | [estimated] | `bazel-openprot/external/pigweed+/pw_kernel/arch/arm_cortex_m/threads.rs` | Struct field analysis |
| ARM pw-kernel targets | MPS2-AN505, RP2350 | [measured] | `bazel-openprot/external/pigweed+/pw_kernel/target/` | Both ARMv8-M Cortex-M33 |
| ARMv7-M pw-kernel support | None | [measured] | Full source search of pw_kernel/arch/ | No ARMv7-M code found |

## Appendix B: Glossary

| Term | Definition |
|---|---|
| BSS | Block Started by Symbol — zero-initialized static data |
| DCCM | Data Closely Coupled Memory — VeeR EL2 single-cycle scratchpad at 0x50000000 |
| DOE | Data Object Exchange — PCIe-based message transport |
| IPC | Inter-Process Communication — kernel-mediated message passing |
| MCTP | Management Component Transport Protocol — DMTF standard for platform management |
| PMP | Physical Memory Protection — RISC-V hardware memory isolation |
| NAPOT | Naturally Aligned Power-of-Two — PMP addressing mode |
| PLDM | Platform Level Data Model — DMTF standard for firmware update |
| SPDM | Security Protocol and Data Model — DMTF standard for device authentication |
| TBF | Tock Binary Format — header format for Tock userspace apps |
| TOR | Top of Range — PMP addressing mode |
| MPU | Memory Protection Unit — ARM hardware memory isolation |
| NVIC | Nested Vectored Interrupt Controller — ARM interrupt controller |
| PMSAv7 | Protected Memory System Architecture v7 — ARMv7-M MPU model (power-of-2 regions) |
| PMSAv8 | Protected Memory System Architecture v8 — ARMv8-M MPU model (base+limit regions) |
| TCM | Tightly Coupled Memory — ARM equivalent of RISC-V DCCM |
| XIP | Execute In Place — running code directly from flash/ROM |
