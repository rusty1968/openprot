# FPGA test debug notes

Setup for everything below: VCK190, Caliptra FPGA CI image, 2.1 bitstream from the
`main-2.1` manifest (`runtime_a2cd177.pdi`, commit `a2cd1773`), caliptra-mcu-sw tree at
Amit's commit `6032ae49` (branch `amit-smoke-test` on `rot-beast`), tests run with
`cargo xtask-fpga fpga test`.

Evidence labels: **[OBS]** seen in a log, **[SRC]** read in source, **[INF]** inferred,
**[UNK]** not established.

---

## 1. OpenPRoT `pldm_throughput_colo` crashes on the first host I3C command (2026-10-20)

### 1.1 Observation [OBS]

Test: `test_openprot_pldm_benchmark_64k` (patch `phase2/2b-openprot-pldm-benchmark.patch`).
The OpenPRoT FPGA image runs as the MCU runtime and the host runs the native PLDM update
agent (UA) against it.

- The image boots and logs `colocated i3c+mctp server starting`, `pldm fd: starting`,
  `pldm fd: waiting for update agent` (same as the boot-only test, step 2a).
- About 32.5M UART time units later (about 1.8 s if the units are core cycles at
  17.91 MHz [INF], which fits the UA's 2 s sleep before discovery) the kernel takes an
  exception:

```
Exception frame 0xa8c0ded0:   mstatus 0x00001800   mcause 0x00000002
mtval 0xbca51073              epc 0xa8c03a04
[FTL] Terminal exception in kernel mode
Exception frame 0xa8c0ddd0:   mcause 0x00000003   epc 0xa8c039ba
FAIL: 1
```

- The UA summary then shows `payload: 0 B`, `chunks: 0` and `n/a` for every phase: it
  never got a reply from the device.

### 1.2 Decoding the crash

- `mcause 2` = illegal instruction. For an illegal-instruction trap `mtval` holds the
  instruction word.
- `0xbca51073` decodes as: opcode `0x73` (SYSTEM), `funct3 = 1` (CSRRW), `rd = x0`,
  `rs1 = x10 (a0)`, `csr = 0xBCA`. That is `csrw 0xbca, a0`. [SRC: decoded by hand]
- CSR `0xBCA` is VeeR's **MEICPCT** (external interrupt claim ID / priority level capture
  trigger). [SRC: `runtime/kernel/veer/src/pic.rs` in caliptra-mcu-sw lists
  `MEICPCT ... 0xBCA` in the old driver]
- `nm -n` on the image ELF puts both `epc` values (`0xa8c03a04`, `0xa8c039ba`) inside
  `trap_handler` (symbol at `0xa8c038ee`). So the illegal instruction is executed in the
  trap/interrupt path, and the second exception (`mcause 3`, breakpoint) is the kernel's
  panic.

### 1.3 Source of the instruction [SRC]

Pigweed `pw_kernel/arch/riscv/veer_pic.rs`, `interrupt()` (around line 218):

```rust
// Claim the interrupt.  The VeeR document leads me to believe that I
// need to write to MEICPCT to capture the interrupt value into meihap.
MeiCPCT::write(MeiCPCTVal::default());
let meihap = MeiHAP::read();
```

(`MeiCPCT` is CSR `0xbca`, defined in `pw_kernel/arch/riscv/regs/veer.rs`.) The OpenPRoT
repo has no code of its own for this: it comes from the Pigweed dependency. Path in the
local Bazel cache:
`~/.cache/bazel/_bazel_antrocha/c29a3eb7.../external/pigweed+/pw_kernel/arch/riscv/`.

### 1.4 Why the native stack does not hit it [SRC]

caliptra-mcu-sw commit `335579894` "PIC and I3C fixes (#291)" (2025-07-29):

- message: "With this, I3C interrupts are working correctly on the FPGA";
- it **removed** the `meicpct` CSR member and the `self.meicpct.set(0)` write from
  `next_pending()` (the old comment said "writing 0 is recommended by the spec");
- the handler now reads `meihap` (`0xFC8`) directly and relies on PIC "fast interrupts".

The native driver still declares a `MEICPCT` bitfield but never writes the CSR.

### 1.5 Conclusion

**Most likely cause [INF]:** `pw_kernel`'s VeeR PIC interrupt handler writes MEICPCT
(`0xBCA`). On this FPGA's VeeR core that CSR write raises an illegal-instruction exception.
It fires at the first interrupt (I3C RX, when the UA's first command arrives), and the
kernel panics.

What supports it: the faulting instruction is exactly that CSR write, inside
`trap_handler`; the native stack removed the same write when it got I3C interrupts working
on the FPGA.

What is **not** established [UNK]:
- Commit `335579894` does not say the write faults. The link between "removed" and
  "faults" is an inference.
- I have not confirmed the faulting instruction is the `MeiCPCT::write` from
  `veer_pic.rs` by disassembling the image around `0xa8c03a04`.
- I have not tried a patched kernel, so this is not yet confirmed as the only problem.

### 1.6 What this does and does not show

- Host to device: the host's first I3C write reached the device far enough to raise the
  I3C interrupt. [INF, from the timing and the interrupt-time crash]
- Device to host (IBIs, private reads) is untested.
- Amit's `openprot_unittest_runner` passed on the same board because it does not use
  interrupts. The boot-only test (2a) passed for the same reason.

### 1.7 Implications for the plan (`openprot-pldm-benchmark-plan.md`)

New blocker, ahead of the existing gaps: **the pw_kernel VeeR interrupt path on the FPGA**.
Fixing it is an OpenPRoT-side kernel change (a patch to the Pigweed dependency that skips
the MEICPCT write for this core), then rebuild the image. The readiness, device identity,
and image-size gaps in the plan come after.

### 1.8 Next steps

1. Inspect how OpenPRoT's `MODULE.bazel` pins Pigweed and prepare a patch that drops the
   `MeiCPCT::write(...)` call (or guards it) in `veer_pic.rs`; rebuild the FPGA image.
2. Confirm by disassembling the image around `0xa8c03a04` (expect the `csrw 0xbca`).
3. Rerun `test_openprot_pldm_benchmark_64k` with the patched image.
4. In parallel, ask Amit and the OpenPRoT/Pigweed owners whether `pw_kernel` is expected
   to write MEICPCT on a Caliptra VeeR core, and whether they have run it with interrupts
   on the FPGA.

---

## 2. Other pitfalls found while getting the FPGA flow working

Short list, for reference. Details are in `caliptra-mcu-sw/fpga-bootstrap-guide.md`.

- **Stale cached MCU ROM on the board.** `get_or_compile_rom("")` in
  `tests/integration/src/lib.rs` copies the ROM from the bundle to
  `.../release/mcu_rom_prebuilt.bin` only `if !output.exists()`. A file from 2026-10-12 with
  a different hash than the current bundle's `mcu_rom.bin` made the custom-runtime test fail
  with `[mcu-rom-otp] OTP not idle` (fatal `0x00030001`, `ROM_OTP_INIT_NOT_IDLE`). Deleting
  the cache file (root-owned) fixed it, and `test_openprot_fpga_smoke` then passed. Check the
  hash before each session.
- **Bitstream and tree must match.** Amit's branch is the `main-2.1` line; its manifest
  pins the 2.1 bitstream. The 2.0 tree and bitstream gave different results.
- **Submodules.** `hw/caliptra-ss` and (on this branch) `network/net/lwip` must be
  initialized before `build-test`.
- **Per-feature runtimes.** Tests that use `TestParams::feature` need that runtime in
  `all-fw.zip` (`fpga build --separate-runtimes`, or `--runtime-features`), otherwise the
  harness tries to compile on the board and fails with `Runtime failed to compile`.
- **32-bit cycle counter wrap.** The wrapper `cycle_count` wraps after about 239.8 s
  (17.91 MHz). Benchmarks longer than that report wrong ticks; use wall time (see
  `pldm-fw-update-benchmark-report.md`).
- **Disk space on `rot-beast`.** Builds with `--separate-runtimes` plus duplicate builder
  images (`podman images` shows an untagged old image after xtask pulls a new `latest`)
  filled the disk.

---

## 3. After the Pigweed patch: MCTP-over-I3C framing mismatch (2026-10-21)

### 3.1 Observation [OBS]

Same test (`test_openprot_pldm_benchmark_64k`), image rebuilt with
`third_party/pigweed/veer_no_meicpct.patch` (openprot commit `df1ee4b5`).

- The kernel no longer crashes. No `Exception frame` / `mcause` lines.
- About 32.5M UART units after `pldm fd: waiting for update agent`, the first host frame
  arrives and the device logs `[ERR] i3c frame decode failed` (from `mctp_i3c_colo.rs`,
  when `MctpI3cReceiver::decode` fails). No further host frames are handled.
- The FD idles for 15 s (`idle timeout (no further UA command)`), then
  `update did not complete`, `FAIL: 13`. The UA summary is all `n/a`.

Conclusion on the earlier issue [INF]: the MEICPCT write was the cause of the crash. The
I3C interrupt path now runs (IRQ, frame latched, `recv`, decode). The patch itself is
confirmed by the change in behaviour, not by disassembly.

### 3.2 Cause: the two stacks frame MCTP-over-I3C differently [SRC]

Host UA side (caliptra-mcu-sw, Amit's tree), `common/testing/src/i3c_socket.rs`:
`send_private_write` appends `pec = calculate_crc8(addr << 1, data)` (line ~250). The frame
on the wire is **[MCTP packet][PEC]**, where the PEC covers the I3C target address byte
(`addr << 1`, write) followed by the packet. On reads it checks the PEC over
`(target_addr << 1) | 1` plus the data (line ~347).

Native device side, `runtime/kernel/capsules/src/mctp/transport_binding.rs`
(`receive_write`): same layout. PEC = CRC8 over `device_address << 1` followed by the buffer;
the MCTP packet starts at byte 0.

OpenPRoT device side, `services/mctp/transport-i3c/src/encap.rs`, `decode_frame`: expects
**[dest<<1][0x0F][byte_count][src<<1|1][MCTP packet][PEC]**, an SMBus-style header, with the
PEC computed over everything before it (header + packet, no separate address byte), and
`byte_count + 3 == len`. It rejects a frame whose second byte is not the command code
`0x0F`. The host's frame starts with the MCTP header byte instead, so decode fails. The
transmit direction (`I3cSender`) emits the same SMBus-style frame, which the host's PEC
check (above) would also reject.

OpenPRoT's own host test (`host_test.rs`) builds frames in the SMBus-style format, so it is
consistent with OpenPRoT's stack but not with the caliptra-mcu-sw host harness.

### 3.3 Options [INF]

- **A. Change OpenPRoT's transport** (`services/mctp/transport-i3c`) to the native framing
  ([packet][PEC over address byte + packet]) behind a feature. The wire format then matches
  the native stack. Changes the device stack under test, and the receiver needs the target's
  dynamic address for the PEC.
- **B. Translate in the host harness** (test-only): in the FPGA model's I3C forwarder
  (`forward_i3c_to_controller`, `handle_i3c`) wrap host frames into the SMBus-style format
  on the way in and strip and re-PEC them on the way out. Leaves the OpenPRoT image unchanged.
  Adds a few bytes per packet and a little host work in the OpenPRoT test only.

Leaning towards B for bring-up (no device changes), and noting the 4-byte header per packet
as a property of OpenPRoT's wire format when comparing results.

### 3.4 Not yet checked [UNK]

- Whether anything else differs after framing (MCTP tags/TO bit handling, EIDs, PLDM
  discovery commands). The plan's gaps 1, 2, 4, 5 are still open.
- The I3C target dynamic address OpenPRoT's receiver would need for option A.

---

## 4. From "frame decodes" to a working UA/FD conversation (2026-10-21)

Setup as before (VCK190, 2.1 bitstream, Amit's `6032ae49`, board checkout on the same
commit), image built from OpenPRoT branch `mctp-i3c-dsp0233-framing` (Pigweed MEICPCT
patch + DSP0233 framing). Test: `test_openprot_pldm_benchmark_64k`.

### 4.1 Framing fix confirmed on hardware [OBS]

With the diagnostic image (banner `diag build`), `i3c frame decode failed` no longer
appears; the device logs `i3c diag: irq event class=1` followed by an `rx pkt` line for
the first host frame. An earlier run without a banner still showed the decode error.
Most likely that run used the old image (the logs of that run were not checked for
`MCU len`) [INF]; the framing fix is the explanation consistent with everything seen
since.

### 4.2 Next blocker: destination EID 0 is dropped silently [SRC + OBS]

First packet received: `len=9 dest=0 src=10 flags=0xc8 type=0x01 inbound_ok=1`, then no
reply for 15 s.

- The harness addresses UA requests to EID 0: `MctpUtil::new_req` sets `dest_eid = 0`
  (`common/testing/src/mctp_util/common.rs:68`), and `wait_for_responder` and
  `send_request` both call it. The native MCU stack tolerates that.
- OpenPRoT's `mctp-lib` `Router::inbound` drops any message whose destination is not its
  own EID and returns `Ok(())`: "Drop messages if eid does not match (for now)". The
  caller ignores the result, so `inbound_ok=1` does not mean delivered.
- The harness's response path also swaps direction (`set_src_eid(dest)`,
  `set_dest_eid(source)`).

Fix (harness only, opt-in): `CPTRA_MCTP_STRICT_EIDS="<ua>,<fd>"`.
- `2c-strict-eids.patch`: `strict_eids()` in `mctp_transport.rs`, source EID and response
  direction; the test sets `"10,8"`.
- `2d-new-req-dest.patch`: 2c set the destination too early (`wait_for_responder` calls
  `new_req` again and reset it; first run after 2c showed `dest=0 src=10`). `MctpUtil`
  gets `req_dest_eid: Option<u8>` and `new_req` uses it.
Result: `dest=8 src=10`, the device answers.

### 4.3 Harness asserts the first packet's sequence number is 0 [SRC + OBS]

After the first response the UA thread panicked at (patched) `common.rs:575`, which is
original line 565: `assert_eq!(mctp_hdr.pkt_seq(), seq_num)` in `assemble`, left 1,
right 0. The OpenPRoT response started at sequence 1. DSP0236 allows a message to start
at any sequence number 0-3. `2e-pkt-seq.patch` takes the first packet's number as the
base. This is a relaxation of an over-strict check, unconditional, so native tests are
unaffected.

### 4.4 UA and FD now converse [OBS]

After 2e the log shows eight request/response pairs, all `dest=8 src=10`, with matching
IBIs (`mdb=0xae`) and `irq event class=2` for each read. By packet size this looks like
PLDM base discovery (SetTID, GetTID, GetPLDMTypes, GetPLDMVersion and GetPLDMCommands
for base and firmware-update types) ending with a header-only request (probably
QueryDeviceIdentifiers) answered with a 19-byte IBI. Then the UA sends nothing more and
the FD idles out after 15 s (`FAIL: 13`). The command identification is inferred from
sizes [INF].

### 4.5 Why it stops: device identity [SRC, cause INF]

OpenPRoT's `MockFdOps::get_device_identifiers` returned 0 descriptors and
`get_firmware_parms` returned defaults. The UA's package targets UUID
`01 02 ... 10` and a Firmware component (classification `0x000A`, identifier `0xffff`,
comparison stamp `0xffffffff`). The native FD reports exactly those
(`platforms/emulator/runtime/userspace/apps/user/src/firmware_update/config.rs`:
UUID, one component, active stamp `0x12345678`). The UA most likely found no matching
device and stopped; the UA-side log that would say so was not captured.

OpenPRoT change (`pldm_throughput_colo/pldm_fd.rs`, uncommitted, FPGA image 130,812 B):
- report the UUID and the native firmware parameters (same `pldm-common` types);
- take the component size from `UpdateComponent` (`FirmwareComponent::comp_image_size`)
  instead of the hard-coded 4096 (gap 4);
- count and discard downloaded bytes, like the native `bench-skip-validation` path.
Risk: building `FirmwareParameters` on the stack in the `pldm_fd` process could overflow
its stack (not yet observed).

### 4.6 State of the changes

| Change | Where | State |
|---|---|---|
| Pigweed MEICPCT patch | OpenPRoT `df1ee4b5` | committed (has a Claude trailer) |
| DSP0233 framing (transport, callers, host tests) | OpenPRoT `b2b856be`, `80561d81` | committed; 7 of 7 emulator tests passed |
| Diagnostic logging in `pldm_throughput_colo/mctp_i3c_colo.rs` | OpenPRoT | uncommitted, local, to be dropped |
| FD identity, size, discard (`pldm_fd.rs`) | OpenPRoT | uncommitted, not yet run on the board |
| 2b (benchmark test, PLDM-ready override), 2c, 2d, 2e | caliptra-mcu-sw on `rot-beast`, branch `amit-smoke-test` | applied, uncommitted; copies in `phase2/` |

### 4.7 Next

1. Run the image with the FD changes (`MCU len 130812`); look for RequestUpdate /
   PassComponentTable / UpdateComponent and `RequestFirmwareData` traffic and a filled-in
   benchmark summary.
2. Remaining plan gaps afterwards: the PLDM-ready milestone (currently bypassed with
   `CPTRA_FORCE_PLDM_READY`), the 4 KiB buffer is gone but chunk size and flow need a look
   once the download runs, and whether the emulator host tests still pass with the FD
   changes (not rerun yet).
3. Drop the diagnostics, then commit the FD change; decide how to carry the harness
   patches (local patch or upstream request).

### 4.8 FD identity works; the download stalls after the first chunk (2026-10-22) [OBS]

Run with the FD changes (image 130,812 B), harness patches 2c, 2d, 2e. `1 test run: 1 passed,
295 skipped` (only the filtered test ran).

UA-side benchmark summary:

| phase | MCU ticks | wall (s) |
|---|---|---|
| discovery | 310,146 | 1.732 |
| negotiation (QueryDeviceIdentifiers .. UpdateComponent) | 205,075 | 1.145 |
| download | n/a | n/a |

`payload: 65536 B, chunks: 1, requested: 196 B, max chunk: 196 B`. So the UA matched the
device (gap 2 closed), negotiated 196 B chunks, and the FD sent its first
`RequestFirmwareData`. IBI lengths, in order, match the PLDM commands by size
[INF]: 35 (QueryDeviceIdentifiers with one UUID descriptor), 99 (GetFirmwareParameters),
13 (RequestUpdate), 12 (PassComponentTable), 18 (UpdateComponent), 17 (RequestFirmwareData
request). After that the device sent nothing and the FD idled out after 15 s.

### 4.9 Cause: the UA's first *response* leaves with the wrong EIDs [SRC, cause INF]

`MctpUtil::send_response` calls `new_resp()`, which swaps source and destination and clears
the tag-owner bit (`common.rs:323-324`). The non-strict path pre-swaps for that reason
(`set_src_eid(dest)`, `set_dest_eid(source)` -> src = UA, dest = FD after the swap).
My 2c strict branch set UA/FD directly, so after the swap the packet left with source =
FD (8) and destination = UA (10). OpenPRoT's router drops packets whose destination is
not its own EID (4.2), silently. The first UA response, to RequestFirmwareData, is the
first message that goes through this path, which matches where the download stops.

Correction to 4.2: the harness's response path is not "inverted"; it relies on the swap
inside `send_response`.

Fix: `2f-response-eids.patch` sets the strict pair pre-swapped (FD as source, UA as
destination). Host harness only; the OpenPRoT image is unchanged.

---

## 5. The chunk response loses fragments 3 and 4 (2026-10-22)

### 5.1 2f worked; the stall moved to the device's receive side [OBS]

With 2c, 2d, 2e, 2f applied and the FD changes (4.5), every UA message now arrives with
`dest=8 src=10`. The UA gets through discovery (about 1.7 s) and negotiation (about 1.2 s),
the FD sends its first `RequestFirmwareData` (IBI length 17), and the UA answers. The
device then logs only **two** of the four fragments of that 201-byte response:

```
rx pkt len=68 dest=8 src=10 flags=0x81 type=0x01   (SOM, seq 0, TO=0, tag 1)
rx pkt len=68 dest=8 src=10 flags=0x11 type=0x3b   (seq 1, TO=0, tag 1)
```

Fragments 3 and 4 (`0x21`, `0x71` expected) never reach the receive path. No
`colo i3c inbound ring full`, no `i3c frame decode failed`, no host-side
`Error writing I3C data`. The FD idles out after 15 s and the benchmark summary shows
`chunks: 1`, `chunk_loop 196 B`, download `n/a`. The count is the same in every run
(deterministic, not a race). The two received fragments were about 70 ms apart on the
device, much longer than the host forwarder's 5 ms write spacing.

### 5.2 Things tried that did not change the result [OBS]

- Driver change in OpenPRoT's `CaliptraI3cTarget::rx_read` (uncommitted): no W1C clear of
  `RxDescStat` and a zero-length descriptor means empty, as in the native driver. The run
  with this image (banner `diag build 4`) still shows exactly two fragments.
- A larger diagnostic budget so the response fragments are logged (they now are, which is
  how 5.1 was seen).

### 5.3 Hardware hints in the harness [SRC]

`common/testing/src/i3c_socket.rs` (`send_packetized_write`) says each packet must fit in
the "256-byte TTI RX FIFO" and inserts 25 ms between packets, "to allow the target to drain
its 256-byte RX data FIFO", plus about 5 ms from the FPGA controller. The native driver
documents the TTI RX/TX FIFO as 64 DWORDs (`MAX_READ_WRITE_SIZE = 256`). The UA's chunk
response is sent as four back-to-back 69-byte frames (68 + PEC) with no pacing beyond the
5 ms forwarder sleep. `XI3CWrapper::write` is `master_send_polled`: it waits up to 2 s
for the target response and returns an error on a NACK, which the model would print.

Pacing hypothesis [INF]: the device does not drain its RX FIFO fast enough, so later
frames of the burst are lost. For: the comments above, the deterministic two frames, the
slow device (my UART diagnostics add tens of ms per frame). Against: three frames (207 B)
should still fit in 256 B; no failed write is reported; earlier runs stalled the same way
when the per-fragment diagnostics were not printing. Not established.

### 5.4 Review: native I3C driver vs OpenPRoT driver [SRC]

Native: `runtime/kernel/drivers/i3c/src/core.rs` (Amit's tree). OpenPRoT:
`target/veer/peripherals/i3c/lib.rs` and `services/i3c/server/src/lib.rs`.

| # | Area | Native | OpenPRoT |
|---|---|---|---|
| 1 | RX status | never writes `RxDescStat`; loops while it is set | W1C-cleared after every single descriptor read (changed locally, see 5.2) |
| 2 | Empty descriptor | `len == 0` means done | returned `Some(0)` (changed locally) |
| 3 | PIC gateway | explicitly sets all gateways active-high, level-triggered (`pic.rs:99-102`) | Pigweed `veer_pic.rs`: "meigwctrl is currently unused", reset default assumed. **Unverified.** |
| 4 | Event handling | handles each status bit separately | `on_interrupt` sets `ResponseRead` for `IbiDone`, then overwrites it with `InboundReady` if `RxDescStat` is also set; `LastIbiStatus` was already read, so `ResponseRead` is lost and `tx_pending` can stay set, holding later sends |
| 5 | Interrupt enables | `RxDescStat` + `IbiDone` + `TxDescComplete` | `RxDescStat` + `IbiDone`; `TxDescComplete` never handled |
| 6 | IBI failure | re-sends the IBI when the status is non-zero | ignores the status |
| 7 | TX staging | waits 100 nops after writing TX data, then raises the IBI | no delay |
| 8 | RX drain | pops only when an RX buffer exists, retries on an alarm | drains immediately into a 6-slot ring |
| 9 | Size limits | documents the 64-DWORD (256 B) FIFO, clamps lengths | `MAX_PAYLOAD` 250, not FIFO-aware |
| 10 | RX descriptor error bits | defined, ignored | ignored |

Ranking for "2 of 4 fragments" [INF]: item 1 explains it for the old driver, but the fixed
driver behaves the same, so the hardware status semantics, item 3, or the slow drain are
open. Item 4 is a real defect likely to cause a later stall in the device-to-host
direction.

### 5.5 Next steps

1. **Register dump (diag build 5, not yet built):** `CaliptraI3cTarget::debug_regs()`
   (TTI interrupt status, queue status, descriptor and data queue depths, target error
   status, status, framing and read-PEC error counters), logged after each received
   packet. Shows whether frames 3 and 4 sit in the hardware queue or never arrived.
2. **Host write log:** `2g-i3c-write-log.patch` (host-only) prints each forwarder write
   (length, time waiting for FIFO space, time taken). Shows whether the host attempts the
   third and fourth writes.
3. Check the PIC gateway type for the I3C interrupt (kernel side); set it to level like
   the native driver if it is edge.
4. If the diagnostics themselves slow the device, rerun with them removed.
5. Fix item 4 (event override) once reception works.

### 5.6 State of the changes (additions to 4.6)

| Change | Where | State |
|---|---|---|
| FD identity, size, discard | OpenPRoT `pldm_fd.rs` | uncommitted, running on the board |
| I3C driver: no W1C of `RxDescStat`, zero-length = empty | OpenPRoT `target/veer/peripherals/i3c/lib.rs` | uncommitted; emulator tests not rerun, and the emulator model's behaviour with this is unverified |
| Register dump (`debug_regs`) | same file | uncommitted, not yet built |
| Packet/IRQ diagnostics | OpenPRoT `pldm_throughput_colo/mctp_i3c_colo.rs` | uncommitted, local, to be dropped |
| 2f (UA response EIDs, pre-swapped for `new_resp`) | caliptra-mcu-sw on `rot-beast` | applied, uncommitted; verified (responses arrive `dest=8 src=10`) |
| 2g (host write log) | caliptra-mcu-sw | patch in `phase2/`, not yet applied |

### 5.7 Root cause found: `RxDescStat` drops while a descriptor is still queued (2026-10-22) [OBS]

Run with diag build 5 (register dump) plus the host write log (patch 2g), same FD and
harness state as 5.1. Host side:

- All four chunk-response writes were issued and succeeded: `i3c write ok len=69 ...`
  (x3) and `len=14` (the short fourth fragment: 9 payload bytes + 4 MCTP header + PEC),
  each with `waited_ms=0 took_ms=0`. So the host sends everything and the target ACKs it.

Device side (three fragments received this time, which differs from 5.1, probably
because of the added logging; the last one is still stranded):

| after received fragment | `irq` (TTI interrupt status) | `qs` | RX desc depth | RX data depth |
|---|---|---|---|---|
| 1 (flags `0x81`) | `0x1` (RxDescStat set) | `0x288` | 2 | 22 DWORDs |
| 2 (flags `0x11`) | `0x1` | `0x288` | 2 | 22 DWORDs |
| 3 (flags `0x21`) | **`0x0`** | `0x288` | **1** | **4 DWORDs** |

Decoding (generated register layout, `registers/generated-firmware/src/i3c.rs`):
`tti_desc_queue_depth[7:0]` is `RxDescQueueDepth`, `tti_data_queue_depth[7:0]` is
`RxDataQueueDepth` in DWORDs, and `qs = 0x288` has the RX queues not empty. A 69-byte
frame is 18 DWORDs and the 14-byte fragment is 4 DWORDs, so after fragment 3 the hardware
still holds exactly the fourth fragment (1 descriptor, 4 DWORDs) while `RxDescStat` is
**clear**. The driver (and the native pattern `while RxDescStat != 0`) stops reading
there, and since nothing new arrives, no further interrupt ever comes for it. The same
mechanism explains the earlier runs that lost fragments 3 and 4: one or two descriptors
were left queued once the status dropped. The exact condition under which the status
clears (it was set with two descriptors queued and clear with one) is observed, not
documented; I have not found the i3c-core definition.

Not the cause: the host (writes succeeded), the FIFO size or pacing (the missing data
is in the hardware queue, not dropped), the framing, EIDs or sequence numbers, and the
`W1C`-clear change from 5.2 (the symptom was identical before and after it).

Note the earlier hypothesis in 5.3 (FIFO overflow / pacing) was **not** the cause. The
register dump is the evidence that replaces it.

### 5.8 Fix [INF, not yet verified]

`CaliptraI3cTarget::rx_read` now treats a frame as available when either `RxDescStat` is
set **or** the RX descriptor queue depth (low byte of `tti_desc_queue_depth`) is non-zero.
The status bit stays as a trigger so a model that does not implement the depth register
(the emulator) behaves as before. `latch_inbound`'s drain loop then keeps reading until
the queue is empty, so the last fragment is read in the same pass. Image: banner
`diag build 6`, 131,540 bytes. Uncommitted; not yet run on the board.

Caveats: the emulator tests have not been rerun with this change, so it is unknown whether
the emulator model implements `tti_desc_queue_depth`. The native caliptra-mcu-sw driver
uses the same `while RxDescStat != 0` pattern and so may be exposed to the same stranding
under a burst of three or more frames; worth telling its owners once this is confirmed.

### 5.9 State of the changes (update to 5.6)

| Change | Where | State |
|---|---|---|
| `rx_read`: queue-depth check, no W1C of `RxDescStat`, zero-length = empty | OpenPRoT `target/veer/peripherals/i3c/lib.rs` | uncommitted, diag build 6 not yet run |
| `debug_regs()` register dump + packet logging | OpenPRoT driver and `mctp_i3c_colo.rs` | uncommitted, local, to be removed |
| 2g (host write log) | caliptra-mcu-sw on `rot-beast` | applied for the run in 5.7; uncommitted, to be removed |

### 5.10 Next

1. Run diag build 6; expect all four fragments, `desc=0x0` afterwards, and the FD
   requesting further chunks.
2. If the transfer completes, record the throughput and note the logging overhead
   (UART lines add tens of ms per frame), then rerun without diagnostics for a clean number.
3. Rerun the seven emulator tests with the driver change before committing it.
4. Later device-to-host item from 5.4 (event override losing `ResponseRead`) is still open.
5. Clean up: drop the diagnostics and 2g; commit the driver fix, the FD change, and decide
   how to carry the harness patches (2b-2f).

## 6. Transfer dies after 19 chunks: MCU watchdog (2026-10-22)

### 6.1 Diag build 6 result [OBS]

The RX-descriptor fix (5.8) works: the transfer went from 1 chunk to 19
(`chunks: 19, requested: 3724 B, max chunk: 196 B`, `avg MCU ticks per chunk: 181936`;
the average is inflated by UART diagnostics). The run then ended with
`Emulator exited with error: firmware fatal error: 0x00000010`. The last UART
timestamps were about 799-800M cycles, and one UART line was garbled by the
"FPGA MCU ROM" banner, i.e. the MCU reset and ran the ROM again.

### 6.2 Cause [SRC, cause INF]

The FPGA MCU ROM arms MCI WDT1 with 800M cycles (about 44.7 s at 17.91 MHz). The
native runtime disables it at OS init (`platforms/fpga/runtime/src/board.rs:1297-1298`,
`Mci::disable_wdt` -> `mci_reg_wdt_timer1_en = 0`, MCI offset 0xb0). OpenPRoT never
touched it, so the MCU reset about 44.7 s after boot and the ROM reported fatal 0x10.
Evidence is the timing match (about 800M cycles) and the ROM banner mid-run; I have
not yet seen the register read back.

### 6.3 Fix [INF, not yet verified on the board]

`target/veer/entry.rs`, `main()`, `#[cfg(feature = "fpga")]`: volatile write of 0 to
`kernel_config::MCI_BASE + 0xb0` before `kernel::main`. Shared by all FPGA test
images. FPGA image builds (`pldm_throughput_colo_fpga_runner`); uncommitted.

### 6.4 Expected on the next run

- No `0x10` fatal error; the transfer continues past 19 chunks.
- A full 64 KiB transfer needs about 335 chunks. With diag logging it may run long;
  the 32-bit cycle counter wraps at about 239.8 s, which would corrupt the measurement.
  Rerun without diagnostics for a clean number.

### 6.5 Also recorded

- Emulator regression after the driver change: 11 of 12 pass. The failure is a
  pre-existing build error in `services/i3c/client/tests/loopback.rs`, unrelated.

## 7. Full 64 KiB transfer completes (2026-10-22)

### 7.1 Result [OBS]

With the watchdog disabled (6.3), `test_openprot_pldm_benchmark_64k` passed (exit 0):
335 chunks, 65536 B requested and received, max chunk 196 B. FD logged
`idle timeout`, `update applied, exiting`, `PASS`. Last UART timestamp about 3.43G cycles.

| phase | MCU ticks | wall (s) |
|---|---|---|
| discovery | 427482 | 2.387 |
| negotiation | 353197 | 1.972 |
| download | 26366497 | 147.213 |

Download throughput 0.43 KiB/s; average 78744 ticks per chunk (about 0.44 s).
Native baseline (same test, same harness): about 40.4 s, 1.58 KiB/s, about 0.12 s per chunk.
So this run is about 3.6x slower than native.

### 7.2 Caveats [INF]

- NOT a clean number: diag build 6 is still active (UART logging for the first 120
  packets plus per-packet register dumps), and patch 2g logs every host I3C write. UART
  lines cost tens of ms each, but the budget only covers the first 120 packets, so
  logging alone probably does not explain 3.6x. The remainder is unexplained.
- Tick/wall mismatch to check: 26.4M ticks over 147.2 s is about 179 kHz, not the
  17.91 MHz clock assumed earlier (the UART timestamps do match 17.91 MHz: 3.43G cycles
  is about 191 s). Understand which counter the benchmark's tick column uses before
  quoting ticks; wall seconds look consistent with the UART timeline.
- Host log shows `Discarding stale client read (len=256), 0 remaining`: the host read
  the device's TX FIFO and got stale data. Related to the open device-to-host items
  (event override losing `ResponseRead`, `TxDescComplete` not handled).

### 7.3 Next

1. Clean run: remove diagnostics (`mctp_i3c_colo.rs`, `debug_regs`) and patch 2g; rebuild; rerun.
2. If still about 3x slower, find where the time goes per chunk (host 5 ms socket sleep,
   IBI/read round trips, response retries, the stale-read path).
3. Cleanup and commit (driver fix, FD change, watchdog fix); rerun emulator tests first.

## 8. Command lines: clean OpenPRoT 64 KiB run (2026-10-22)

State: OpenPRoT branch `mctp-i3c-dsp0233-framing`. Commits: `6b84a35c` (RX driver fix),
`7b90420f` (FD identity), `578a33b1` (watchdog), `8bf03243` (diagnostics, known-good
baseline), `a3e0421b` (revert of diagnostics; this is the clean build).

### 8.1 Build the FPGA image (WSL, OpenPRoT repo)

```
cd ~/work/pldm-i3c/openprot
bazel build --config=k_veer --//target/veer:target_type=fpga \
  //target/veer/tests/pldm_throughput_colo:pldm_throughput_colo_fpga_runner
sha256sum bazel-bin/target/veer/tests/pldm_throughput_colo/pldm_throughput_colo_image.bin
```
Built from `a3e0421b52fec4a1ab19b2641fa46cbd0d54a633` (`git rev-parse HEAD`, clean tree). Clean build sha256: `a7545b068769db1b94fad4c582dbfa00df50842099dcc1a806dd73cc552b9429`.
Entry point should be 0xa8c00000 (FPGA MCU SRAM base).

### 8.2 Copy to rot-beast (load the key into the agent in the same shell first)

```
ssh-add
scp \
  ~/work/pldm-i3c/openprot/bazel-bin/target/veer/tests/pldm_throughput_colo/pldm_throughput_colo_image.bin \
  antrocha@rot-beast:work/caliptra-mcu-sw/tests/integration/src/openprot_pldm_throughput_colo.bin
```
Then verify the copy on rot-beast; the output must match the hash in 8.1 exactly:

```
ssh antrocha@rot-beast sha256sum work/caliptra-mcu-sw/tests/integration/src/openprot_pldm_throughput_colo.bin
# expect: a7545b068769db1b94fad4c582dbfa00df50842099dcc1a806dd73cc552b9429
```
Note: a `\` continuation must be the last character on its line.

### 8.3 On rot-beast (in ~/work/caliptra-mcu-sw)

```
# remove the host write-timing log (patch 2g) so it does not skew timing
git apply -R <path>/2g-i3c-write-log.patch

# make sure no stale cached ROM is used
rm -f <cache dir>/mcu_rom_prebuilt.bin

cargo xtask-fpga fpga build-test --target-host runner@10.223.163.182

CPTRA_FORCE_PLDM_READY=1 cargo xtask-fpga fpga test --target-host runner@10.223.163.182 \
  --test-output \
  --test-filter='package(caliptra-mcu-tests-integration) and test(test_openprot_pldm_benchmark_64k)'
```
If the env var does not reach the board-side run, pass it another way.

### 8.4 Check the result

```
grep -E "chunks:|requested:|avg MCU ticks|fatal error|FPGA MCU ROM|PASS|FAIL" <log>
```
- Pass: exit 0, 335 chunks, 65536 B, no `fatal error`.
- Previous run (with diagnostics and 2g): download 147.2 s, 0.43 KiB/s, 78744 ticks/chunk.
- Native baseline: about 40.4 s, 1.58 KiB/s.

### 8.5 Going back to the diagnostics build, if needed

```
cd ~/work/pldm-i3c/openprot
git revert a3e0421b      # or: git checkout 8bf03243 -- target/veer
```

## 9. Clean run result (2026-10-22)

Image `a7545b06…9429` (OpenPRoT `a3e0421b`, diagnostics reverted), patch 2g reverted on
rot-beast. `test_openprot_pldm_benchmark_64k` passed: 335 chunks, 65536 B, max chunk 196 B,
test finished in 165.8 s.

| phase | MCU ticks | wall (s) |
|---|---|---|
| discovery | 241279 | 1.347 |
| negotiation | 192166 | 1.073 |
| download | 21894909 | 122.247 |

Download 0.52 KiB/s, 65401 ticks per chunk (about 0.365 s per chunk).

| | download (s) | KiB/s | per chunk |
|---|---|---|---|
| Native (reproduced baseline) | about 40.4 | 1.58 | about 0.12 s |
| OpenPRoT, diag build + 2g (section 7) | 147.2 | 0.43 | about 0.44 s |
| OpenPRoT, clean | 122.2 | 0.52 | about 0.365 s |

[OBS] Removing the diagnostics and 2g saved about 25 s (17%), so they were not the main
cost. OpenPRoT is still about 3x slower than native for the same payload, chunk count,
and host UA. Final UART timestamp 2.94G cycles (about 164 s at 17.91 MHz) matches the
165.8 s test time, so wall seconds look real. The tick column still implies about 179 kHz
(21.9M ticks / 122.2 s); not yet explained.

[OBS] Host log still shows `Discarding stale client read (len=256), 0 remaining` at the end.

[INF] The gap is per chunk, not setup: discovery and negotiation are about 2.4 s total.
Candidate causes (not tested): device-to-host path (IBI then read round trip per
request, `ResponseRead` event lost, `TxDescComplete` unhandled), response fragmentation
(the host sends a 196 B chunk as multiple MCTP packets), the colo server's wake/ack
pattern, or kernel scheduling between the two processes.

### 9.1 Next

1. Locate where the 0.365 s per chunk goes: use UART timestamps (cycles) on a few events
   per chunk (request sent, first packet in, last packet in, response out) on a short run.
2. Compare the same events on the native runtime if it can be instrumented.
3. Commit the harness side (patches 2b-2f) in caliptra-mcu-sw; not done yet.

### 9.2 Correction: patch 2g was still applied during the section 9 run [OBS]

`grep -n "i3c write ok" hw/model/src/model_fpga_realtime.rs` on rot-beast matched line 261,
so the host write timing log was active. The 122.2 s result is therefore NOT a clean
number (the log prints one line per host I3C write and reads `Instant`; the cost is
probably small but unmeasured). The 8.3 revert step was not done. To redo:

```
# on WSL: ssh-add; scp ~/work/open-prot/phase2/2g-i3c-write-log.patch antrocha@rot-beast:work/caliptra-mcu-sw/
# on rot-beast, in ~/work/caliptra-mcu-sw:
git apply -R 2g-i3c-write-log.patch
grep -c "i3c write ok" hw/model/src/model_fpga_realtime.rs      # expect 0
cargo xtask-fpga fpga build-test --target-host runner@10.223.163.182
# then the run from 8.3, and: grep -c "i3c write ok" <log>      # expect 0
```

### 9.3 Rerun with diagnostics and 2g both removed [OBS]

Image `a7545b06…9429`, 2g reverted (`grep -c "i3c write ok"` = 0), 2b-2f intact.
`test_openprot_pldm_benchmark_64k` passed: 335 chunks, 65536 B, test time 165.9 s.

| phase | MCU ticks | wall (s) |
|---|---|---|
| discovery | 251461 | 1.404 |
| negotiation | 194032 | 1.083 |
| download | 21888075 | 122.209 |

0.52 KiB/s, 65364 ticks per chunk. Compared with 9 (2g still applied): 122.209 s vs
122.247 s, so 2g had no measurable effect. This is the clean OpenPRoT number:
**122.2 s, 0.52 KiB/s, about 3x slower than native (about 40.4 s, 1.58 KiB/s)**.
The gap is about 0.245 s per chunk (0.365 s vs 0.12 s), consistent across runs, which
suggests a fixed per-chunk latency rather than throughput-bound work. Next: section 9.1.

## 10. Per-chunk timing instrumentation (2026-10-22)

Goal: find where the 0.245 s per chunk (9.3) goes. Uncommitted on top of `a3e0421b`
(`mctp_i3c_colo.rs`, `pldm_fd.rs`). Each line is an `evt ...` log; the UART timestamp
in front of it is the core cycle count (17.91 MHz). Only the first events are logged
(colo budget 200 lines, FD budget 30 lines, about 15 chunks) to limit UART cost.

Events per chunk, in expected order:
1. `evt fd: next request off=N` (FD builds the next RequestFirmwareData)
2. `evt i3c: send staged` (request handed to the I3C driver; IBI raised)
3. `evt i3c: irq ibi done` (IbiDone interrupt) / `irq inbound`
4. `evt i3c: rx pkt len=L` (response fragment received)
5. `evt fd: chunk data off=N` (FD callback has the data)

Read the log by subtracting timestamps: 1 to 2 is FD and MCTP server overhead, 2 to 4 is
host round trip (IBI, host read, host UA, host write), 4 to 5 is device-side reassembly
and delivery, 5 to the next 1 is the FD's own processing. At 17.91 MHz, 0.365 s is about
6.5M cycles.

Run: copy the image as in 8.2 (sha256 below), `build-test`, run as in 8.3, then
`grep "evt " <log> | head -80`.
Instrumented image sha256: `192283264a3c0315de42c4067988f18dec18df800c581f3a62e9d4df1228b9a6` (uncommitted on `a3e0421b`).

### 10.1 First instrumented image crashed at startup (store fault) [OBS, cause INF]

Image `19228326…` crashed in user mode right after `pldm_fd` was allocated:
`mcause 0x7` (store access fault), `mtval 0xa8c48000`, `epc 0xa8c1edb0`, `sp 0xa8c67ff0`.
The ELF shows `.static_init_ram.pldm_throughput_colo_0` (4 bytes) at exactly 0xa8c48000,
the start of that process's RAM. The only new non-zero static was `EVT_BUDGET:
AtomicU32 = 200` in `mctp_i3c_colo.rs`; its first `store` hit that address. So a store
to a mutable static placed in a process's `.static_init_ram` faults, i.e. that word is
not writable (looks like a process memory-map or PMP issue in the OpenPRoT pw_kernel
setup for processes that have initialised mutable statics; not investigated, and I did not
see it before because no process had one). Symbolizing `epc` did not work (process
symbols are stripped from the combined image).

Workaround: count up from a zero-initialised static (`EVT_COUNT`, cap 200). The rebuilt
ELF has a zero-length `.static_init_ram` for the process. New image sha256:
`913e884f15d4ea3762862098adbcbccd2d27d4067b82df3dbcb3fbc720d85bc9`.
Open item: root-cause the non-writable `.static_init_ram` (affects any future OpenPRoT
process with an initialised mutable static).

### 10.2 The zero-initialised static crashed too; counters are now locals [OBS]

Second image (`913e884f…`, zero-initialised `EVT_COUNT`) crashed the same way: store fault
at `mtval 0xa8c48000`, now with `a0=0xa8c48000, a2=4`, `sp=0xa8c68000` (top of that
process's RAM, so right at startup), consistent with the startup code initialising a
4-byte data word at the first word of the process RAM. Without my changes both
`.static_init_ram` and `.zero_init_ram` of that process are empty and the stack starts at
0xa8c48000; with a 4-byte static in either section the startup store faults. So (INF,
not root-caused): any process that has static data hits a non-writable first RAM word.
Revised hypothesis for 10.1: not the non-zero initial value, but any static.

Fix: no statics. `EVT_COUNT` is now a local `Cell<u32>` in `colo_loop`, shared with both
`ColoTransport`s. ELF check: `.static_init_ram` and `.zero_init_ram` both 0 bytes again.
Image sha256: `5f86eb3ce38238e03ce96aec151d70de3e3de543d6a4cc3fcaa04810fac69122`.
Open item: root-cause the non-writable first RAM word for processes with static data.

### 10.3 Per-chunk timeline from the instrumented run [OBS, interpretation INF]

Image `5f86eb3c…9122`. No exception. Timestamps are core cycles (17.91 MHz: 1M cycles =
55.8 ms; the benchmark's "MCU ticks" are cycles/100). Steady-state chunk (request for
off=196), from the `evt` lines:

| segment | cycles | ms |
|---|---|---|
| fd next request -> i3c send staged (FD, MCTP server, IPC, sender) | 1.57M | 88 |
| send staged -> irq ibi done (IBI raised and accepted) | 0.42M | 23 |
| ibi done -> first inbound irq (host reads request, UA, host starts write) | 1.70M | 95 |
| first inbound irq -> last rx fragment (4 fragments: 68, 68, 68, 13 B) | 2.05M | 115 |
| last rx fragment -> fd chunk data (reassembly, IPC to FD) | 1.44M | 80 |
| fd chunk data -> next request (FD between callbacks) | 1.05M | 59 |
| **total per chunk (instrumented)** | **8.2M** | **about 450** |

Clean runs measure 6.5M cycles (0.365 s), so the 9 log lines per chunk add about 1.5M;
each UART line costs roughly 0.2M cycles (consecutive lines are 0.38-0.5M apart, which
includes real work). Treat each segment as +-0.2M.

Findings:
- [OBS] The build is not unoptimised: rustc runs with `opt-level=s` (aquery), so that
  theory is out.
- [INF] About half of each chunk is device-side software (rows 1, 5, 6: about 4M
  cycles, about 0.2 s), more than native's whole chunk time (about 2.15M cycles).
  The FD callbacks themselves are trivial, so the cost is in the path around them:
  `run_terminus`, the MCTP server, the IPC hops, and the sender.
- [INF] The remaining half is the I3C/host round trip (IBI to host response, rows 2-4,
  about 4.2M). The 4 response fragments arrive 0.4-0.7M cycles apart; the host
  forwarder sleeps 5 ms (0.09M) per write, so the device may also add delay per fragment.
  Native uses the same host code, so this half is probably comparable, but I have not
  confirmed it.
- Negotiation messages (8-37 B) take about 3-4M cycles per request/response exchange.

Next candidates: (1) rerun with only 4 log lines per chunk to cut the measurement skew;
(2) find the CPU cost of the device-side path without UART skew (e.g. emulator cycle
counts per chunk for the same image); (3) look at what the MCTP server and
`run_terminus` do per message (timers, waits, IPC round trips).

## 11. Fine-grained device trace (2026-10-22)

Replaces the `evt` log lines of section 10 (they cost about 0.2M cycles each and skewed
the timeline). Events are recorded as `(tag, kernel clock ticks, extra)` into a local
buffer via `syscall::debug_clock_now()` (1 tick = 100 core cycles, 5.58 us), and printed
afterwards as `tr idx tag ticks extra`, 4 lines per wake-up, once the buffer is full
(colo 640 entries, FD 120 entries), so the hot path has no UART output. Uncommitted.

Tags: 1 irq entry, 2 on_interrupt done (1 inbound / 2 ibi-done / 3 other), 3 latch done,
4 irq acked, 5 frame decoded (len), 6 mctp inbound done, 7 send begin (len), 8 send end
(driver TX write plus IBI raise), 9 callback exit, 10 FD next request (offset), 11 FD
chunk data (offset).

Segments this separates: 7->8 is the I3C driver send; 1->2 is `on_interrupt`; 2->3 is
the RX drain (driver); 3->5 is client recv plus frame decode; 5->6 is MCTP inbound
(reassembly); 6/9 -> 11 is delivery to the FD; 11 -> 10 is the FD between callbacks;
10 -> 7 is FD, MCTP server and sender down to the driver.

Image sha256: `eac56bc399505e0afa1e655f5babc4d029aa5371b54346797e13c9c2dee36a8d`.
Analysis script (outside the repos): `tools/analyze_trace.py LOGFILE [N]`; it merges the
FD and colo trace lines by ticks and prints per-event deltas in ms. Sanity check: one
steady-state chunk should span about 65-80k ticks.

### 11.1 Result of the trace run [OBS, interpretation INF]

Run `trace-run.log` (image `eac56bc3…6a8d`): passed, 335 chunks, no exception; the 760
trace lines all printed. Download 148 s (slower than the clean 122 s because the dump
prints 4 lines per wake-up once the buffers fill, so only the early chunks are usable).

**Correction to section 11:** the kernel clock ticks are core cycles (17.91 MHz), not
cycles/100: one chunk spans about 7.2M ticks, not 65-80k. `tools/analyze_trace.py`
still divides as if 1 tick = 100 cycles, so its printed ms are 100x too large. The
numbers below are corrected (real ms = printed ms / 100). The script fix is pending.

Steady-state chunk (request for offset 392), 7.18M cycles = 401 ms:

| segment | ms | share |
|---|---|---|
| FD next request -> send begin (FD, MCTP server, IPC, sender) | 85.7 | 21% |
| send begin -> send end (I3C driver: TX write + IBI raise) | 14.9 | 4% |
| send end -> IBI-done irq (host accepts the IBI) | 7.2 | 2% |
| IBI-done callback (about 2.5 handling, 26.7 from ack to exit) | 29.2 | 7% |
| callback exit -> first response fragment irq | 2.9 | 1% |
| 4 response fragments: first irq entry -> last callback exit | 136 | 34% |
| last callback exit -> FD chunk data (MCTP server, IPC to FD) | 68.9 | 17% |
| FD chunk data -> next request (FD between callbacks) | 56.1 | 14% |

Inside the 136 ms receive phase: `irq acked -> frame decoded` (client recv + frame decode)
is about 20.4 ms per 68-byte fragment (about 97 ms of the 136), 15.6 ms for the 13-byte
one, about 2.5 ms for an 8-byte frame; the RX drain in the driver (`latch`) is 2-3 ms
per frame; `on_interrupt` is 0.44 ms; MCTP `inbound` is 1.7-2.2 ms per frame.

Findings:
- [OBS] The register-level I3C driver (send 14.9, on_interrupt, RX drain) is about 25 ms,
  about 6% of a chunk. It is not the main cost.
- [OBS] FD + MCTP server + IPC around the driver (rows 1, 7, 8) is about 211 ms, 53%.
- [OBS] Client recv + frame decode in the colo process is about 97 ms, 24%, and grows
  with frame size (about 0.3 ms per byte = about 5.4k cycles per byte). Not explained:
  the bitwise CRC-8 in `encap.rs` should cost a few hundred cycles per byte at most.
  Needs a finer split (recv vs decode).
- [OBS] The host side is not the bottleneck here: callback exit to the first response
  irq is only 2.9 ms, and the IBI-done latency is 7.2 ms.
- [INF] The 26.7 ms between `irq acked` and `callback exit` in the IBI-done callback,
  where nothing is decoded, is probably another thread (the FD) running before this
  one resumes; not confirmed.
- Early sends with small payloads (11-20 B) take about 14.9 ms in the driver, while
  larger ones (36-100 B) take 1.9-2.2 ms. Unexplained.

Next: split `irq acked -> frame decoded` into recv and decode; look at what the FD and
MCTP server do in rows 1, 7, 8; check why a driver send takes 14.9 ms for small payloads.

## 12. CPI (cycles per instruction) investigation (2026-10-22)

### 12.1 Review claim and what I found in the code [SRC]

`cpu-efficiency-review.md` item 2 infers an effective CPI of about 50 (a no-data `recv`
of about 500 instructions takes 27k cycles) and suspects the SRAM window is uncached.

MRAC (VeeR cacheability CSR) is computed per 256 MB region (`common/config/src/lib.rs`,
`compute_mrac`). A region containing both memory and MMIO becomes MMIO ("safety first":
side effects, not cacheable). On the FPGA map, MCU SRAM is at 0xa8c00000, in the same
256 MB region (0xa0000000-0xafffffff) as the I3C (0xa4030000), MCI (0xa8000000), mailbox,
SoC, OTP and LC register blocks. So that whole region is non-cacheable by construction,
and every instruction fetch and data access in SRAM goes over the bus uncached. [INF:
I read the rule and the map; I did not read MRAC on the board.]

Consequences [INF]:
- This is a property of the SoC memory map, not of OpenPRoT: the native runtime also
  lives in SRAM (0xa8c...) and uses the same MRAC. So CPI does not explain the 3x
  difference by itself; native simply executes far fewer instructions per chunk.
- It does make instruction count the thing to optimise (CRC table, copies, zero-fills,
  IPC round trips), since each instruction costs tens of cycles.
- The region cannot be made cacheable without moving SRAM away from the MMIO blocks.
  DCCM (0x50000000, 16 KB, handoff data at 0x50003C00) is fast, but I do not know whether
  the core can fetch code from it; it could hold hot data (stacks, buffers). Not tried.

### 12.2 Measurement [added, not yet run]

`mctp_i3c_colo.rs` runs a calibration at start: a 3-instruction register-only loop
(`addi; add; bnez`, 20,000 iterations = 60,000 instructions), timed with
`debug_clock_now` (ticks = cycles), and logs `cpi cal: ... took N cycles`. CPI is
N / 60,000: about 1-3 means fetch is cached or fast; about 20-50 confirms uncached fetch.
Image sha256 `4f66efb1d94175d462c67c6245f1c0008b966c2d042d7b6878172f2d58a46810` (also
includes the CRC table and the trace). Temporary, uncommitted.

### 12.3 Native runtime has the same memory placement [SRC]

The native FPGA runtime is built from the bundler manifest
`firmware-bundler/reference/fpga/user-app.toml`: `runtime_memory.sram` is one block at
0xa8c00000, 512 KB, holding instructions and data together (the bundler splits it 10/16
instructions, 6/16 data, `firmware-bundler/src/ld.rs:101`). The DCCM (0x50000000) is only
0x3C00 bytes used for the VeeR PIC vector table, with the ROM/runtime handoff table at
0x50003C00; ROM is at 0xb0040000. The MCU ROM sets MRAC (`platforms/fpga/rom/src/start.s:34-40`,
`FPGA_MRAC_VALUE` from `compute_mrac()`), before any runtime starts, for both stacks.

So native and OpenPRoT both execute from the same SRAM under the same MRAC; CPI is the
same, and the 3x gap is instruction count (plus waits), not a different memory setup.
Still unmeasured: the CPI itself (12.2 calibration) and the MRAC value read on the board.

### 12.4 Calibration result and CRC-table run (2026-10-22) [OBS]

`cpi cal: 20000 iters (3 instr each) took 3499487 cycles` -> 3,499,487 / 60,000 =
**58 cycles per instruction** in SRAM (image `4f66efb1…6810`). Independent of the
review's estimate (about 50 from a no-data `recv`: about 500 instructions in 27k cycles).
Caveat: the loop has one taken branch per 3 instructions, so straight-line code may be a
bit cheaper per instruction; treat 50-58 as the range.

Consequences:
- 7.2M cycles per chunk is about 124k instructions; native's about 2.15M cycles per chunk
  is about 37k. OpenPRoT executes roughly 3x the instructions per chunk, on the same core
  and memory (12.3). So reducing instruction count is the lever.
- Every removed instruction is worth about 58 cycles (3.2 us): the table CRC replaces
  about 60-90 instructions per byte with a few.

CRC table run (same instrumentation as the trace run, image `4f66efb1…6810`): passed,
download 127.2 s versus 148.0 s for the trace run with the bitwise CRC (both include the
trace dump overhead), about 21 s (14%) faster, in line with the review's estimate
(about 60 ms per chunk). Per-fragment decode times not yet compared (log not copied).
Not a clean number: trace and calibration still in the image.

### 12.5 Effect of the CRC table, from the traces (`trace-run.log` vs `cpi-run.log`) [OBS]

Both traces from the same instrumented image apart from the CRC. `irq acked -> frame decoded`
(client recv + decode), median cycles:

| frame | bitwise CRC | table CRC |
|---|---|---|
| 68 B (n=58/60) | 364,922 (20.4 ms) | 58,638 (3.3 ms) |
| 13 B (n=21/22) | 279,218 (15.6 ms) | 36,305 (2.0 ms) |
| 8 B | 44,707 | 34,019 |

Per-fragment decode is about 6x faster for 68 B frames. Chunk period (FD next request to the
next, chunks before the colo trace buffer fills, so no dump overhead): 402 ms (19 chunks)
-> 361 ms (20 chunks), i.e. -41 ms per chunk (-10%), less than the sum of the decode
savings (about 65 ms per chunk: 3 x 306k + 243k cycles). [INF] The rest is hidden: the
device decodes one fragment while the host is still sending the next, so only the part not
overlapped with host writes shortens the chunk.
Remaining per chunk: 361 ms. Exceptions in the log: 0.

## 13. client-ipc zero-fill removed; request handle reused (2026-10-22)

Changes (OpenPRoT working tree, uncommitted, on top of the CRC table):
1. `services/mctp/client-ipc/src/lib.rs` `send_recv`: the request is sent straight from
   `request_buf` (disjoint field borrow of `request_buf` / `response_buf`); the per-call
   `[0u8; MAX_REQUEST_SIZE]` plus copy (about 1 KB memset + memcpy per IPC call, 5 calls per
   chunk) is gone.
2. `services/mctp/api/src/stack.rs`: `Stack::req_reusable(eid, timeout)` keeps the request
   handle in `Stack::cached_req` when the channel is dropped, instead of unbinding it. The
   handle is kept only if the channel completed cleanly (every send answered by a recv, no
   error); after any error it is released as before (a timeout can leave a receive pending
   in the server). A cached handle for a different EID is released and replaced; `Stack`
   releases it on drop.
3. `services/pldm/src/transport.rs` `send_request` uses `req_reusable`, so the FD's
   RequestFirmwareData requests no longer allocate and unbind a handle per chunk (saves 2 of
   about 5 IPC round trips per chunk, one of them in the 68.9 ms "last fragment -> FD gets
   data" segment).

Checks: 4 new `Stack` unit tests (handle kept across 3 channels, released on stack drop,
replaced for another EID, released after an unanswered send); all 14 host test targets in
`services/mctp`, `services/pldm` pass (incl. `firmware_update_host_test`, which runs the real
server); FPGA image builds with no static data. Not yet run on the board.
Note [SRC]: `mctp-lib` never sets `ReqHandle::last_tag`, so `unbind` never cancelled a flow;
reusing the handle does not change flow lifetime.

Image (still carries the trace and the calibration, for comparison with `cpi-run.log`,
chunk period 361 ms): sha256 `a48cffa728d19c6b03b900755e301a28b0017edf317c9a5973b03f4b379882e3`.
Compare the chunk period (FD next-request to next-request, chunks before the colo trace
buffer fills) and the segments `FD next request -> send begin`, `last callback exit -> FD
chunk data`.

### 13.1 Result on the board (image `a48cffa7…82e3`) [OBS]

Passed, 335 chunks, 0 exceptions. Download 88.2 s (15,794,185 ticks, 47,204 ticks per chunk =
4.72M cycles = 263 ms per chunk), 0.73 KiB/s. Test time 131.8 s.

| image | download (s) | KiB/s |
|---|---|---|
| bitwise CRC, trace (16:43 run) | 148.0 | 0.43 |
| table CRC, trace + calibration (`cpi-run.log`) | 127.2 | 0.50 |
| + no 1 KB zero-fill, request handle reused (this run) | 88.2 | 0.73 |
| native (reproduced baseline) | about 40.4 | 1.58 |

This step alone: -39 s (-31%), larger than the review's estimate for these two items. All
rows except native include the trace dump overhead (and the calibration), so a clean
build will be faster. Remaining gap to native: about 2.2x on this build. Per-segment
numbers pending the log (`ipc-run.log`).

### 13.2 Per-segment effect (cpi-run.log vs ipc-run.log), medians over the first 18 chunks [OBS]

| segment | table CRC | + IPC changes |
|---|---|---|
| FD next request -> send begin | 66.3 ms | 26.4 ms |
| send begin -> send end (driver) | 14.9 | 1.8 |
| send end -> first inbound irq | 84.7 | 98.2 |
| first inbound irq -> last callback exit (4 fragments) | 73.3 | 73.3 |
| last callback exit -> FD chunk data | 72.1 | 29.9 |
| FD chunk data -> next request | 56.6 | 33.2 |
| **total chunk** | **361.0** | **263.0** |

Inside "send end -> first inbound irq" (ms): send end -> IBI-done irq 6.5 -> 44.6;
irq entry -> ack 2.5 -> 2.5; ack -> callback exit 40.4 -> 1.5; callback exit -> first
inbound irq 34.8 -> 49.2. Sum 84.7 -> 97.8.

Findings:
- [OBS] FD-side segments (rows 1, 5, 6) fell from 195 ms to 90 ms. The 14.9 ms "driver
  send" and the 40 ms between `irq acked` and `callback exit` in the IBI-done callback
  (the long-unexplained windows) both collapsed. [INF] They were the FD thread's IPC work
  running while the colo thread was waiting: the fewer IPC calls and the lack of the 1 KB
  copy removed them. The driver itself takes 1.8 ms to send.
- [OBS] "send end -> first inbound irq" is about 98 ms before and after: the device is
  mostly idle there; the host takes the IBI, reads the request, the UA builds the response
  and the host starts writing. [INF] The work that used to overlap inside this window moved
  (IBI-done irq entry now comes later), but the window itself did not shrink.
- [OBS] The 4-fragment receive is 73 ms in both; decode is now only about 3 ms per
  fragment, so this is mostly the host pacing its writes (5 ms sleep per write plus FIFO
  waits), not device work.
- Roughly: host/bus-bound about 171 ms (98 + 73), device compute about 90 ms of the
  263 ms. Native is about 120 ms per chunk in total with the same host code, so either
  the host-bound part is shorter on native, or something in how the device behaves
  lengthens the host's part. Not determined; needs host-side timestamps.

### 13.3 Host-side patches and the native baseline [user-reported, corrected]

Correction: I first wrote this section as if the native benchmark had been run before and
after the host patches. What was said: the native 64 KiB benchmark was first reproduced on
this FPGA with a delta of about 1% against the reference result, and since then the only
changes to the host side are the patches 2b-2f (made by this work). So the native baseline
(about 40.4 s, about 120 ms per chunk) was measured on the unpatched host. It has NOT
been re-measured with the patches applied. The patches touch shared host code
(`common/testing/src/mctp_transport.rs`, `mctp_util/common.rs`, `model_fpga_realtime.rs`);
they are meant to change behaviour only for the OpenPRoT path, but that is unmeasured.

Next check: rerun `test_firmware_update_benchmark_64k` (native) on `rot-beast` with the
current patched host tree. If it stays within about 1-2% of 40.4 s, the host patches do not
skew the comparison and the remaining gap is device behaviour; if not, the patches are
part of the difference.

### 13.4 Native benchmark with the patched host [OBS]

`test_firmware_update_benchmark_64k` on `rot-beast` with patches 2b-2f applied (2g
reverted): passed, 335 chunks, download 40.536 s (7,260,185 ticks, 21,668 ticks per
chunk = 2.17M cycles = 121 ms), 1.58 KiB/s. The reference baseline was about 40.4 s, so the
delta is about 0.3%: the host patches do not skew the native result. The comparison holds:
same host code, native 121 ms per chunk versus OpenPRoT 263 ms (instrumented image;
13.2). The host-bound windows of the OpenPRoT chunk (about 171 ms) are longer than
native's entire chunk, so the difference must come from how the device behaves toward the
host. Next: timestamped host-side logs on both stacks.

## 14. Host-side timestamps, same host on both stacks (2026-10-23)

Goal: compare what the host sees per chunk with the native device and with the OpenPRoT
device (13.4: host patched, native 121 ms per chunk vs OpenPRoT 263 ms).

`phase2/2h-host-timestamps.py FILE` edits `hw/model/src/model_fpga_realtime.rs` in place
(every anchor must match exactly once, else it aborts without writing) and adds
`[hw-ts] <microseconds> <event>` lines: `ibi received`, `ibi read begin/end`,
`client read queued`, `client read discarded`, `client read begin`, `write wait`,
`write end` (host I3C write toward the device; the gap to the previous `write end` or
`write wait` shows FIFO waiting). Checked: applies to the local 2b-patched file, passes a
rustfmt parse check. Not compiled against the FPGA features. Revert by restoring the
pre-2h copy of the file.

Device image for these runs: clean build of OpenPRoT `533a67b4` (CRC table, IPC changes,
watchdog fix; no trace, no calibration): sha256
`07df11f39fb565f951629a1e1decfd7d0287e4643ecbf5f5ba67e2886143b40c`
(`phase2/openprot_pldm_throughput_colo_clean_533a67b4.bin`). Its OpenPRoT benchmark result is
also the first clean number for the current code.

Runs: OpenPRoT benchmark and native benchmark, each logged to a file; compare the
`[hw-ts]` event timelines for one chunk.

### 14.1 Result: the gap is device processing after the last fragment [OBS]

Logs: `host-ts-native.log`, `host-ts-openprot.log` (same host build, 2h timestamps, patch 2g
off; device image for OpenPRoT = clean build of `533a67b4`, sha256 `07df11f3…b40c`).
Both passed, 335 chunks, 4463 `[hw-ts]` lines each.

Benchmark tables:

| | download | KiB/s | per chunk |
|---|---|---|---|
| native | 40.736 s | 1.57 | 21,774 ticks = 121 ms |
| OpenPRoT (clean, `533a67b4`) | **71.999 s** | 0.89 | 38,501 ticks = 215 ms |

OpenPRoT is now 1.77x native (first clean number for the current code; it was 3x).

Host-visible timeline of one chunk, medians over 331 chunks (period = `ibi received len=17`
to the next):

| segment | native | OpenPRoT |
|---|---|---|
| IBI received -> first host write (host UA turnaround) | 51.9 ms | 51.0 ms |
| four host writes (first wait -> last end) | 16.6 | 16.4 |
| **last host write end -> next IBI received (device)** | **36.6** | **131.6** |
| period | 107.4 | 204.2 |

The host-side parts are identical on both stacks (UA turnaround and write pacing). The
whole difference, about 95 ms per chunk, is the time the device takes from receiving the
last fragment to raising the next request's IBI: decode, MCTP reassembly, delivery to the
FD, the FD's callbacks and next request, MCTP send, I3C send and IBI. That corrects my
earlier reading (13.2) that about 171 ms of the chunk was host/bus-bound: those windows
overlapped device work. The device compute is the lever; at 58 cycles per instruction,
95 ms is about 1.7M cycles, about 29k instructions.

From 13.2, the device-side segments on that path are: last callback exit -> FD chunk data
29.9 ms, FD chunk data -> next request 33.2, FD next request -> send begin 26.4, driver
send 1.8. Remaining candidates from the review: 1 ms responder poll after every step
(item 5), server loop overhead per wake (item 7), copies and per-call zero-fills in the
I3C client and sender (item 6), remaining IPC round trips (item 4).

## 15. Timing diagram and rate-limited responder poll (2026-10-23)

Diagram: `/home/antrocha/work/open-prot/chunk-timing-diagram.md` (host timeline for both
stacks, device-internal flow of one chunk with the current time allocations, A-G table).

Change (OpenPRoT working tree, uncommitted, on top of `533a67b4`):
- `services/pldm/src/firmware_device.rs`: `FirmwareDevice::set_responder_poll_interval(steps)`
  (default 1 = unchanged behaviour). While an FD-initiated request is active, the responder
  poll (1 ms timeout, one IPC round trip per step) runs once every `steps` steps; when idle it
  still blocks on every step. A UA command arriving between polls is answered up to `steps`
  requests late (about 16 x 0.2 s = 3 s at the current speed): a CancelUpdate would be seen
  late. Open question: does the UA have a response timeout shorter than that for commands sent
  during a transfer? Not checked; the benchmark UA sends none.
- `services/pldm/tests/firmware_update_host.rs`: the full-flow test is now a helper run twice
  (default, and every 16 steps); both pass, with all other pldm tests (6 targets).
- `target/veer/tests/pldm_throughput_colo/pldm_fd.rs`: `RESPONDER_POLL_EVERY_STEPS = 16`.
  Only this benchmark FD uses it.

Clean image (no trace, no calibration; HEAD `533a67b4` + the change above): sha256
`40449d0b104b14171e55c2ce4f031b3c83e468f39d0f1e4903d64450b77ccf98`
(`phase2/openprot_pldm_throughput_colo_poll16_clean.bin`). Compare with the 72.0 s clean run
(`host-ts-openprot.log`, 2h timestamps on, so keep 2h applied for this run too).

### 15.1 Result: responder poll every 16 steps [OBS]

Image `40449d0b…ccf98` (clean, poll every 16), host without 2h timestamps (the 72.0 s
baseline had 2h on; its effect is expected to be well under 1%, unmeasured). Passed, 335 chunks.
Download 62.192 s (11,138,817 ticks, 33,283 ticks per chunk = 3.33M cycles = 186 ms),
1.03 KiB/s.

| | download | KiB/s | per chunk |
|---|---|---|---|
| native | 40.7 s | 1.57 | 121 ms |
| OpenPRoT clean, poll every step (`533a67b4`) | 72.0 s | 0.89 | 215 ms |
| OpenPRoT clean, poll every 16 steps | **62.2 s** | 1.03 | 186 ms |

-9.8 s (-14%), 29 ms per chunk, far more than the 1-2 ms poll timeout suggested: the poll
costs a full IPC call, a server wake on the deadline, the timeout path and the
client-side handling, each at about 58 cycles per instruction. Gap to native: 1.53x
(65 ms per chunk). Remaining review candidates: server loop overhead per wake (item 7),
copies and zero-fills in the I3C client and sender (item 6), remaining IPC round trips
(item 4).

## 16. Scope: event-driven FD (2026-10-23)

Scoped in `event-driven-fd-scope.md`: a server-side "wait for any of these handles" receive
and a combined send + wait-for-(response | UA command) op, so the FD neither polls the responder
nor needs two IPC round trips per chunk. Honest gain: versus poll-every-16 the poll removal
itself is only about 2 ms per chunk (it fixes the cancel latency); the real prize is one fewer
IPC round trip per chunk, estimated 15-30 ms (a guess). Phase 0 (measure the cost of one IPC
round trip on the board) decides whether the full change is worth about 2-3 working days.

### 16.1 Event-driven FD implemented (2026-10-23) [OBS, host/emulator only]

Poll evidence: in `host-ts-native.log` and `host-ts-openprot.log`, between the first and last request
IBI the host wrote only the response fragments (334 x (3 writes of 69 B + 1 of 14 B) = 1002 + 334),
nothing else, so the UA sent no command during the download: every responder poll found nothing.
(The interim fix polled every 16 requests; its timeout stayed 1 ms. It was not a longer wait.)

Implemented on OpenPRoT working tree (uncommitted, on top of `41280405`):
- wire: `MctpOp::RecvAny = 7`, `encode_recv_any` / `decode_recv_any` (up to 4 handles, priority =
  list order), `encode_recv_any_response` (the response header carries the handle that fired).
- server: `Server::try_recv_any`, `register_recv_any` (all-or-nothing, BadArgument / AddrInUse /
  NoSpace, group keyed by the first handle), `update` resolves a group once (message beats timeout;
  the earlier handle wins and the other message stays queued), `RecvResult::MessageOn`;
  `cancel_recv` and `unbind` drop a whole group. `dispatch_mctp_op` handles `RecvAny`.
- client: `MctpClient::recv_any` (default returns BadArgument), `IpcMctpClient::recv_any`,
  `StackReqChannel::recv_or_command` returning `ReqEvent::{Response, Command}`.
- pldm: `MctpPldmTransport::send_request_serving` (send, then wait for the response or a command on
  the listener; a command is framed like `respond_once`, passed to the handler and answered, then the
  wait resumes); `FirmwareDevice` Phase 1 uses it and no longer polls between requests. The
  `set_responder_poll_interval` stopgap and the 16-step setting in `pldm_fd.rs` are removed. Phase 2
  (idle blocking, or a 1 ms poll when in initiator mode with nothing to send) is unchanged.
- Limitation: the requester's client issues the `RecvAny` with the listener handle allocated by the
  responder's client; both must talk to the same server (true in all FD apps here: one
  `handle::MCTP` channel). A single transport for both roles would be cleaner but changes
  `FirmwareDevice::init`.

Tests: host suites for `services/mctp` and `services/pldm` all pass, with 8 new server tests (group
resolve, priority, timeout, message-vs-timeout, all-or-nothing, no room, cancel/unbind), 4 new
dispatch tests, 6 new `send_request_serving` tests (command while waiting answered then response,
declined command, response beats a queued command, handler error, timeout) plus wire and `Stack`
tests. Emulator: 12 of 12 pass (`pldm_i3c`, `pldm_throughput`, `pldm_throughput_colo`, `mctp_i3c`,
`i3c_throughput`, `i3c_throughput_colo` and their no_panics tests).

Clean FPGA image (no trace, no calibration): sha256
`43d20ec3dfcd5126c1932b692ca6609d711339d63eaf97f37ac2ddfdf8b3124d`
(`phase2/openprot_pldm_throughput_colo_eventdriven_clean.bin`). Not yet run on the board. Compare
with the poll-every-16 run (62.2 s) and the poll-every-step run (72.0 s); expect about the poll16
result (the poll was already 15/16 gone), i.e. no speed change, but a CancelUpdate is now answered at
once.

### 16.2 Architecture diagrams

`/home/antrocha/work/open-prot/event-driven-fd-architecture.md`: before / interim (poll every 16) /
after (`recv_any`) sequence diagrams, how the server group receive works, the FD step before and
after, what did not change, and the open limitations. Scope and design rationale:
`event-driven-fd-scope.md`. Pending: board run of the event-driven image (`43d20ec3…124d`).

### 16.3 Board result: event-driven FD (image `43d20ec3…124d`) [OBS]

Passed, 335 chunks, exit 0. Download 65.028 s (11,646,809 ticks, 34,773 ticks per chunk = 3.48M cycles
= 194 ms), 0.98 KiB/s. Host without 2h, same as the poll16 run.

| | download | KiB/s | per chunk |
|---|---|---|---|
| native | 40.7 s | 1.57 | 121 ms |
| poll every step (`533a67b4`) | 72.0 s | 0.89 | 215 ms |
| poll every 16 steps | 62.2 s | 1.03 | 186 ms |
| **event-driven (`recv_any`)** | **65.0 s** | 0.98 | 194 ms |

- vs the original poll-every-step: -7.0 s (-9.7%), with immediate response to a UA command.
- vs poll-every-16: +2.8 s (+4.5%), about 8 ms per chunk (about 150k cycles, about 2,500 instructions at
  58 cycles each). I expected about +2 ms (the 1/16 of the poll that poll16 still paid); the
  measurement is higher. Run-to-run noise on this setup has been about 0.03%, so the difference is real.
- [INF, untested] likely cause: the group receive does more work per server pass than a plain
  receive. `update()` runs about 11 times per chunk, and for a group it scans the table for groups,
  collects and sorts the members, and looks up two cookies (response and listener) instead of one;
  `recv_any` also encodes and decodes a handle list. Candidate fixes: keep group members already
  ordered so no sort or scan is needed, skip the group pass when no group is registered, and check the
  listener cookie only when the request handle has nothing. Needs a measurement (trace around
  `drive_pending`) before changing anything.

## 17. IPC round-trip timing (Phase 0 of event-driven-fd-scope.md) (2026-10-23)

Question: what does one IPC round trip to the MCTP server cost on the board? It decides how much
merging send and receive into one call (option C) could save, and how much of the remaining device
time is IPC.

Measurement (temporary, not committed; patch in `phase2/ipc-timing-pldm_fd.patch`): in `pldm_fd.rs`,
after `set_eid`, 64 calls of `stack().get_eid()` (the cheapest server operation: client encode,
`channel_transact`, one pass of the server loop, reply), each timed with `debug_clock_now` (ticks are
core cycles). The same code with two clock reads back to back gives the clock-syscall floor, which
is subtracted (each sample includes one clock syscall). Logs
`ipc timing: get_eid x64 cycles min/med/max/avg` and `ipc timing: clock-read floor ...` once at boot.
Caveats: it runs before the update agent starts talking (probably, not checked), so the I3C path is
idle; it measures the cheapest op, so a send (fragmenting, I3C TX) or a deferred recv costs more.

Image (clean HEAD `528517cc` code, no trace; only this measurement): sha256
`073f6fe7a3f67854af1e75e6ef64f87fc25978053c81b46688d004f597253c7b`
(`phase2/openprot_pldm_throughput_colo_ipctiming.bin`). Not yet run.
Decision rule from the scope doc: about 5 ms or less per round trip, stop at correctness and look
elsewhere; 20 ms or more, merging send and receive is worth doing.

### 17.1 Result: one IPC round trip costs about 21 ms [OBS]

```
ipc timing: get_eid x64 cycles min=385295 med=385369 max=388967 avg=386063
ipc timing: clock-read floor cycles min=6318 med=6325 max=6328 avg=6324
```

- Floor (one clock-read syscall): 6,325 cycles = 0.35 ms (about 110 instructions at 58 cycles each).
- `get_eid` round trip minus the floor: 385,369 - 6,325 = 379,044 cycles = **21.2 ms** (about 6,500
  instructions at 58 cycles each). Very stable (min 385,295, max 388,967, 64 samples).
- It is the cheapest server operation, measured at boot with the I3C path idle; real `send` and
  deferred `recv` calls cost at least this much.
- An IPC round trip is about 60x a plain syscall: client encode, `channel_transact`, a switch to the
  server process, one pass of the server loop, `channel_respond`, a switch back, decode.

Consequences:
- Per chunk the FD makes 2 IPC round trips (send, wait): at least 42 ms of the 194 ms chunk (22%),
  and about 58% of the 73 ms gap to native (121 ms per chunk).
- Decision rule from the scope doc: 20 ms or more -> merging send and receive into one call
  (option C, `SendRecvAny`) is worth doing: saves at least one round trip, about 21 ms per chunk
  (about 11% of the chunk, about 29% of the gap to native), about 4 s of the 65 s download. The
  per-IPC fixed cost, not the amount of work inside it, is the lever.
- Bigger levers: removing the IPC boundary altogether (run the FD in the colocated server process,
  like the i3c/mctp merge) would save both round trips, about 42 ms; or making the IPC path itself
  cheaper (6,500 instructions for a round trip is a lot: process-switch cost, PMP reprogramming,
  copies, WaitGroup). The same fixed cost probably applies to each of the 5 IRQ wakes per chunk in
  the colo process (unmeasured).

## 18. Send and wait in one IPC call (option C, `SendRecvAny`) (2026-10-23)

Motivation: one IPC round trip costs about 21 ms (17.1); the FD made two per chunk (send, then wait).
Implemented (OpenPRoT working tree, uncommitted, on top of `528517cc` + `e744c2fc`):
- wire: `MctpOp::SendRecvAny = 8`, `encode_send_recv_any` / `decode_send_recv_any` (header carries the
  request handle, payload carries timeout, listener handle and the message).
- server: `Server::check_recv_any` (validation split out of `register_recv_any`); dispatch sends on the
  request handle, then `try_recv_any([req, listener])`, else `register_recv_any` and defers under the
  request handle. The wait is validated before the send, so nothing is sent if it cannot be
  registered. A waiting command is returned at once after the send; the request stays outstanding.
- client: `MctpClient::send_recv_any` (default BadArgument), `IpcMctpClient` implementation,
  `StackReqChannel::send_recv_or_command` (shares `event_from` with `recv_or_command`).
- pldm: `send_request_serving` sends in its first wait and uses `recv_or_command` afterwards. Whatever
  arrives is received into `cmd_buf[1..]` (the combined call reads the request out of `buf`), a
  response is copied into `buf[1..]` once (about 200 B per chunk).
- tests: 35 of 35 under `//services/...`; emulator 12 of 12 (with the trace stashed).

Image (clean, no trace, no calibration): sha256
`193a3930da3c93bc26455d8a6b6e28235d93a88af7677f1ec963be6e47120088`
(`phase2/openprot_pldm_throughput_colo_sendrecvany_clean.bin`). Not yet run on the board.
Expectation: about 21 ms per chunk saved (the measured round-trip floor), so about 58 s for the
download against 65.0 s (event-driven, two round trips); the group-receive overhead (about 8 ms per
chunk, 16.3) is still in. An estimate until measured.

### 18.1 Board result: send + wait in one call did NOT help [OBS]

Image `193a3930…0088`. Passed. Download 66.038 s (11,827,637 ticks, 35,345 per chunk = 3.53M cycles =
197 ms), 0.97 KiB/s.

| | download | per chunk |
|---|---|---|
| native | 40.7 s | 121 ms |
| poll every step | 72.0 s | 215 ms |
| poll every 16 | 62.2 s | 186 ms |
| event-driven (2 IPC per chunk) | 65.0 s | 194 ms |
| **event-driven + send/wait in one call (1 IPC per chunk)** | **66.0 s** | 197 ms |

My estimate was about 58 s (one round trip fewer = 21 ms per chunk). It came out 1.0 s slower
(about 3 ms per chunk; run-to-run noise on this setup is about 0.03%, so real).

Why the estimate was wrong [INF]: I counted every IPC as chunk time. Only IPCs on the critical path
count. After the FD sends a request, the device is idle for about 98 ms while the host takes the
IBI, the UA builds the response and the host starts writing (14.1, 13.2). The FD's wait IPC runs
inside that idle window, so removing it saves nothing; what the merge removes is off the critical
path. What is on it, between the last response fragment and the next IBI: the callback that handles the last fragments (the last two fragments usually arrive together; see 20.2, the EOM fragment itself is not special)
callback (about 27 ms), the server-to-FD wake that delivers the reassembled message (about 30 ms,
13.2 row F), the FD's callbacks, and the FD-to-server send IPC (about 26 ms, row A). Those still
exist in the merged version, and the merged call is slightly more work per chunk (bigger request,
one response copy, handle list handling), hence the small loss.

Decision: do not keep option C. It is saved as `phase2/option-c-sendrecvany.patch` (uncommitted
`services/` diff at that time) in case it is wanted later.

What the critical path says about the next lever: the two hops that cost about 56 ms per chunk (wake
the FD, FD sends) are the FD living in a different process from the MCTP server. Running the FD in
the colocated process (direct calls, no IPC) would remove both, roughly 40-50 ms per chunk
(an estimate; the floor of one round trip is 21 ms and two hops sit on the path). That is the
architecture change measured by the IPC timing, and it trades away the FD's process isolation.

## 19. FD co-located with the MCTP server and the I3C driver (2026-10-23)

Why: 17.1 (one IPC round trip = 21 ms) and 18.1 (the two hops on the critical path, waking the FD
with the reassembled message and the FD's send, cost about 56 ms per chunk and exist only because the
FD is a separate process).

New test target `target/veer/tests/pldm_throughput_fdcolo` (untracked; the 2-process
`pldm_throughput_colo` stays as the A/B baseline): ONE process `fd_colo` holds the FD, the MCTP
server and the I3C driver. The FD is unchanged (`FirmwareDevice::run_terminus` over
`MctpPldmTransport`); its MCTP client is an in-process `MctpClient` (`ColoClient`) that calls the
server directly. There is no kernel channel. One thread does everything, so the client's blocking
`recv` / `recv_any` run the event loop: check the server, wait on the I3C interrupt through the wait
group (the caller's timeout is the deadline), service it (drain frames into the router), check again.
Process RAM is 0x20000 (the sum of the two processes it replaces).

Gotcha found on the way: a wait group is waited with `Signals::READABLE | signals::I3C`, not with the
IRQ signal alone; with the IRQ signal alone the FD never saw a frame (idle timeout at 15 s).

Checks: emulator `pldm_throughput_fdcolo_test` (full 4 KiB PLDM update) passes, `no_panics_test`
passes, image has no static data. Not yet run on the board.
Image (clean): sha256 printed below / `phase2/openprot_pldm_throughput_fdcolo.bin`.
Expectation: removes the FD wake and the FD send IPC from the critical path, roughly 40-50 ms per
chunk (an estimate: two hops, floor 21 ms each); the download would drop from about 65 s towards 50 s.
ffba59bece1b2be8065496b0b8add934a18da3a977a4767946b0a5dd1f702a4e  phase2/openprot_pldm_throughput_fdcolo.bin

### 19.1 Board result: FD co-located (image `ffba59be…2a4e`) [OBS]

Passed, 335 chunks, exit 0. Download 52.172 s (9,344,245 ticks, 27,897 ticks per chunk = 2.79M cycles
= 156 ms), 1.23 KiB/s. Discovery 1.078 s and negotiation 0.719 s are also faster (1.27 s and 0.97 s
before).

| | download | KiB/s | per chunk |
|---|---|---|---|
| native | 40.7 s | 1.57 | 121 ms |
| poll every step, separate FD process (`533a67b4`) | 72.0 s | 0.89 | 215 ms |
| event-driven, separate FD process | 65.0 s | 0.98 | 194 ms |
| **event-driven, FD co-located (this run)** | **52.2 s** | **1.23** | **156 ms** |

- Removing the FD<->MCTP-server process boundary: -12.8 s (-19.7%), 38 ms per chunk. My estimate was
  40-50 ms per chunk (two hops on the critical path, floor 21 ms each): the measurement agrees,
  slightly below. Run-to-run noise on this setup is about 0.03%.
- Gap to native: 1.28x, 35 ms per chunk (it was 3x at the start).
- The price of isolation for this one boundary, on this platform: 20% of the download (about 13 s).
  The earlier i3c<->mctp boundary was measured at about 26% on the full PLDM flow in the emulator
  (`target/veer/tests/ipc_isolation_cost.md`). Caveat: on this platform every instruction costs about
  58 cycles (uncached SRAM, 12.4), which inflates any instruction-count-driven cost.
- The co-located variant gives up the FD's process isolation; it is an experiment, not a
  recommendation. Remaining levers that keep the isolation: make an IPC round trip cheaper (kernel
  path versus the server loop's per-pass overhead, not yet decomposed) and shorten the work on the
  critical path (the work after the last fragment arrives; not yet traced on the co-located build,
  and reassembly itself was shown NOT to be the cost, see 20.2).

## 20. Last-fragment cost: is it logging? (2026-10-25)

> **SUPERSEDED, premise wrong (see 20.2).** The "8x for the last fragment" figure below was taken
> from the CPU-efficiency review without checking it against the traces. In the traces and in an
> isolated benchmark the last fragment costs the same as the middle ones. What section 20 shows that
> still holds: the `log` macros in `mctp-estack` are runtime-disabled (a compare and a load). The
> hypotheses listed here (router completion bookkeeping, flow table, odd-alignment copies) are
> withdrawn. Kept for the record.

Observation (13.2 and the earlier trace): MCTP `inbound` for the last fragment of a message is about
257k cycles against about 31k for a middle fragment (about 8x, about 3,900 extra instructions at 58
cycles each). Reading `mctp-estack` `reassemble.rs` / `lib.rs`: the end-of-message path copies at most
64 B and removes one flow-map entry, which does not explain it.

Hypothesis "log or trace macros on the EOM path (`trace!("message reassembly complete, len {}")`
and others)": **ruled out** [SRC]. `MODULE.bazel` builds `mctp-estack` with the `log` feature, so the
macros are `log::trace!` etc. In `log` 0.4.32 each macro expands to
`if lvl <= STATIC_MAX_LEVEL && lvl <= max_level()` (`macros.rs:123`), and `MAX_LOG_LEVEL_FILTER`
starts at 0 = Off (`lib.rs:458`). Nothing in this tree (pw_log, pw_kernel, our crates) calls
`set_max_level` or `set_logger`, so every macro is a static compare plus one load; the arguments are
never formatted.

Remaining hypotheses (unverified): completion bookkeeping in the `mctp-lib` router above the stack
(matching listeners and requests, retaining the message); flow-table updates; copies at odd alignments
(the 4-byte MCTP header offsets payloads) falling back to byte-wise loops (mostly the later consume
step). Next: an isolated micro-benchmark on the device that feeds synthetic 4-fragment messages
(68, 68, 68, 13 B) into a throwaway MCTP server and into `mctp_estack::Stack` directly, timing each
`inbound` / `receive` and the consuming `try_recv` separately.

### 20.1 Reassembly micro-benchmark (built; emulator-validated, not yet run on the board)

Temporary code in the co-located process (`phase2/fd_colo_reassembly_bench.rs`; the clean
`fd_colo.rs` is restored in the tree): at boot it creates a throwaway MCTP server S (EID 8, drops
outbound) and a second server T (EID 0x0a) that only builds response packets. Per iteration (31):
S sends a request on a request handle (flow and tag allocated), T answers with a 200-byte response
that fragments as 68, 68, 68, 13 byte packets (same as the real chunk), and each `S.inbound(pkt)` and
the consuming `S.try_recv(handle)` are timed with the kernel clock (ticks are core cycles); the clock
syscall floor is measured and reported. One summary line per fragment plus one for the consume.

Emulator run (it validates the logic: 31 of 31 iterations delivered a message, fragments 4, sizes
68 68 68 13; emulator ticks are not FPGA cycles): inbound fragments 1331, 1086, 1086, 1044 (floor
243), consume 364. In isolation the last fragment is NOT more expensive than the middle ones in the
emulator, so the 8x [premise later shown wrong, see 20.2] seen on the board in the real flow may come from the surrounding state (or from
memory effects that the emulator does not model) rather than from reassembly itself. The board run
decides.

Image sha256: `870587113d92050aab706a54531453f566cdf7489fd4be234c92e6f8031315d0`
(`phase2/openprot_pldm_throughput_fdcolo_reassembly_bench.bin`).

### 20.2 Board result of the reassembly benchmark, and a CORRECTION [OBS]

Image `87058711…15d0`, FPGA, isolated from I3C and the interrupt path, 31 iterations (median cycles,
clock-syscall floor 6,428 subtracted):

| step | cycles | ms |
|---|---|---|
| `inbound` fragment 0 (SOM, 68 B) | 31,226 | 1.74 |
| `inbound` fragment 1 (68 B) | 24,331 | 1.36 |
| `inbound` fragment 2 (68 B) | 24,331 | 1.36 |
| `inbound` fragment 3 (EOM, 13 B) | 23,442 | 1.31 |
| `try_recv` (consume the message) | 3,534 | 0.20 |

31 of 31 iterations delivered a message (4 fragments, sizes 68 68 68 13).

**The last fragment is not special.** In isolation the EOM fragment costs the same as the middle
ones (23.4k vs 24.3k cycles). My own traces agree: `inbound` after a 13-byte frame is 29.9k cycles
median, after a 68-byte frame 31.2k (`trace-run.log`, `cpi-run.log`, `ipc-run.log`; n=21 to 63).

**Retraction:** I reported (12.x / chat, also in section 20) that `inbound` for the last fragment was
about 257k cycles, about 8x a middle fragment. That figure came from the CPU-efficiency review's
table and does not match the traces I then checked; I repeated it without verifying it. It is wrong.
The roughly 27 ms "last-fragment callback" in 13.2 is large because that IRQ callback usually
handles the last TWO fragments (frames 3 and 4 arrive together), not because the EOM fragment is
expensive. Hypotheses built on the 8x (router completion bookkeeping, flow-table update, odd-alignment
copies) are withdrawn; section 20's "last fragment" framing should be disregarded.

Per-fragment device cost is uniform: decode about 3.3 ms (frame length dependent), `inbound` about
1.4-1.7 ms, RX drain 2-3 ms, plus the interrupt ack (about 1.6 ms) once per IRQ. Most of it overlaps
the host's write pacing; what matters for the chunk time is the tail after the last fragment arrives.

Next: trace the co-located image (156 ms chunk against native 121 ms) with events at: IRQ entry,
frame decoded, `inbound`, message delivered to the FD, FD callback, next request sent, IBI raised.

## 21. Where things stand (2026-10-25)

Final numbers, 64 KiB PLDM update, FPGA, same host UA, host unpatched except 2b-2f:

| configuration | download | KiB/s | per chunk | vs native |
|---|---|---|---|---|
| native (caliptra-mcu-sw) | 40.7 s | 1.57 | 121 ms | 1.00x |
| OpenPRoT, FD in its own process (event-driven) | 65.0 s | 0.98 | 194 ms | 1.6x |
| OpenPRoT, FD co-located with the MCTP server and the I3C driver | 52.2 s | 1.23 | 156 ms | 1.28x |

Where the gain came from, in the order it was found (each step measured, details in the sections cited):
CRC lookup table (12.5) and removing the 1 KB per-call zero-fill and reusing the request handle (13);
dropping the per-chunk responder poll, first every 16 steps (15.1), then event-driven `RecvAny` (16.3);
removing the FD process boundary (19.1). The last step costs the FD's process isolation.

Measured facts that frame the rest: one IPC round trip is 21 ms (17.1); an instruction costs about
58 cycles (SRAM is uncached, 12.4); the host side of a chunk is identical on both stacks (14.1); the
remaining gap is 35 ms per chunk of device work after the last fragment arrives.

Tried and not kept: send and wait in one IPC call (18.1, no gain: the IPC it removed was off the
critical path).

Retractions and corrections (do not rely on the earlier text):
- "inbound for the last fragment costs 8x a middle one" (about 257k cycles): wrong (20.2).
- "the last-fragment callback is expensive": it handles two fragments; the EOM fragment is not special.
- "a merged send/wait call saves about 21 ms per chunk" (18): wrong, only IPCs on the critical path count.
- "poll every 16 steps": the poll's timeout stayed 1 ms; the frequency changed, not the wait.
- the claim that this change would be about 58 s (send/wait merge) and about 62 s (event-driven):
  estimates that the board did not confirm (66.0 s and 65.0 s).

Open items: trace the co-located image to find the remaining 35 ms; decompose the 21 ms IPC cost
(kernel path versus the server loop's per-pass work, bare-kernel echo benchmark); the group-receive
overhead of the event-driven wait (about 8 ms per chunk vs poll-every-16, 16.3, unmeasured cause);
commit the `pldm_throughput_fdcolo` experiment target (untracked); commit-or-drop the trace
instrumentation (uncommitted in `pldm_throughput_colo`).

## 22. Trace of the co-located image (2026-10-25)

Goal: find the remaining 35 ms per chunk (156 ms vs native 121 ms) on the build we are actually
comparing, instead of inferring it from the separate-process build.

`phase2/fd_colo_trace.rs` (the tree's `fd_colo.rs` is clean): records `(tag, clock ticks, extra)`
into a local buffer (1200 entries, no UART output on the hot path) and prints 4 entries per `rec`
call once it is full. Tags: 1 irq entry, 2 on_interrupt done, 3 frames latched, 4 irq acked,
5 frame decoded, 6 mctp inbound done, 7 irq exit, 10 FD offset query, 11 FD chunk data, 12 send
begin, 13 send end, 14 blocking receive entered, 15 message delivered to the FD, 16 wait on irq,
17 woke. Analysis: `tools/analyze_fdcolo_trace.py LOGFILE` (medians per chunk and one chunk event by
event). Image sha256 `74ee0be295df4bf58a02838aeb207c6405070d46e7f8869a78bd4072ed38d9f6`
(`phase2/openprot_pldm_throughput_fdcolo_trace.bin`); emulator test and no_panics pass, no static data.
Not yet run on the board. Only the early chunks (before the dump starts) are usable: the dump adds
UART time to later ones.

### 22.1 Result: trace of the co-located image on the board [OBS unless marked]

Image `74ee0be2…38d9f6`. Passed (335 chunks, exit 0). 1200 events, 27 chunks analysed (the trace
instrumentation makes a chunk 182 ms; the clean build is 156 ms, so read these as proportions).
`tools/analyze_fdcolo_trace.py fdcolo-trace.log`:

| segment (median) | ms |
|---|---|
| FD query -> send begin | 2.1 |
| **send (MCTP send + I3C TX + IBI raise)** | **19.0** |
| send end -> first response interrupt (IBI done) | 4.7 |
| first response interrupt -> last interrupt entry (host turnaround + writes) | 86.4 |
| **last interrupt entry -> message delivered to the FD** (2 frames in that interrupt) | **42.3** |
| delivered -> FD chunk data callback | 1.0 |
| chunk data -> next query | 0.5 |
| chunk period (instrumented) | 182.0 |

Critical path after the last interrupt begins (delivery 42.3 + FD 1.5 + query to send 2.1 + send
19.0) is about 65 ms in this build.

Findings:
1. **The send costs 19 ms for one small packet** (about 340k cycles, about 5,900 instructions at 58
   cycles each) and is on the critical path (it ends with the IBI). In the separate-process build
   the part of it that I could see (dispatch to the driver) was 1.8 ms, so the bulk is the MCTP send
   path (router, fragmenter, PEC, framing) plus per-call buffers. Review item 6 listed zero-fills
   of about 1 KB per call in `I3cClient` (4 calls) and in `I3cSender::send_vectored` per loop
   iteration, and `mctp-lib` copies the payload into a `MAX_PAYLOAD` buffer; none measured yet.
2. **Gaps of about 15 ms appear at arbitrary points.** 164 gaps of 12 to 17.5 ms outside waits in
   8.7 s of trace (median 14.95 ms), about 5 per chunk and about 75 ms of the 182 ms chunk
   (28% of all trace time; a heuristic classification). They occur between ANY two consecutive
   events, including one where the code is a single register read (`on_interrupt`, normally 0.4 ms,
   13.6 ms three times), so they are not slow code. Their count per chunk is the number of
   interrupts plus 2 (2 interrupts -> 4 gaps, 3 -> 5), and their spacing clusters at about 22.7 ms
   and multiples. [INF] Something takes about 15 ms of CPU time per occurrence while the host is
   active; candidates: kernel interrupt handling (about 15 ms = 270k cycles per interrupt?), or a
   periodic event. It is not present at boot: the boot-time IPC timing (17.1) had 64 samples within
   1% of each other. Cause unknown.
3. Frames 1 and 3 of the response take about 15.8 ms from ack or `inbound` to "decoded", frames 2 and
   4 take 2 to 3 ms: consistent with finding 2 (a gap lands on every other frame), not with decode.
4. The earlier unexplained windows (the 26 ms "IBI-done callback tail", the 15 ms driver send, the
   "FD thread running" explanation in 13.2 / 18.1) may be this same effect. There is no other thread
   in this build, so that attribution was probably wrong.

Next experiments: (a) a gap detector, a spin loop that reads the clock and logs every jump over 1 ms
with its size and spacing, run at boot and during the transfer, to measure the stolen time directly and
see whether it is periodic or interrupt-driven; (b) read the pw_kernel interrupt path for what an
interrupt costs; (c) a micro-benchmark of the MCTP send path (like the reassembly one) to split the 19 ms.

### 22.2 Read of the pw_kernel interrupt and tick path (2026-10-25) [SRC]

Files read (pigweed `pw_kernel`): `arch/riscv/veer_pic.rs`, `exceptions.rs`, `timer.rs`,
`timer/mtime.rs`, `kernel/scheduler/core.rs` (`tick`), and `target/veer/config.rs`.

- External interrupt top half (`veer_pic::interrupt`): read `MEIHAP` (CSR), `set_interrupt_enable(irq,
  false)` and a gateway clear (two PIC MMIO writes), then the registered handler; `LOG_INTERRUPTS` is
  false. `userspace_interrupt_ack` is one PIC write. Nothing here suggests about 270k cycles. The PMP
  reload in `trap_handler` (`exceptions_reload_pmp`) is only enabled for earlgrey, not veer.
- There is a periodic scheduler tick: `SCHEDULER_TICK_HZ = 100` (`config/lib.rs:20`), so the machine
  timer fires every 10 ms of kernel time. Each tick: `set_next_monotonic_tick` (a few MMIO accesses),
  `scheduler::tick` (`process_queue`, then `try_reschedule(Ticked)`). `MTIME_HZ` is configured as 20 MHz
  for the FPGA (`target/veer/config.rs:46`) while the clock the traces show is 17.91 MHz, so the real
  tick period is about 11.2 ms (assuming MTIME runs at the core clock). The 15 ms gaps in 22.1 recur
  about every 22.7 ms, about two tick periods; whether that is a coincidence is not known.
- Not read yet: `scheduler::timer::process_queue`, `try_reschedule` and the context-switch path, and
  the userspace interrupt handler (the object signalling).

What the reading does NOT support: that a tick or an interrupt itself costs 15 ms. The boot-time IPC
timing (17.1; 64 samples of about 21 ms, all within 1%) would have shown it: such a sample spans one or
two ticks. So the 15 ms gaps appear only while the host drives the I3C controller, and nothing read
explains them. Other candidates, untested: contention on the AXI bus that serves the MCU's SRAM
(instruction fetch and data) while the host side polls the I3C controller and the debug FIFO; or
something the I3C core does while a transfer is in flight.

Next: the gap detector (22.1, item a): a spin loop that logs every clock jump over a small
threshold with its size and timestamp, run at boot (no host), while idle in the wait loop, and during
the transfer. It separates tick cost (small, every 11 ms), host-driven theft, and bus contention.

## 23. I3C driver deviations from the native driver: changes (2026-10-25)

Review items from 5.4 and what was done (OpenPRoT working tree, uncommitted):

| # | deviation | decision |
|---|---|---|
| 1, 2 | RX status handling, empty descriptor | already fixed (6b84a35c) |
| 3 | PIC gateway type unverified | patch `third_party/pigweed/veer_pic_gateways.patch` (wired into `MODULE.bazel`): every gateway level-triggered, active high at kernel init, as caliptra-mcu-sw's `pic.rs:99-102`. Applies cleanly; the patched file contains it. No behaviour change if the reset value already matches (believed, not verified). |
| 4 | `ResponseRead` lost when it shares an interrupt with an inbound frame | `on_interrupt` now handles each status bit on its own; on a coincidence only `InboundReady` is returned (runtimes release back-pressure on inbound), so nothing is lost that mattered |
| 5 | `TxDescComplete` not handled; `ResponseRead` derived from `IbiDone` | `ResponseRead` now comes from `TxDescComplete` (the controller consumed the staged response). Arming it costs one extra interrupt per outbound message, so `enable()` does NOT arm it; new `I3cTarget::enable_response_read_events()` (default no-op, `hal/blocking`) arms it, and only `services/i3c/server-runtime` (which gates a second send on it) calls it. The colocated benchmark images keep their interrupt count. |
| 6 | IBI failure ignored | `on_interrupt` re-raises the IBI when `LastIbiStatus` is non-zero, up to 16 times (native: unbounded) |
| 7 | 100-nop delay before the IBI | NOT mirrored: no failure seen, and it would add about 0.3 ms per chunk to the critical path |
| 8 | RX drain differs | no change (ring drain) |
| 9 | `MAX_PAYLOAD` 250 vs FIFO 256 | no change |
| 10 | RX descriptor error bits ignored | no change (native ignores them too) |

Also corrected the `server-runtime` comments that said `ResponseRead` comes from `IbiDone`.

Emulator suites with the first version of the driver (all three interrupts armed in `enable()`): 22 of 22
pass (i3c_service, i3c_backpressure, i3c_irq_preempt, i3c_user_irq, mctp_i3c, pldm_i3c, pldm_throughput,
pldm_throughput_colo, pldm_throughput_fdcolo, i3c_throughput, i3c_throughput_colo and their no_panics
tests). Rerun pending with the final split and the gateway patch.

Board image (co-located FD, clean, with the driver changes and the gateway patch): sha256
`285325540dfc95a7a7c5e3cc6b9a5fc4c78151553cc4ba54feaa0172a1cf62ff`
(`phase2/openprot_pldm_throughput_fdcolo_i3cdrv.bin`). Compare with the 52.172 s run (`ffba59be…2a4e`).

## 24. Gap detector (2026-10-26)

Goal: measure directly whether something takes CPU time away from the MCU (22.1: gaps of about 15 ms,
about five per chunk, count tracking interrupts), and whether it is the scheduler tick, host-driven, or
bus contention.

`phase2/fd_colo_gap.rs` (the tree's `fd_colo.rs` is clean): while the FD waits for the i3c interrupt it
spin-polls instead of sleeping. A pass is one non-blocking `object_wait` (deadline = now) plus a clock
read; its length is the pass time. A pass over 36,000 cycles (about 2 ms) is recorded as (timestamp,
duration); the timer is reset around servicing an interrupt, so only pure spinning counts. Before the FD
starts it spins for 3 s as a baseline (host probably not yet driving the bus; the baseline ends early if
an interrupt arrives). The summary and the recorded gaps are printed once, just before the FD exits.
Emulator run: 1806 passes, no gaps, 27 interrupts (passes and cycle counts there are emulator ticks).
Analysis: `tools/analyze_gaps.py LOGFILE` (size histogram, spacing between gaps).

What to expect: a normal pass is a syscall plus a clock read, about 12k cycles (0.7 ms) on the board
(6.3k per clock read, 17.1). If something takes about 15 ms every 22.7 ms the detector will show gaps of
that size with that spacing; a tick (about 11 ms period) would show as small regular gaps just above the
threshold if it costs more than 2 ms. No gaps at all in the baseline and during the transfer would mean
the earlier gaps were not stolen time but slow code.

Image (clean, with the driver changes and the gateway patch): sha256
`99a59bb2caf2b620e8630ea8faaeb01558621328384f8518bf5cfa4abf3e67af`
(`phase2/openprot_pldm_throughput_fdcolo_gap.bin`). The test run's download time is not comparable (the FD
spins).

### 24.1 Result: the gap detector did not work as designed; it measured the scheduler tick instead [OBS]

Run `gap.log` (image `99a59bb2…67af`): the test FAILED (`exit code: 1`, 0 chunks, `pldm fd: update did not
complete`), so no transfer was observed. The detector's summary line:
`passes 1656 avg 202047 min 105141 max 202223 cycles; gaps over 36000: 1656, irqs 1`.

What went wrong: I assumed `object_wait` with a deadline of "now" returns at once. It does not: every
pass took one scheduler tick, so all 1656 passes were "gaps", and the FD serviced only ONE interrupt in the
whole run (an interrupt that was already pending is not reported by a wait whose deadline has passed, or the
wait never got a chance to see it), so it never answered the update agent. The experiment design was wrong,
and the run says nothing about stolen time during a transfer.

By-products (measured):
- **Scheduler tick period = 202,160 cycles = 11.29 ms** (88.6 Hz; 512 consecutive pass ends are spaced exactly
  202,160 cycles apart). Nominal is `SCHEDULER_TICK_HZ = 100` (10 ms) on a clock configured as 20 MHz while the
  traces show 17.91 MHz, so 11.17 ms was expected; 11.29 ms is measured.
- **A wait with a finite deadline resolves only at a tick boundary** (the kernel arms its timer at a fixed tick,
  not at the earliest deadline; `mtime.rs::set_next_monotonic_tick`). So any timeout (the FD's 15 s idle and
  5 s request timeouts, and the old 1 ms responder poll) has up to 11.3 ms granularity. [INF] This may be part
  of why the 1 ms poll cost about 29 ms per chunk (13/15.1): a poll that times out waits for the next tick.
  An interrupt wake is not tick-bound as far as I know; not verified.

Re-check of 22.1 with the measured period: the 12-17.5 ms gaps have median 14.9 ms = 1.32 ticks, and their
start and end times are NOT locked to the tick grid (flat phase histograms over one tick, 164 gaps).
A slight period mismatch (spacing about 22.7 ms vs 2 ticks = 22.58 ms) could hide a lock, so this does not
rule out a tick connection; it just gives no support for one.

Not yet answered: whether anything steals CPU time while the host is active.

Next (simple and decisive): (a) a pure clock-read spin (no `object_wait`) for 3 s at boot and again after the
transfer, recording jumps over 1 ms: tells whether our thread loses the CPU when idle with no host traffic;
(b) in the trace image, stamp the tick phase at boot (two waits on a past deadline return at tick boundaries)
so the gaps' phase relative to the tick can be tested exactly.

## 25. The bootstrap thread spins: a likely root cause of the "IPC cost" and the gaps (2026-10-26)

Lead from 24.1 (the tick is 11.29 ms and finite-deadline waits resolve at tick boundaries) plus a look at
`pw_kernel`:
- `kernel/lib.rs`: the kernel starts a `bootstrap` thread (priority `DEFAULT_PRIORITY`) that runs
  `target::main()`. Our `target.rs` files do `codegen::start(); loop {}`: after starting the app threads,
  the bootstrap thread **busy-loops forever**.
- `tooling/system_generator/system_config.rs:350`: app threads default to `DEFAULT_PRIORITY`, the same
  priority. The scheduler (`scheduler::tick` -> `try_reschedule(Ticked)`) round-robins threads of equal
  priority at every tick (11.29 ms).
- Pigweed's own reference targets (`target/pw_rp2350/ipc/user/target.rs`, `clock/user/target.rs`) do
  `codegen::start(); loop { kernel::sleep_until(Arch, Instant::MAX) }`: the bootstrap thread sleeps.
  All 12 veer test targets here use `loop {}`.

What a spinning equal-priority thread would do [INF, not yet tested on the board]:
- A woken app thread does not run until the spinning thread's time slice ends: up to one tick (11.3 ms)
  of latency per wake-up.
- While an app thread is runnable it loses the CPU to the spinner for a whole slice at a tick: gaps of
  about one tick (median 14.9 ms = 1.32 ticks), about every second tick (22.6 ms; the trace spacing was
  22.7 ms), about five per chunk (22.1).
- An IPC round trip (client blocks, server runs, client resumes) is two thread switches: 385k cycles =
  1.9 ticks (17.1). That would make the "21 ms IPC cost" mostly scheduling latency, not instructions, and
  the isolation price (19.1: 20%) mostly the spinner, not isolation.
- It would also bear on the "58 cycles per instruction" calibration (12.4): a 3.5M-cycle loop spans 17 ticks.
  [INF] Part of that figure may be lost time slices, not memory speed. Not tested.

Experiment: `target.rs` of `pldm_throughput_fdcolo` and `pldm_throughput_colo` (uncommitted) changed to
`loop { let _ = kernel::sleep_until(Arch, kernel::Instant::MAX); }`. Emulator: `pldm_throughput_fdcolo_test`
and `no_panics_test` pass. Board images (clean, with the driver changes and gateway patch; no trace):
- `phase2/openprot_pldm_throughput_colo_nospin.bin`   (FD in its own process): sha256 `b47b7ab157f21e079e0767555059c0099eac46fe9c25f834591df617d066faba`
- `phase2/openprot_pldm_throughput_fdcolo_nospin.bin` (FD co-located):         sha256 `2f21cb30ca55bc93c0a4c90ac7876d5ba5cd1dfc3c30d02bb701c852762a44b5`
Compare with 65.0 s (separate-process FD, `528517cc`) and 52.2 s (co-located, `ffba59be`).
How to read the result: a large drop in both and a much smaller gap between them confirms this; no change
means the spinner was not the cause.

### 25.1 Board result: co-located FD with a sleeping bootstrap thread [OBS]

Image `2f21cb30…44b5` (`phase2/openprot_pldm_throughput_fdcolo_nospin.bin`; includes the I3C driver changes
and the PIC gateway patch). Passed, 335 chunks, exit 0. Download **37.317 s** (6,683,598 ticks, 19,973 ticks
per chunk = 1.99M cycles = 111 ms), **1.72 KiB/s**.

| configuration | download | KiB/s | per chunk |
|---|---|---|---|
| native (caliptra-mcu-sw) | 40.7 s | 1.57 | 121 ms |
| co-located FD, spinning bootstrap thread | 52.2 s | 1.23 | 156 ms |
| **co-located FD, sleeping bootstrap thread** | **37.3 s** | **1.72** | **111 ms** |

Removing the busy loop took the co-located build from 52.2 s to 37.3 s (-14.9 s, -29%), and it is now 8% FASTER
than native. This supports the hypothesis in `bootstrap-thread-spin.md`: a spinning equal-priority thread was
costing a large part of every chunk.

Caveats: one run; the image also carries the driver changes and the gateway patch (both expected to be
neutral for this flow, not separated); the 2-process (isolated FD) image `b47b7ab1…faba` has not been run, so
the isolation price with the spinner removed is still unknown.

Consequence for earlier conclusions (to be rewritten once the 2-process result is in): the "isolation price"
(19.1: 20%) and the "IPC round trip = 21 ms" (17.1) were largely the spinner; the Slack draft that says the gap
was narrowed by giving up memory isolation should not be posted as written.

### 25.2 Board result: FD in its own process with a sleeping bootstrap thread, and a CORRECTION [OBS]

Image `b47b7ab1…faba` (`phase2/openprot_pldm_throughput_colo_nospin.bin`; includes the driver changes and the
gateway patch). Passed, 335 chunks, exit 0. Download **48.992 s** (8,774,612 ticks, 26,198 ticks per chunk =
2.62M cycles = 146 ms), **1.31 KiB/s**.

| configuration | spinning bootstrap | sleeping bootstrap | spinner cost |
|---|---|---|---|
| FD in its own process | 65.0 s (194 ms/chunk) | **49.0 s** (146 ms/chunk) | -16.0 s (-25%), 48 ms/chunk |
| FD co-located | 52.2 s (156 ms/chunk) | **37.3 s** (111 ms/chunk) | -14.9 s (-29%), 45 ms/chunk |
| cost of the FD process boundary | 12.8 s, 38 ms/chunk | **11.7 s, 35 ms/chunk** | |

Native: 40.7 s (121 ms/chunk). With the spinner removed: co-located 0.92x native, FD in its own process 1.20x.

**Correction of 25.1 and `bootstrap-thread-spin.md` (sections 3.3, 3.4, 6):** the claim that the "isolation price"
(20%) and the IPC round trip were "largely the spinner" is REFUTED. The spinner costs about the same
absolute time (45 to 48 ms per chunk) in both configurations, and the FD process boundary still costs about 35
ms per chunk without it (38 ms with it). So the boundary price is real and independent of the spinner. What
the spinner removal explains is the roughly 45 to 48 ms per chunk common to both builds (lost slices and
wake-up latency).

Still open: the cost of one IPC round trip and the "58 cycles per instruction" were measured with the
spinner running. Whether the 21 ms (385k cycles) round trip contained spinner slices is not known; the 35 ms per
chunk for two boundary hops suggests about 17 ms per hop, so most of the IPC cost is likely real. Re-measure
both with the sleeping bootstrap thread.

Also open: with the spinner gone, do the event-driven wait (about 1,000 lines) and the poll changes still earn
their place? Test: the poll-every-step code (`533a67b4`) with the spinner fix, against 49.0 s.
