# Running the PLDM-over-MCTP-over-I3C firmware-update test on the FPGA

Runbook for building an OpenPRoT PLDM image, running it on the VCK190 / Caliptra
FPGA subsystem and reading the result. It is a condensed, current-state view. The
evidence and history behind each step are in the other documents in this directory
(see [Further reading](#further-reading)); several of them were written
mid-investigation, so where they disagree with this page, this page wins.

Setup these steps were verified against: VCK190, Caliptra FPGA CI image, 2.1
bitstream from the `main-2.1` manifest, caliptra-mcu-sw on branch `amit-smoke-test`
(Amit's commit `6032ae49`) on the board host `rot-beast`.

## 0. How it fits together

```
build machine (WSL)                          rot-beast (build/test host)         FPGA board
bazel build  ->  *_image.bin  --scp-->  caliptra-mcu-sw/tests/integration/src/  --> MCU SRAM
                                         cargo xtask-fpga fpga build-test
                                         cargo xtask-fpga fpga test  ---ssh-->   runs the host UA
                                                                                 against the image
```

- The **device** under test is the OpenPRoT image: the VeeR core at 17.91 MHz running
  Pigweed `pw_kernel` with the I3C driver, MCTP server and PLDM firmware device (FD).
- The **host** side is the caliptra-mcu-sw test harness, whose UA (update agent) drives
  the board's I3C controller. The harness is patched for OpenPRoT; see
  [section 3](#3-host-side-prerequisites-on-rot-beast).
- Test: `test_openprot_pldm_benchmark_64k`: a 64 KiB download, 335 chunks of 196 B.

## 1. Image variants

| Bazel target (`//target/veer/tests/...`) | What it is |
|---|---|
| `pldm_throughput_colo:pldm_throughput_colo_fpga_runner` | I3C driver + MCTP server in one process, **FD in its own process** (IPC channel). Keeps FD isolation. |
| `pldm_throughput_fdcolo:pldm_throughput_fdcolo_runner` | FD also co-located with the MCTP server and I3C driver. No FD IPC boundary. |

Reference results (64 KiB download, wall time, clean images; native
caliptra-mcu-sw is about 40.7 s):

| Image | Download |
|---|---|
| `colo`, FD in its own process, sleeping bootstrap thread | 49.0 s |
| `fdcolo`, sleeping bootstrap thread | 37.3 s |

The bootstrap thread must sleep rather than spin (`loop {}` in `entry.rs` costs about 15
s). See `bootstrap-thread-spin.md`.

## 2. Build the image (build machine)

```
cd ~/work/pldm-i3c/openprot
bazel build --config=k_veer --//target/veer:target_type=fpga \
  //target/veer/tests/pldm_throughput_colo:pldm_throughput_colo_fpga_runner
sha256sum bazel-bin/target/veer/tests/pldm_throughput_colo/pldm_throughput_colo_image.bin
```

Notes:
- Use the `bazel-bin/...` image. Its entry point is `0xa8c00000` (FPGA MCU SRAM base).
  The `k8-fastbuild-ST-...` image is the emulator build and will not boot on the board.
- Build from a clean tree and record `git rev-parse HEAD` with the hash. Uncommitted
  diagnostics (trace instrumentation) change timing; do not use such an image for a
  benchmark number.
- The Pigweed patch `third_party/pigweed/veer_no_meicpct.patch` (wired in
  `MODULE.bazel`) is required, or the kernel crashes on the first I3C interrupt.
  See `fpga-test-debug.md` section 1.

## 3. Host-side prerequisites (on rot-beast)

Done once per caliptra-mcu-sw checkout; check them before each session.

1. **Board and checkout:** bootstrap the board and match the checkout to the bitstream.
   Follow `fpga-bootstrap-guide.md` (steps 2 and 3). After every board reboot or power
   cycle, rerun its bootstrap and verify steps.
2. **Bitstream and tree must match:** `amit-smoke-test` is the `main-2.1` line, so use the
   2.1 bitstream. The 2.0 tree and bitstream gave different results.
3. **Submodules:** `hw/caliptra-ss` and (on this branch) `network/net/lwip` must be
   initialized before `build-test`.
4. **Harness patches** (sources in the maintainer's `phase2/` directory; they are
   uncommitted in caliptra-mcu-sw):

   | Patch | Purpose |
   |---|---|
   | `2b-openprot-pldm-benchmark.patch` | adds `test_openprot_pldm_benchmark_64k`; `CPTRA_FORCE_PLDM_READY` override because OpenPRoT does not publish the PLDM-ready milestone |
   | `2c-strict-eids.patch`, `2d-new-req-dest.patch`, `2f-response-eids.patch` | UA/FD EIDs set strictly so MCTP destination/source match |
   | `2e-pkt-seq.patch` | accept any first-packet sequence number |
   | `2g-i3c-write-log.patch` | host write log: diagnostics only, **revert for timing runs** (`git apply -R`) |
   | `2h-host-timestamps.py FILE` | optional host timestamps in `hw/model/src/model_fpga_realtime.rs` |

   Required: 2b to 2f. 2g must not be applied for a benchmark. 2h is optional (well under
   1% effect, unmeasured).
5. **Stale cached MCU ROM:** delete `<cache dir>/mcu_rom_prebuilt.bin` before each session
   (`get_or_compile_rom` only copies it if the file is absent; a stale one fails with
   `[mcu-rom-otp] OTP not idle`, fatal `0x00030001`).
6. **Disk space:** builds with `--separate-runtimes` plus duplicate builder images fill
   `rot-beast`; check `podman images` for untagged old images.

## 4. Copy the image to rot-beast

```
ssh-add
scp ~/work/pldm-i3c/openprot/bazel-bin/target/veer/tests/pldm_throughput_colo/pldm_throughput_colo_image.bin \
    antrocha@rot-beast:work/caliptra-mcu-sw/tests/integration/src/openprot_pldm_throughput_colo.bin
ssh antrocha@rot-beast sha256sum work/caliptra-mcu-sw/tests/integration/src/openprot_pldm_throughput_colo.bin
```

The two hashes must match. The `\` continuation must be the last character on its line.
The destination file name is what the benchmark test loads, so keep it. The notes do not
record a separate name for the `fdcolo` image; it is presumably copied over the same
destination name (not verified; check `test_openprot_pldm_benchmark_64k` in the 2b patch).

## 5. Run (on rot-beast, in `~/work/caliptra-mcu-sw`)

```
cargo xtask-fpga fpga build-test --target-host runner@10.223.163.182

CPTRA_FORCE_PLDM_READY=1 cargo xtask-fpga fpga test --target-host runner@10.223.163.182 \
  --test-output \
  --test-filter='package(caliptra-mcu-tests-integration) and test(test_openprot_pldm_benchmark_64k)' \
  2>&1 | tee run.log
```

If the environment variable does not reach the board-side run, pass it another way.
`build-test` is only needed after the image or host patches change.

Tests that program OTP with the lifecycle controller in Dev state can hang the FPGA
(see `.config/nextest.toml` in caliptra-mcu-sw). For a first check of the board, run a
short test from `fpga-bootstrap-guide.md` section 5.

## 6. Read the result

```
grep -E "chunks:|requested:|avg MCU ticks|fatal error|FPGA MCU ROM|PASS|FAIL" run.log
```

- **Pass:** exit 0, 335 chunks, 65536 B requested, no `fatal error`.
- **Use wall-clock download time**, not ticks: the 32-bit `cycle_count` wraps after about
  239.8 s at 17.91 MHz. Any run longer than that reports wrong ticks.
- Compare against a native run of `test_firmware_update_benchmark_64k` (about 40.4 s,
  1.58 KiB/s) on the same board and host patches. Do not compare against numbers from
  other sessions.

## 7. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Kernel crash on first I3C interrupt | Pigweed MEICPCT patch missing | `fpga-test-debug.md` section 1 |
| `[mcu-rom-otp] OTP not idle` | stale cached MCU ROM on the board | delete `mcu_rom_prebuilt.bin` |
| UA never sees discovery responses | wrong framing / EIDs | DSP0233 framing; patches 2c to 2f (`fpga-test-debug.md` sections 3, 4) |
| Download stalls after chunk 1, or fragments 3 and 4 lost | driver stranded a frame (`RxDescStat` drops while a descriptor is queued) | needs the RX descriptor-queue-depth drain fix (`fpga-test-debug.md` section 5.7) |
| Dies after about 19 chunks, `fatal 0x10` | ROM-armed MCU watchdog | needs the WDT1 disable in `target/veer/entry.rs` (section 6) |
| `Runtime failed to compile` on the board | feature runtime missing from `all-fw.zip` | `fpga build --separate-runtimes` |
| `FPGA is not bootstrapped` | board rebooted | rerun bootstrap |
| Slower than expected | diagnostics or 2g left in | rebuild clean, `git apply -R` 2g |

More host-board problems are in the troubleshooting table of `fpga-bootstrap-guide.md`.

## 8. Further reading

All in this directory:

| Document | Contents |
|---|---|
| `fpga-bootstrap-guide.md` | board bootstrap, build and hardware-test flow for the Caliptra CI image |
| `openprot-pldm-benchmark-plan.md` | plan, gaps between the stacks, fairness checklist, dated status log |
| `fpga-test-debug.md` | full debug and measurement log, with evidence labels |
| `pldm-fw-update-benchmark-report.md` | native-stack benchmark report |
| `cpu-efficiency-review.md` | read-only review of CPU hot spots (suspects, not measurements; annotated in the debug log where later disproved) |
| `chunk-timing-diagram.md` | where the time goes in one chunk |
| `bootstrap-thread-spin.md` | the spinning bootstrap-thread finding |
| `event-driven-fd-architecture.md`, `event-driven-fd-scope.md` | event-driven FD design and scope |

The documents are copied as written during the work and keep their original paths and
file names for patches and logs (`phase2/`, `trace-run.log`, ...), which live outside
this repository.
