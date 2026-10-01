# PLDM download throughput — result and analysis

Measured by `pldm_throughput` (UA-side wall-clock over the download phase, the
same metric caliptra-mcu-sw uses in commit `78e0c7b6`). See
[`README.md`](README.md) for more detail.

## Command

```bash
bazel test //target/veer/tests/pldm_throughput:pldm_throughput_test \
    --runs_per_test=5 --flaky_test_attempts=1 \
    --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
    2>&1 | grep -E 'PLDM_THROUGHPUT|Stats over'
```

- `--test_arg=--nocapture` is required — otherwise cargo hides the firmware's
  `PLDM_THROUGHPUT:` line on a passing test.
- `--nocache_test_results` forces real runs (not a cached result).
- `--flaky_test_attempts=1` stops retries from masking a flake.
- `--jobs=2` throttles the RAM-heavy image build on this box.

## Result (5 runs, 4096 B image)

| run | time | throughput |
|---:|---:|---:|
| 1 | 3.076 s | 1331.6 B/s |
| 2 | 3.092 s | 1324.7 B/s |
| 3 | 3.100 s | 1321.2 B/s |
| 4 | 3.048 s | 1343.8 B/s |
| 5 | 3.168 s | 1292.9 B/s |

**Mean ≈ 1323 B/s** (range 1293–1344, tight), 5/5 reliable.

### vs caliptra-mcu-sw (commit `78e0c7b6`)

| | upstream UA | this test |
|---|---|---|
| metric | UA wall-clock, download B/s | UA wall-clock, download B/s |
| result | 1100 → 1300 B/s | **~1323 B/s** |

At the top of their band — **but both are emulator numbers** (see below).

### How we got here

| config | throughput |
|---|---:|
| baseline (180 B single-fragment, 50 ms poll) | 475 B/s |
| 460 B (2-fragment), 1 ms poll | 906 B/s |
| 512 B (old `pldm-common` ceiling) | 932 B/s |
| 960 B (patched cap + `RX_RING=6`) | **~1323 B/s** |

~2.8× over baseline, all from cutting the number of round-trips / IPC crossings
/ fragments per byte.

## Is this the emulator, or the code?

Both — but they are different claims and must be kept apart.

### The absolute number is largely an emulator artifact

The 1.3 KB/s is **host wall-clock** (3.08 s for 4 KB). Three emulator-side
factors dominate it; none is I3C bus bandwidth:

1. **The emulated CPU is modeled at 1 MHz** (`SYSTEM_CLOCK_HZ = 1_000_000` for the
   emulator target — see `target/veer/config.rs`). Real VeeR runs at hundreds of
   MHz; everything the firmware does is paced to 1 MHz here.
2. **Host execution overhead** — the emulator models the VeeR on the host CPU, so
   emulated cycles cost real host time.
3. **localhost TCP round-trips** — each chunk is several private-read/write hops
   over the emulator's i3c socket.

On real silicon the same firmware would be gated by the I3C bus and
per-transaction overhead, not by any of these. So **1.3 KB/s is not a
real-hardware throughput figure**, and "parity with upstream's 1.3 KB/s" is an
emulator-to-emulator comparison, not a silicon claim.

### What *is* a genuine, hardware-independent code signal

1. **The relative improvements.** 475 → 1323 B/s came entirely from reducing the
   *count* of round-trips / IPC crossings / fragments per byte (bigger chunks,
   the inbound ring, multi-fragment). That is a property of the protocol/transport
   code, and it scaled with round-trip count on two independent benchmarks
   (`pldm_throughput` here and [`../i3c_throughput`](../i3c_throughput)). These
   wins carry to real hardware.
2. **Per-byte CPU cost.** `i3c_throughput` measures in *emulated* time (the
   `mtime` counter, not host wall-clock) and still shows ~800 emulated CPU-cycles
   per byte. That is the firmware genuinely spending ~800 cyc/byte on the
   three-process IPC copies (`SyscallBuffer`), MCTP reassembly, the PLDM state
   machine, and context switches — a code property, not emulator speed, and the
   part that would still cost on silicon.

### How to separate them cleanly

- Quote the **emulated-time** figure (`i3c_throughput`'s `mtime`-based B/s, or
  cycles/byte) when talking about code efficiency, not this wall-clock number.
- Raise the emulator clock config (or run on FPGA / silicon): the wall-clock
  number should move while cycles-per-byte stays fixed — directly proving the
  split.

## Takeaway

Read 1.3 KB/s as "we removed the round-trips that inflated the per-byte cost —
confirmed on two benchmarks — and landed structurally level with upstream on the
same class of emulator." The optimization is real and transfers; the absolute
speed is the emulator's, not the hardware's.
