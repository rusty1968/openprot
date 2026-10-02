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
# PLDM_THROUGHPUT: 4096 bytes in 3.300 s (1241.2 B/s, 1.2 KB/s)
```

`--jobs=2` throttles the RAM-heavy image build; the test is tagged `emulator` and
`exclusive` (hardcoded emulator i3c socket port), so `./pw ci`/wildcard builds
skip it and it never runs in parallel.

## Reference result and comparison

| | caliptra-mcu-sw UA (commit `78e0c7b6`) | this test |
|---|---|---|
| metric | UA wall-clock, download B/s | UA wall-clock, download B/s |
| result | 1100 → 1300 B/s | **1241 B/s** (4096 B image) |

The test runs with the throughput levers **applied** — `FD_XFER_CAP = 960`
(~5-fragment chunks) and a 1 ms download poll — reaching **1241 B/s** (5/5
reliable), **within upstream's range**, up from a **475 B/s** single-fragment/
50 ms-poll baseline (~2.6×). The journey, in order of impact:

1. **Multi-fragment chunks (dominant).** Raising `FD_XFER_CAP` from 180 B
   (single-fragment) to 960 B makes each `RequestFirmwareData` span ~5 inbound
   MCTP fragments, cutting a 4096 B download from ~23 round-trips to ~5 — the same
   lever [`../i3c_throughput`](../i3c_throughput) quantifies on the raw transport.
   This needs two limits raised together, which this branch does:
   - **The dependency cap.** `pldm-common`'s `RequestFirmwareDataResponse::encode`
     rejected payloads over a hardcoded `MAX_TRANSFER_SIZE = 512` placeholder
     (`700` → `BufferTooShort`). Raised to 960 via a `crate.annotation` patch in
     `MODULE.bazel` (`third_party/crates_io/pldm_max_transfer.patch`).
     caliptra-mcu-sw instead *derives* this (~1019 B) from the MCTP message size.
   - **The inbound ring.** `services/i3c/server`'s `RX_RING` 4 → 6, so a
     5-fragment chunk's burst fits (the ring must hold a whole message's
     fragments). The 960 B chunk reassembles to a ~965 B MCTP message, within the
     1023 B ceiling.
2. **Download poll cadence (marginal).** Dropping `read_fd_request`'s poll from
   50 ms to 1 ms gave only ~4% — the per-chunk cost is firmware/transport round
   trips, not host polling — so the round-trip *count* (lever 1) is what matters.

Intermediate single-fragment-only steps measured 494 B/s (180→460, 2 fragments,
ring 4) and 932 B/s (512, the old dependency ceiling); the multi-fragment jump to
960 B closed the rest of the gap.

## Related tests

- [`../pldm_i3c`](../pldm_i3c) — the PLDM correctness test (small image, pass/fail).
- [`../i3c_throughput`](../i3c_throughput) — transport-only throughput (no PLDM),
  for isolating the i3c + MCTP pipe.
