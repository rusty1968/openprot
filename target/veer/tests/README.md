# VeeR on-target tests

On-target tests for the Caliptra VeeR-EL2 target (`rv32imc`, `pw_kernel`), run on
the Caliptra MCU emulator. They exercise the target-agnostic `drivers/` and
`services/` crates with the *real* kernel, IPC channels, interrupts, and I3C
peripheral — the counterpart to the host tests that live next to each service and
run `dispatch()`/`LoopbackTransport` without a kernel.

The same encoders/decoders run in both places; these tests add what the host
tests cannot cover: kernel scheduling, cross-process IPC, interrupt delivery, and
the real `CaliptraI3cTarget` MMIO driver.

## Two harness shapes

**Self-contained** (`caliptra_test`) — the firmware decides pass/fail itself and
writes `TEST_RESULT:PASS` / `TEST_RESULT:FAIL` to the UART; the harness scans the
UART for the sentinel. No host driver. Because the sentinel is UART (not
semihosting), the *same* image passes under QEMU and on real hardware.

**Host-driven** (`rust_test`) — a host binary (the test) launches the emulator via
the test's `*_runner`, connects to the emulator's I3C TCP socket, and plays the
I3C **controller** (and, for PLDM, the Update Agent): it drives private
writes/reads and asserts on the exchange. The firmware signals completion by
exiting; the host observes it.

Every test also has a `no_panics_test` (the pw_kernel panic detector over the
built image).

## The tests

| Directory | Harness | What it covers |
|---|---|---|
| `interrupts` | self-contained | External interrupt delivery on VeeR (MEIVT redirect table). |
| `channel_stress` | self-contained | Regression guard for a sustained-`channel_transact` kernel/emulator crash (resolved by the caliptra uprev). See its `BUG_REPORT.md`. |
| `i3c_host` | host library | Shared host-side I3C socket harness (wire protocol + PEC) used by the host-driven tests below; not a firmware test. Has a small `i3c_host_unit_test`. |
| `i3c_service` | host-driven | `//services/i3c` client↔server over real IPC — the on-target proof of the typed `I3cClient` calls. |
| `i3c_backpressure` | host-driven | Outbound back-pressure: three responses staged in a row, each `Send` held until the controller reads the prior one. |
| `i3c_user_irq` | host-driven | A single inbound private write delivered to userspace through the I3C interrupt path. |
| `i3c_irq_preempt` | host-driven | I3C userspace-IRQ delivery under preemption. |
| `mctp_i3c` | host-driven | MCTP echo over the full `i3c + mctp` stack (`//services/mctp/echo` over `IpcMctpClient`). |
| `pldm_i3c` | host-driven | A full one-component PLDM firmware update (`RequestUpdate → PassComponentTable → UpdateComponent → download → verify → apply`) over MCTP over I3C, host playing the Update Agent. |
| `i3c_throughput` | host-driven | Throughput **benchmark**: the sink app receives a fixed byte total over MCTP/I3C and logs a rate measured against the emulator `mtime` clock. Output is a number, not just pass/fail. |

The i3c/mctp/pldm tests stack on each other: `i3c_service` → `mctp_i3c` →
`pldm_i3c` add one layer each, all over the same three-process topology
(`i3c_server`, `mctp_server`, app). See the per-service READMEs
(`services/i3c`, `services/mctp`, `services/pldm`) for the seams they exercise.

## Running

These targets are tagged `emulator`, so `./pw ci` and wildcard builds skip them;
name them explicitly. No `--config` is needed — the host-driven tests launch the
emulator themselves.

```bash
bazel test //target/veer/tests/pldm_i3c:pldm_i3c_test
bazel test //target/veer/tests/mctp_i3c:mctp_i3c_test
bazel test //target/veer/tests/channel_stress:channel_stress_test
```

Constraints and tips:

- **Exclusive.** The host-driven I3C tests share a hardcoded emulator socket port
  (`--i3c-port=65534` in `caliptra_runner.py`), so they are tagged `exclusive`
  and do not run in parallel with each other.
- **Memory.** Building the emulator images is RAM-heavy and can OOM a dev box;
  throttle with `--jobs=2`.
- **Reliability.** To check a test is not flaky, run it repeatedly and disable the
  retry mask so a flake is not hidden:

  ```bash
  bazel test //target/veer/tests/pldm_i3c:pldm_i3c_test \
      --runs_per_test=10 --flaky_test_attempts=1 --jobs=2
  ```

- **Debugging a failure.** Read the per-run `test.log` under
  `bazel-out/.../testlogs/.../<name>_test/run_N_of_M/test.log`; it interleaves the
  host's `I3C HOST TRACE` lines with the firmware's `pw_log` output and any
  kernel exception frame.

## Throughput benchmark (`i3c_throughput`)

`i3c_throughput` is a benchmark, not a pass/fail gate: its value is the rate the
firmware logs. The firmware times the transfer against the emulator's `mtime`
counter (RV timer at `0x21000000+0xe4`, 1 MHz on the emulator — the same clock
`//target/veer/syscall_latency` uses), since the Caliptra emulator has no
built-in throughput instrumentation.

The number is **end-to-end request/response throughput** — the host's
private-read/write round-trip between messages is part of the emulated elapsed
time — so use it for *relative* comparison (vary message size / chunk fill, the
inbound ring depth, or multi-fragment reassembly and compare), not as an absolute
transport ceiling.

Passing tests capture firmware output, so surface the rate with `--nocapture`:

```bash
bazel test //target/veer/tests/i3c_throughput:i3c_throughput_test \
    --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
    2>&1 | grep -E 'THROUGHPUT|drove [0-9]+ message'
# [INF] THROUGHPUT: 8200 bytes in 6659688 ticks (1231 B/s)
```
