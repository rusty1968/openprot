# Caliptra-MCU SRAM Memory Budget

## 1. Theory of Operation: How the Kernel Gets Loaded into SRAM

### Overview

The caliptra-mcu system has **no execute-in-place (XIP)** capability. The entire firmware — kernel code, kernel data, application code, and application data — runs from a single 512 KB SRAM region. There is no separation between "code flash" and "data RAM" as on typical microcontrollers. Everything lives in SRAM at runtime.

### Memory Architecture

The MCU has three distinct memory regions:

| Region | Address | Size | Used By |
|--------|---------|------|---------|
| ROM | 0x8000_0000 | 32–128 KB | MCU ROM bootloader (immutable, hardware-mapped) |
| SRAM | 0x4000_0000 | 512 KB | Tock kernel + all apps (code + data) |
| DCCM | 0x5000_0000 | 16–256 KB | ROM stack/BSS at boot; PIC vector table at runtime |

ROM is true hardware ROM — it contains only the bootloader and executes from its fixed address. SRAM is the sole memory for all mutable firmware.

DCCM (Data Closely Coupled Memory) is a feature of the VeeR EL2 RISC-V core — it is a **separate physical memory**, not part of SRAM. It is a single-cycle-access scratchpad at a distinct address (0x5000_0000). The ROM uses DCCM for its stack and BSS during boot. At runtime, the kernel places only the PIC interrupt vector table (1 KB) at the top of DCCM. The kernel and apps do not otherwise use DCCM for code or data.

> **Note:** DCCM is specific to VeeR-based implementations. Other SoC integrations (e.g., ARM-based platforms like the AST1060, which uses Cortex-M with TCM) would have a different tightly-coupled memory architecture or none at all. The SRAM budget analysis in this document is independent of DCCM — nothing in the 512 KB SRAM budget relies on DCCM availability.

### Boot Sequence

The firmware loading follows a multi-stage process involving three processors: the MCU core, the Caliptra security co-processor, and an external BMC (Board Management Controller).

```
Power-on
  │
  ▼
MCU ROM starts at 0x8000_0000 (Cold Boot flow)
  │
  ├─ Initialize I3C recovery interface
  ├─ Assert Caliptra boot-go signal
  ├─ Wait for Caliptra "ready for fuses"
  ├─ Write fuses to Caliptra
  ├─ Lock security configuration
  │
  ├─ Send RI_DOWNLOAD_FIRMWARE mailbox command to Caliptra
  │     │
  │     ▼
  │   Caliptra ROM authenticates and loads Caliptra FW
  │   Caliptra Runtime starts
  │     │
  │     ├─ Receives SoC manifest over recovery interface → verifies
  │     ├─ Receives MCU runtime image over recovery interface
  │     ├─ DMA writes MCU image directly to MCU SRAM at 0x4000_0000
  │     ├─ Hashes MCU image in-place in SRAM
  │     ├─ Verifies hash against SoC manifest
  │     └─ Sets MCI FW_EXEC_CTRL[2] bit ("firmware ready")
  │
  ├─ MCU ROM polls for firmware ready
  ├─ MCU ROM verifies image header in SRAM
  ├─ MCU ROM triggers reset (writes RESET_REQUEST = 0x1)
  │
  ▼
MCU restarts → ROM enters Firmware Boot Reset flow
  │
  ├─ Checks RESET_REASON == FirmwareBootReset
  ├─ Validates firmware at SRAM entry point is non-zero
  └─ Jumps to SRAM: jr (0x4000_0000 + mcu_image_header_size)
       │
       ▼
     Tock kernel starts executing from SRAM
```

### Key Points

1. **Caliptra does the heavy lifting.** The MCU ROM never touches SPI flash directly during normal boot. It sends a single mailbox command (`RI_DOWNLOAD_FIRMWARE`) and Caliptra handles the entire firmware download, verification, and DMA into MCU SRAM.

2. **The firmware image is a single blob.** The firmware bundler packs the kernel binary and all app binaries (with TBF headers) into one contiguous image. Caliptra writes this entire blob to SRAM starting at 0x4000_0000. When the kernel starts, the apps are already in SRAM — there is no secondary loading step.

3. **Two resets to boot.** Cold boot loads the image but doesn't execute it. Instead, the ROM triggers a hardware reset, which causes the MCU to re-enter ROM with `RESET_REASON = FirmwareBootReset`. This second pass through ROM simply validates and jumps to the firmware. This two-reset design cleanly separates the loading phase from the execution phase.

4. **SRAM holds everything.** Since Caliptra DMA writes the firmware blob to 0x4000_0000, the kernel's `.text`, `.rodata`, app `.text`, `.rodata`, and all runtime data (stacks, heaps, BSS, grants) must fit within the 512 KB SRAM.

### SRAM Layout After Boot

The firmware bundler and kernel linker script partition SRAM into regions. The linker script names `rom` and `prog` are Tock conventions — they do **not** refer to hardware ROM.

```
0x40000000 ┌─────────────────────────────────┐
           │  "rom" region                   │  Kernel .text + .rodata
           │  (kernel executable code)       │  Loaded by Caliptra DMA
           │                                 │
0x4001D000 ├─────────────────────────────────┤
           │  "prog" region                  │  App TBF headers + .text + .rodata
           │  (application code)             │  Part of the same DMA blob
           │                                 │
0x40043000 ├─────────────────────────────────┤
           │  "ram" region                   │  Kernel stack, .data, .bss
           │  ┌─ Kernel .stack (8 KB)        │  Initialized by kernel startup
           │  ├─ Kernel .data (relocated)    │
           │  └─ Kernel .bss (zeroed)        │
           │                                 │
0x4004C000 ├─────────────────────────────────┤
           │  "app_ram" region               │  App stacks, .data, .bss, heaps
           │  (per-process memory)           │  Managed by Tock process loader
           │  ┌─ App .stack                  │
           │  ├─ App .data                   │
           │  ├─ App .bss                    │
           │  ├─ App heap                    │
           │  └─ Grant space                 │
           │                                 │
0x40080000 └─────────────────────────────────┘  End of SRAM (512 KB)
```

The kernel discovers apps by scanning TBF headers starting at the `_sapps` linker symbol (0x4001D000). It allocates per-process RAM from the `app_ram` region bounded by `_sappmem` (0x4004C000) and `_eappmem` (0x40080000). The linker asserts at build time that the kernel's own RAM usage (`_kernel_ram_done`) does not overflow into `app_ram`.

### Flash-Resident Storage (Not SRAM)

The `.storage` section (kernel log buffers, test logs) is mapped to a separate flash region at 0x3BFE0000, **not** SRAM. Tools like `rust-nm` report these as BSS symbols, but their addresses confirm they reside in flash. The 64 KB `LOG` buffer and test log buffers do not consume SRAM.

### Cryptographic Operations: Offloaded to Caliptra

The MCU does not perform any cryptographic operations itself. All crypto is offloaded to the Caliptra security co-processor via mailbox commands:

- **Firmware authentication** — Caliptra verifies SoC manifest signatures and hashes the MCU image in SRAM before allowing execution.
- **Key derivation** — Caliptra derives runtime keys (DICE identity, CDI, etc.) internally.
- **Runtime crypto services** — The MCU requests crypto operations through Caliptra mailbox commands (`CM_SHA_INIT/UPDATE/FINAL`, `CM_AES_GCM_DECRYPT_DMA`, `CM_STASH_MEASUREMENT`, etc.).
- **Firmware decryption** — If the MCU image is encrypted, Caliptra decrypts it in-place in SRAM via DMA.

This design means the MCU firmware carries no crypto library — no SHA, AES, ECC, or PQC implementations. The SPDM and PLDM protocol stacks in the user app handle message framing and state machines, but delegate all signature verification, hashing, and encryption to Caliptra. This significantly reduces the MCU's SRAM footprint compared to a design with an embedded crypto stack.

## 2. Kernel-Mode vs User-Mode Memory Split

Tock enforces a hard boundary between kernel memory and application (userspace) memory. The kernel owns the lower portion of SRAM; applications own the upper portion. The PMP (Physical Memory Protection) hardware enforces isolation at runtime — apps cannot access kernel memory or each other's memory.

### Split Overview (Emulator Build, 1 App)

```
SRAM 512 KB
┌─────────────────────────────────────────────────────────┐
│                                                         │
│  KERNEL MODE           304 KB (59.4%)                   │
│  0x40000000 – 0x4004C000                                │
│                                                         │
│  ┌─ Kernel .text + .rodata        113.4 KB  (code)      │
│  ├─ Alignment padding               1.6 KB              │
│  ├─ App .text + .rodata (prog)    151.7 KB  (app code)  │
│  ├─ Alignment padding               1.4 KB              │
│  ├─ Kernel .stack                    8.0 KB             │
│  ├─ Kernel .data                     0.0 KB             │
│  ├─ Kernel .bss                     28.0 KB             │
│  └─ (4 KB PMP alignment to app_ram boundary)            │
│                                                         │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  USER MODE             208 KB (40.6%)                   │
│  0x4004C000 – 0x40080000                                │
│                                                         │
│  ┌─ App .stack                     43.5 KB              │
│  ├─ App .data                       0.1 KB              │
│  ├─ App .bss (includes heap)       39.7 KB              │
│  ├─ Grant space                    16.0 KB              │
│  ├─ Alignment / rounding            0.7 KB              │
│  │                              ──────────              │
│  │  Allocated                    100.0 KB              │
│  │  Unallocated (free)           108.0 KB              │
│  └─                                                     │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### Kernel-Mode Memory (0x40000000 – 0x4004C000)

Everything below the `_sappmem` boundary (0x4004C000) is kernel-owned. This includes both kernel code/data **and** application code, because app `.text` and `.rodata` are in the `prog` region which is kernel-managed (the kernel maps it read-only for processes).

| Category | Size | % of SRAM | Contents |
|----------|------|-----------|----------|
| Kernel code | 113.4 KB | 22.1% | `.text` + `.rodata` in `rom` linker region |
| App code (kernel-managed) | 151.7 KB | 29.6% | TBF headers + `.start` + `.text` + `.rodata` in `prog` region |
| Kernel data | 36.0 KB | 7.0% | `.stack` (8 KB) + `.data` (36 B) + `.bss` (28 KB) |
| Alignment padding | 3.0 KB | 0.6% | Gaps between `rom`→`prog` and `prog`→`ram` |
| **Total kernel-mode** | **304.0 KB** | **59.4%** | |

The dominant kernel-mode consumers are:
1. **App code in `prog`** (151.7 KB) — the user-app binary is large due to SPDM, PLDM, MCTP, and DPE protocol stacks
2. **Kernel code in `rom`** (113.4 KB) — Tock kernel, drivers, capsules, and board setup
3. **Kernel BSS** (28.0 KB) — mostly MCTP/driver buffers (~24 KB of `BUF` arrays)

### User-Mode Memory (0x4004C000 – 0x40080000)

Everything from `_sappmem` to `_eappmem` is available for app processes. With one app loaded:

| Category | Size | % of SRAM | Contents |
|----------|------|-----------|----------|
| App stack | 43.5 KB | 8.5% | Process execution stack |
| App data (.data + .bss) | 39.8 KB | 7.8% | Includes 16 KB heap + 20 KB SPDM pool + protocol state |
| Grant space | 16.0 KB | 3.1% | Kernel-to-app shared state (per-capsule grants) |
| Alignment | 0.7 KB | 0.1% | Rounding to 4 KB boundary |
| **Allocated** | **100.0 KB** | **19.5%** | |
| **Free (unallocated)** | **108.0 KB** | **21.1%** | Available for additional apps or growth |
| **Total app_ram region** | **208.0 KB** | **40.6%** | |

The dominant user-mode consumers are:
1. **App stack** (43.5 KB) — configured in manifest; may be over-provisioned
2. **SPDM/DOE task pool** (20.3 KB) — `spdm_doe_responder::POOL` static in `.bss`
3. **App heap** (16.0 KB) — `HEAP_MEM` for `embedded-alloc`, within `.bss`
4. **Grant space** (16.0 KB) — Tock per-capsule grants for driver state

### Where the App Code Lives

An important subtlety: the **app executable code** (151.7 KB) lives in the kernel-mode region (`prog`), not in `app_ram`. The kernel maps this code read-execute for the process. Only the app's mutable state (stack, data, BSS, heap, grants) lives in `app_ram`. This means the app code does not compete with app data for the `app_ram` budget, but it does compete with the kernel for overall SRAM.

### Scaling: What Happens With More Apps?

The system is configured for `NUM_PROCS = 4`. Currently only 1 app is loaded. Adding more apps would:

- **Grow `prog`**: each additional app adds its code + TBF header
- **Grow `app_ram` allocation**: each app needs its own stack, BSS, heap, and grant space
- **Shrink free space**: the 108 KB unallocated `app_ram` is the growth budget

With the current single app using 100 KB of `app_ram`, there is room for roughly one more app of similar size before `app_ram` is exhausted — assuming the `prog` region can also accommodate the additional code.

## 3. Notable Static Buffer Allocations

Static buffers are fixed-size arrays placed in `.bss` (zero-initialized) or `.data` (initialized) at compile time. They are the primary consumers of SRAM beyond code. This section inventories all significant static allocations across kernel and user app.

### Kernel Static Buffers

The Tock kernel uses `static_init!` and component macros to allocate driver state and I/O buffers. Each component macro expands to one or more `kernel::static_buf!()` calls that create `static mut MaybeUninit<T>` arrays in kernel `.bss`.

#### MCTP Driver Buffers (dominant kernel consumer)

Each MCTP driver instance allocates **3 x 2,048-byte message buffers** via `mctp_driver_component_static!`:

| Buffer | Size | Purpose |
|--------|------|---------|
| `rx_msg_buf` | 2,048 B | Receive message assembly |
| `tx_msg_buf` | 2,048 B | Transmit message staging |
| `buffered_rx_msg` | 2,048 B | Buffered receive for app |

The emulator board instantiates **3 MCTP driver instances** (SPDM, PLDM, Caliptra):

| Instance | Source | 3 x 2,048 B |
|----------|--------|-------------|
| MCTP SPDM driver | `board.rs:567` | 6,144 B |
| MCTP PLDM driver | `board.rs:583` | 6,144 B |
| MCTP Caliptra driver | `board.rs:592` | 6,144 B |
| **Total MCTP driver buffers** | | **18,432 B (18 KB)** |

#### MCTP Mux (Transport Binding) Buffers

The MCTP mux layer (`mctp_mux_component_static!`) allocates I3C-level packet buffers:

| Buffer | Size | Purpose |
|--------|------|---------|
| `tx_buffer` | 250 B | I3C TX packet (MAX_READ_WRITE_SIZE) |
| `rx_buffer` | 250 B | I3C RX packet (MAX_READ_WRITE_SIZE) |
| **Total mux buffers** | **500 B** | |

#### Other Kernel Driver Buffers

| Buffer | Size | Source | Purpose |
|--------|------|--------|---------|
| Flash partition buffer | 512 B | `flash_partition_component_static!` | Flash read/write staging |
| DOE mailbox buffers | ~2 KB (est.) | `doe_component_static!` | DOE message buffers |
| MCU MBOX0 | ~4 KB (est.) | `mcu_mbox_component_static!` | MCU mailbox data staging |
| `DEFCALLS` bitmap | 256 B | Tock kernel | Deferred call bitmap |

#### Kernel Buffer Summary

| Category | Size | Notes |
|----------|------|-------|
| MCTP driver message buffers | 18,432 B | 3 instances x 3 x 2,048 B |
| MCTP mux I3C packet buffers | 500 B | 1 instance x 2 x 250 B |
| Flash/DOE/MBOX driver buffers | ~7 KB | Various driver staging |
| Other kernel state | ~2.5 KB | Deferred calls, alarm state, etc. |
| **Total kernel .bss** | **~28 KB** | Matches measured 28,632 B |

> **Observation:** MCTP message buffers (18 KB) account for ~65% of kernel BSS. Each protocol (SPDM, PLDM, Caliptra) gets its own set of 3 x 2 KB buffers. If protocols could share buffers or use smaller message sizes, this would be the largest single optimization target in kernel memory.

### User App Static Buffers

The user app's `.bss` (40,640 B) is dominated by a few large static allocations:

#### Embassy Task Pools (async executor state)

Each `#[embassy_executor::task]` generates a static `POOL` that holds the task's future state machine. The pool size equals the size of the compiled async function's state (all locals, temporaries, and await points).

| Symbol | Size | Task | Contents |
|--------|------|------|----------|
| `spdm_doe_responder::POOL` | 20,288 B | `spdm_doe_responder()` | DOE SPDM responder state machine |
| `spdm_task::POOL` | 1,704 B | `spdm_task()` | SPDM orchestrator state |
| Other task pools | ~1.5 KB (est.) | PLDM, MCTP-VDM, etc. | Smaller daemon tasks |

> **Observation:** The `spdm_doe_responder::POOL` at 20 KB is the single largest user-app allocation. This is the compiled size of the async state machine for the DOE SPDM responder -- it captures all stack frames across `.await` points. Large async functions with many local buffers produce large pools. Splitting into smaller async functions or reducing local buffer sizes would shrink this.

#### Heap

| Symbol | Size | Purpose |
|--------|------|---------|
| `HEAP_MEM` | 16,384 B | `embedded-alloc` heap for `Box`, `Vec`, etc. |

Used by the async runtime for heap-allocated futures (pinned I/O futures per MEM-1 in `requirements.md`) and dynamic protocol state.

#### Certificate / Protocol State

| Symbol | Size | Purpose |
|--------|------|---------|
| `SHARED_DPE_LEAF_CERT` | 2,072 B | DPE leaf certificate cache (2,048 B buffer + Mutex overhead) |
| Endorsement cert data (`.rodata`) | ~1,100 B | Root CA + DevID test certs (in `prog`, not `.bss`) |

### Combined Static Buffer Map

```
Kernel .bss (28 KB)                    User App .bss (40 KB)
+-----------------------+              +-----------------------+
| MCTP SPDM bufs  6 KB |              | DOE SPDM POOL  20 KB |
| MCTP PLDM bufs  6 KB |              | HEAP_MEM       16 KB |
| MCTP Caliptra   6 KB |              | DPE leaf cert   2 KB |
| Flash/DOE/MBOX  7 KB |              | SPDM task POOL  2 KB |
| Other state     3 KB |              | Other state     1 KB |
+-----------------------+              +-----------------------+
```

### What's NOT in SRAM (Common Misconceptions)

| Item | Size | Actual Location | Why it's confusing |
|------|------|-----------------|-------------------|
| `LOG` buffer | 64 KB | Flash (0x3BFE0000) | `rust-nm` shows it as BSS type `B` |
| `CIRCULAR_TEST_LOG` | 1 KB | Flash (0x3BFF0000) | Same -- `storage_volume!` macro |
| `LINEAR_TEST_LOG` | 1 KB | Flash (0x3BFF0400) | Same |
| MCU MBOX1 staging SRAM | 1 MB | MMIO (MCI peripheral) | `from_raw_parts_mut` at MCI offset, not SRAM |
| ROM stacks | 12 KB | DCCM (0x5000_0000) | Different memory region entirely |
