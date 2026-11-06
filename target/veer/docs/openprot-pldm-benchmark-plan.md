# Plan: PLDM firmware-update benchmark, native stack vs OpenPRoT stack (FPGA, apples-to-apples)

Written 2026-10-20. Nothing in this plan has been implemented yet.

Sources read for this plan:
- OpenPRoT `target/veer/tests/pldm_throughput_colo/` (`README.md`, `BUILD.bazel`, `target.rs`, `pldm_fd.rs`, `mctp_i3c_colo.rs`, `host_test.rs`) in `~/work/pldm-i3c/openprot` (HEAD `782e972e`, "make pldm_throughput_colo build for the FPGA target").
- caliptra-mcu-sw `run_hw_benchmark` and the benchmark tests in `tests/integration/src/test_firmware_update.rs`, as of Amit's commit `6032ae49` (branch `amit-smoke-test` on `rot-beast`).
- `pldm-fw-update-benchmark-report.md` (same directory as this file).

Evidence labels: **[SRC]** read in source, **[CHECK]** not yet verified.

## 1. Goal

Compare the caliptra-mcu-sw native stack (Tock runtime, PLDM FD in userspace) with the OpenPRoT stack (pw_kernel, `openprot-pldm-service`) on the same FPGA, with the same host update agent (UA), payload, and measurement. Only the device-side stack differs.

## 2. Approach

Reuse the `run_hw_benchmark` harness unchanged. Add a sibling test, `test_openprot_pldm_benchmark_64k`, in `tests/integration` that:

- boots the OpenPRoT `pldm_throughput_colo` image as `custom_mcu_runtime` (the smoke test already proved this path works: `test_openprot_fpga_smoke` passed);
- runs the same host `PldmDaemon` with the same 64 KiB synthetic package and the same `PLDM_BENCH` instrumentation;
- reports the same `download` row (first `RequestFirmwareData` served to `TransferComplete`).

Because the host side is shared, these are identical for both stacks: the MCTP packetizer (64 B packets), the I3C forwarder (5 ms sleep per write), the tick source (FPGA `cycle_count` / 100, about 17.91 MHz), and the wall clock.

## 3. Gaps between the two stacks

Items 1 to 3 were the likely blockers. Item 7, found in step 2b, blocks everything else and
comes first.

1. **Readiness milestone.** [SRC] The host UA waits for the `FIRMWARE_PLDM_READY` boot milestone (bit 12 of the MCI flow status; read at `hw/model/src/model_fpga_realtime.rs:126`, waited on in `common/testing/src/mctp_transport.rs:75`) before its first send. The native runtime sets it. OpenPRoT's FD never does, so the UA would wait for its timeout. Options: set the milestone from `pldm_fd` (needs MCI access in the system config), or bypass the wait in the new test.
2. **Device identity.** [SRC] The UA's package carries a device UUID descriptor (`get_device_uuid()`, `test_firmware_update.rs:187`) and expects matching firmware parameters. The native FD returns them from its download context (`pldm_fdops.rs`). OpenPRoT's `MockFdOps` returns 0 descriptors and default parameters. Mirror the native behavior.
3. **I3C signalling on the board.** [CHECK] The FPGA harness services IBIs and private reads. The colo README says the host test was written against the emulator's TCP socket and that no board path exists yet. Whether OpenPRoT's I3C target (`CaliptraI3cTarget`) raises IBIs the way the harness expects is not verified. This is the biggest unknown, so check it first.
4. **Hard-coded size.** [SRC] `pldm_fd.rs` has `IMAGE_SIZE = 4096` and an in-RAM buffer of that size. A 64 KiB run needs the size from `UpdateComponent`, and the data discarded after counting. This matches the native `bench-skip-validation` (staging writes skipped).
5. **Addresses and EIDs.** [CHECK] OpenPRoT constants: `FD_EID = 8`, `UA_EID = 0x0a`, remote I3C address `0x0a`. Harness: `create_socket(EndpointId(0x08), EndpointId(0))`, `fd_tid = 0x01`. They are compile-time constants in the image. Reconcile and rebuild.
6. **Image variant.** [CHECK] Confirm the `.bin` was built with `--//target/veer:target_type=fpga` and linked for this FPGA's MCU memory map (inspect the ELF segments), not the emulator variant.

7. **Kernel crash on the first I3C interrupt (found in step 2b, 2026-10-20).** [OBS, cause INF]
   The image crashes when the first host command raises the I3C interrupt: illegal instruction
   (`mcause 2`) on `csrw 0xbca, a0` (VeeR MEICPCT) inside `trap_handler`. The write comes from
   Pigweed `pw_kernel/arch/riscv/veer_pic.rs::interrupt()`. caliptra-mcu-sw's native driver
   dropped the same write in `335579894` (#291) when it got I3C interrupts working on the FPGA;
   the emulator accepts the write, which is why it only shows on the board. Details in
   `fpga-test-debug.md`. Fix: a patch to the Pigweed dependency
   (`third_party/pigweed/veer_no_meicpct.patch`, wired in `MODULE.bazel`), then rebuild the
   FPGA image.

Also note: OpenPRoT's FD caps chunks at `FD_XFER_CAP = 960`, but `get_xfer_size` takes the minimum with the UA's advertised 196 B, so chunks should be 196 B as in the native run. [SRC, expected]

## 4. Phases

0. **Kernel interrupt fix (new, first):** apply the Pigweed patch for item 7, rebuild the FPGA
   image, rerun the 2b test. The crash must be gone before anything else can be tested.
1. **OpenPRoT-side changes** (branch of `~/work/pldm-i3c/openprot`): readiness milestone, device identity, size from `UpdateComponent`, EIDs and addresses, quiet logging during transfer. Build the FPGA image.
2. **Boot check:** drop the `.bin` into `tests/integration/src/` and run a stub test (like `test_openprot_fpga_smoke`) that confirms boot, the readiness milestone, and discovery. This answers item 3 cheaply.
3. **Add the test:** a copy of `run_hw_benchmark` using the OpenPRoT image, 64 KiB payload, same asserts.
4. **Compare:** run both tests several times. Report wall-clock B/s first and ticks second. The implied clock (ticks x 100 / wall) must read about 17.91 MHz, and every run must stay under the 32-bit counter wrap (about 239.8 s).

## 5. Fairness checklist

- Same bitstream (`a2cd177`, hw 2.1) and same board (VCK190, CI image).
- Same ROM and Caliptra firmware (the same `all-fw.zip`).
- 196 B chunks, one outstanding request, 4 MCTP packets of 64 B per chunk.
- Device-side validation and staging skipped on both stacks.
- Same measured interval and the same UA summary code.
- Check `mcu_rom_prebuilt.bin` on the board against the bundle's ROM hash before each session (a stale cached ROM caused the earlier `OTP not idle` failure).
- Keep device logging comparable (native prints over UART, OpenPRoT over `pw_log`).

## 6. Risks and open questions

- Do I3C IBIs work as the harness expects (item 3)? Check this before any other work.
- Can `pldm_fd` write the MCI milestone register, or do we bypass the readiness wait in the test?
- Does `FirmwareDevice::run_terminus` expect behavior (SetTID, MCTP tag ownership and the TO bit) that the shared `MctpUtil` UA handles differently from the hand-rolled frames in the OpenPRoT `host_test.rs`?
- The colo image deliberately removes one IPC boundary (I3C and MCTP in one process). Decide whether to benchmark the colo layout, the 3-process `pldm_throughput`, or both, and say so when reporting.
- The 15 s idle timeout in `pldm_fd.rs` only affects the tail after `TransferComplete`, so it does not change the measured interval.

## 7. Commands (from this session, for reference)

Build and run on the FPGA from `rot-beast`, branch `amit-smoke-test`:

```
cargo xtask-fpga fpga build-test --target-host runner@<board-ip>
cargo xtask-fpga fpga test --target-host runner@<board-ip> --test-output \
  --test-filter='package(caliptra-mcu-tests-integration) and test(<test_name>)'
```

Native baseline: `test_firmware_update_benchmark_64k` (reproduced on this setup: 335 chunks, about 40.4 s, about 1.58 KiB/s, implied clock 17.91 MHz).

OpenPRoT FPGA image (from the colo README):

```
bazel build --config=k_veer --//target/veer:target_type=fpga \
    //target/veer/tests/pldm_throughput_colo:pldm_throughput_colo_fpga_runner
```

## 8. Status log

- 2026-10-20, step 2a (boot-only test): passed. The FPGA image boots and the FD reaches
  `waiting for update agent`. Gap 6 closed: the `bazel-bin` image has its entry point at
  `0xa8c00000` (FPGA MCU SRAM base); the `k8-fastbuild-ST-...` one is the emulator build.
- 2026-10-20, step 2b (host UA against the image): the kernel crashes on the first I3C
  interrupt (item 7). No discovery responses reached the UA.
- 2026-10-21: Pigweed patch prepared and wired into `MODULE.bazel` (uncommitted, in
  `~/work/pldm-i3c/openprot`). It applies cleanly to the cached Pigweed copy
  (`patch --dry-run`). Not yet built or run on the board.
- 2026-10-21: DSP0233 framing fix committed in OpenPRoT and confirmed on the FPGA (host
  frames decode). Harness patches 2c/2d (strict EIDs) and 2e (packet sequence) let the UA
  and FD exchange PLDM discovery over I3C. FD identity/size changes (gaps 2 and 4) built
  (image 130,812 B), not yet run on the board. Details in `fpga-test-debug.md` section 4.
- 2026-10-22: FD identity and size changes work on the board: UA completes discovery and
  negotiation and the FD requests the first chunk. The chunk response loses fragments 3
  and 4 on the device (2 of 4 arrive, deterministic). Compared the native and OpenPRoT
  I3C drivers (10 deviations, see `fpga-test-debug.md` section 5); register-dump and
  host-write diagnostics are next.
- 2026-10-22: Root cause of the lost chunk-response fragments found with a register dump:
  the TTI `RxDescStat` bit drops while a descriptor is still queued, stranding the last
  frame (host sends all four, hardware holds the fourth). Driver fix (also check the RX
  descriptor queue depth) built as diag build 6, not yet run. See `fpga-test-debug.md`
  sections 5.7 to 5.10.

- 2026-10-22: Diag build 6 reached 19 chunks, then MCU reset (fatal 0x10) from the ROM-armed WDT1 (~44.7 s). Fix: disable WDT1 in `target/veer/entry.rs` (fpga). Built, not yet run on the board. Details in fpga-test-debug.md section 6.
- 2026-10-22: Full 64 KiB transfer passes on the FPGA (335 chunks, download 147 s, 0.43 KiB/s vs native 40.4 s / 1.58 KiB/s). Not yet a clean number (diagnostics still on). See fpga-test-debug.md section 7.
- 2026-10-22: Run with diagnostics reverted but patch 2g still applied (see fpga-test-debug.md 9.2): download 122.2 s, 0.52 KiB/s (native about 40.4 s, 1.58 KiB/s), about 3x slower. Diagnostics were not the main cost. See fpga-test-debug.md section 9.
- 2026-10-22: Clean run (diagnostics and 2g removed): download 122.2 s, 0.52 KiB/s vs native about 40.4 s, 1.58 KiB/s. 2g had no measurable effect. See fpga-test-debug.md 9.3.
- 2026-10-23: CRC table + client-ipc zero-fill removal + request-handle reuse: download 148.0 -> 127.2 -> 88.2 s (instrumented image); native about 40.4 s. See fpga-test-debug.md sections 12-13.
- 2026-10-23: Native benchmark re-run with the patched host: download 40.536 s (baseline about 40.4 s), so the host patches do not skew the comparison. See fpga-test-debug.md 13.4.
- 2026-10-23: Clean OpenPRoT image (533a67b4): download 72.0 s vs native 40.7 s (1.77x). Host timeline identical on both stacks; the whole gap (about 95 ms per chunk) is device processing between the last fragment and the next IBI. See fpga-test-debug.md 14.1.
- 2026-10-23: Responder poll every 16 steps: download 72.0 -> 62.2 s (native 40.7 s), 1.53x native. See fpga-test-debug.md 15.1.
- 2026-10-23: Event-driven FD (RecvAny group receive, send_request_serving) implemented and host/emulator tested (12/12); board run pending. Diagrams: event-driven-fd-architecture.md; notes 16-16.2.
- 2026-10-23: Event-driven FD on the board: download 65.0 s (poll every step 72.0 s, poll every 16 62.2 s, native 40.7 s). 9.7% faster than the original, 4.5% slower than poll16; likely extra group-receive work per server pass. See fpga-test-debug.md 16.3.
- 2026-10-23: IPC timing on the board: one round trip (get_eid) = 385k cycles = 21.2 ms (clock syscall floor 0.35 ms). The FD makes 2 per chunk (about 42 ms of the 194 ms chunk). See fpga-test-debug.md 17.1.
- 2026-10-23: Send+wait in one IPC call: download 66.0 s (event-driven 65.0 s): no gain, the removed IPC was off the critical path (device idle waiting for the host). Not kept. See fpga-test-debug.md 18.1.
- 2026-10-23: FD co-located with MCTP server + I3C driver: download 52.2 s (separate FD process 65.0 s, native 40.7 s), 1.28x native. Isolation price for the FD boundary about 20% on this platform. See fpga-test-debug.md 19-19.1.
- 2026-10-25: Reassembly micro-benchmark on the board: every fragment inbound is about 23-31k cycles, the last fragment is not special. Retracted the '8x last fragment' claim (the review's 257k figure was wrong). Notes corrected: fpga-test-debug.md 20, 20.2, 21; chunk-timing-diagram.md; cpu-efficiency-review.md row annotated.
- 2026-10-25: Trace of the co-located image on the board: send = 19 ms (critical path), and about five 15 ms gaps per chunk (count tracks interrupts), about 75 ms of the instrumented 182 ms chunk; cause unknown. See fpga-test-debug.md 22-22.1.
- 2026-10-26: Gap detector run failed (object_wait with a past deadline waits for the next scheduler tick, every pass = 202,160 cycles = 11.29 ms; 0 chunks). By-product: scheduler tick period measured = 11.29 ms, finite-deadline waits are tick-granular. See fpga-test-debug.md 24.1.
- 2026-10-26: Found that the kernel bootstrap thread busy-loops (loop {} in target.rs) at the same priority as the app threads; tick = 11.29 ms. Likely cause of the 21 ms IPC cost and the 15 ms gaps. Test images built with sleep_until instead. See fpga-test-debug.md 25.
- 2026-10-26: Co-located FD with a sleeping bootstrap thread: download 37.3 s (was 52.2 s; native 40.7 s). The busy-looping bootstrap thread was the dominant cost. 2-process image still to run. See fpga-test-debug.md 25.1, bootstrap-thread-spin.md.
- 2026-10-26: FD in its own process with a sleeping bootstrap thread: 49.0 s (was 65.0 s). The FD process boundary still costs about 35 ms per chunk (11.7 s) with or without the spinner, so the isolation price is real. See fpga-test-debug.md 25.2, bootstrap-thread-spin.md section 8.
