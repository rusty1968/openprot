# i3c_throughput — MCTP-over-I3C throughput benchmark

A **benchmark**, not a pass/fail gate: its value is the transfer rate the firmware
logs. It measures how fast the `i3c + MCTP` transport moves bytes on the Caliptra
VeeR emulator, so throughput changes (bigger chunks, deeper inbound ring,
multi-fragment reassembly) can be compared before/after with a number.

## What it measures, and how

The Caliptra emulator exposes **no built-in throughput instrument** — only an
emulated monotonic clock. So the rate is derived in firmware: the sink reads the
RV timer `mtime` MMIO at `0x21000000 + 0xe4` (1 MHz on the emulator → 1 tick = 1 µs
of emulated time — the same counter `//target/veer/syscall_latency` uses) before
the first byte and after the last, and reports `bytes * 1e6 / elapsed_ticks`.

Topology is the standard three-process i3c stack (see the parent
[`../README.md`](../README.md)):

```
host (controller) ──TCP socket──▶ i3c_server ──"i3c"──▶ mctp_server ──"mctp"──▶ throughput_sink
```

- **`sink.rs`** — receives `TARGET_BYTES` (8 KiB) as a stream of MCTP messages,
  **acks each one** (a 1-byte response) so the single-frame-at-a-time controller
  never overruns the inbound ring, times the steady-state interval against
  `mtime`, and logs `THROUGHPUT:` / `THROUGHPUT_KBPS:`. The first message is not
  counted (it only starts the clock), so the figure is steady-state.
- **`host_test.rs`** — an *eager* driver: no poll sleeps, strict
  one-message-in-flight pacing (send a message, then poll private reads until a
  **non-empty** ack frame returns, then send the next). Messages larger than the
  single-fragment MTU are fragmented into MCTP packets (SOM/EOM/seq); the inbound
  ring queues the fragments and the MCTP stack reassembles them.

## Running it

Passing tests capture firmware output, so surface the number with `--nocapture`:

```bash
bazel test //target/veer/tests/i3c_throughput:i3c_throughput_test \
    --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
    2>&1 | grep -E 'THROUGHPUT:|fragment\(s\) each'
# driving 240-byte messages (2 i3c fragment(s) each)
# [INF] THROUGHPUT: 8400 bytes in 35 msgs (240 B/msg) in 6789020 ticks (1237 B/s)
```

`--jobs=2` throttles the (RAM-heavy) image build; the test is tagged `emulator`
and `exclusive` (hardcoded emulator i3c socket port), so it is skipped by
`./pw ci`/wildcard builds and never runs in parallel.

### Sweeping message size (no rebuild)

Message size is tunable at runtime via `TPUT_MSG_BYTES` (default 240), so a sweep
across the single-/multi-fragment boundary needs one build:

```bash
for n in 120 240 480 720; do
  echo "=== $n B ==="
  bazel test //target/veer/tests/i3c_throughput:i3c_throughput_test \
    --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
    --test_env=TPUT_MSG_BYTES=$n 2>&1 | grep -E 'THROUGHPUT:'
done
```

Keep the message within the ring depth — 4 fragments ≈ 960 B for the current
`RX_RING = 4` — so no fragment is dropped (a drop logs `i3c inbound ring full`
and breaks reassembly). Values are clamped to `[1, 959]`.

## Interpreting the number

It is **end-to-end request/response throughput**: the firmware sits in `recv`
between messages while the host completes its private-read/write round-trip, and
that latency is part of the emulated elapsed time. Treat it as a **relative**
instrument — hold the host round-trip constant and vary one transport parameter —
**not** as an absolute transport ceiling.

Why it's shaped this way: the `i3c↔mctp` IPC boundary is crossed **once per
fragment**, but the `mctp↔sink` boundary **once per whole message**, and each
message is one host round-trip. So fewer, bigger messages mean fewer crossings and
round-trips per byte. (This benchmark carries no PLDM — the sink is a plain MCTP
app — so it isolates the transport that a PLDM download would otherwise ride on.)

## Reference result

Sweep over ~8.4 KiB (`RX_RING = 4`, caliptra `a8b5eb8c`, pigweed `3ada25a6`):

| `TPUT_MSG_BYTES` | fragments/msg | messages | emulated ticks | throughput |
|---:|---:|---:|---:|---:|
| 120 | 1 | 69 | 9.91 M | 835 B/s |
| 240 | 2 | 35 | 6.79 M | 1237 B/s |
| 480 | 3 | 18 | 5.03 M | 1718 B/s |
| 720 | 4 | 12 | 4.23 M | 2043 B/s |

Throughput rises **2.4×** from 120 B to 720 B messages as the message count falls
69 → 12 — fewer IPC crossings per byte. The 720 B case (4 fragments/message)
reassembles with no ring-full drops, validating the N=4 inbound ring's
multi-fragment path end to end.
