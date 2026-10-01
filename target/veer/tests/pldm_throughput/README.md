# pldm_throughput — PLDM firmware-update throughput benchmark

A **benchmark**, not a pass/fail gate: it runs a full PLDM firmware update over
MCTP over I3C on the Caliptra VeeR emulator and reports the **download
throughput in bytes/s**, measured the same way caliptra-mcu-sw measures its own
PLDM transfer speed (commit `78e0c7b6`, "PLDM transfer speedup by reducing async
locking"). It exists so our PLDM-FD download rate can be compared apples-to-apples
with upstream and tracked across transport changes.

## What it measures, and how

It is the same three-process stack and the same flow as
[`../pldm_i3c`](../pldm_i3c) — the real `openprot-pldm-service` `FirmwareDevice`
driven through `RequestUpdate → PassComponentTable → UpdateComponent → download →
verify → apply`, host playing the Update Agent — with two changes:

- **Larger image** (`IMAGE_SIZE = 4096`, vs 512 in the correctness test) so the
  download phase is long enough for a stable number.
- **UA-side timing** in `host_test.rs`: the host starts a wall-clock timer on the
  first `RequestFirmwareData` chunk it serves and, at `TransferComplete`, logs
  `downloaded / elapsed` as B/s and KB/s. This mirrors upstream's UA metric
  (start at first chunk, report bytes/s over the download).

This is a **host wall-clock** measurement of the download phase — deliberately,
to match upstream. It is therefore dominated by the UA round-trip (the host poll
cadence) and the per-chunk transport cost, not pure firmware speed.

## Running it

Passing tests capture output, so surface the number with `--nocapture`:

```bash
bazel test //target/veer/tests/pldm_throughput:pldm_throughput_test \
    --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
    2>&1 | grep -E 'PLDM_THROUGHPUT'
# PLDM_THROUGHPUT: 4096 bytes in 8.624 s (475.0 B/s, 0.5 KB/s)
```

`--jobs=2` throttles the RAM-heavy image build; the test is tagged `emulator` and
`exclusive` (hardcoded emulator i3c socket port), so `./pw ci`/wildcard builds
skip it and it never runs in parallel.

## Reference result and comparison

| | caliptra-mcu-sw UA (commit `78e0c7b6`) | this test (baseline) |
|---|---|---|
| metric | UA wall-clock, download B/s | UA wall-clock, download B/s |
| result | 1100 → 1300 B/s | **475 B/s** (4096 B image) |

Ours is ~2.5–3× slower at the same measurement, for two known, actionable
reasons:

1. **Host poll cadence** — the UA download loop (`read_fd_request`) sleeps 50 ms
   per poll. Upstream's speedup cut its UA-side alarm sleep to one tick; our 50 ms
   is the analogous bottleneck.
2. **Single-fragment 180 B chunks** — a 4096 B image is 23 round-trips. The
   multi-fragment lever (see [`../i3c_throughput`](../i3c_throughput), which
   showed 835→2043 B/s as messages grew) would cut that round-trip count.

## Related tests

- [`../pldm_i3c`](../pldm_i3c) — the PLDM correctness test (small image, pass/fail).
- [`../i3c_throughput`](../i3c_throughput) — transport-only throughput (no PLDM),
  for isolating the i3c + MCTP pipe.
