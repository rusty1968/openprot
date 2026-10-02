# pldm_throughput_colo — colocated PLDM download throughput

`pldm_throughput` with the `i3c_server` and `mctp_server` processes **merged into
one** (`mctp_i3c_colo`, the same merged process as
[`../i3c_throughput_colo`](../i3c_throughput_colo)), removing the `mctp <-> i3c`
IPC boundary. The PLDM FirmwareDevice stays a separate, isolated process over the
`mctp` channel. Everything else matches `pldm_throughput`, so the difference is
purely that removed boundary, measured on the full PLDM download.

On the `i3c-mctp-colocated` branch, separate from the main throughput work.

## Result (UA-side wall-clock, 4096 B image, 5 runs)

| | 3-process `pldm_throughput` | this (colocated) |
|---|---:|---:|
| download | ~1323 B/s | **~1670 B/s** (1665–1687, dev 0.1 s) |

**+26%** from removing the i3c↔mctp boundary, 5/5 reliable.

Two notes on the number:
- It is **UA-side wall-clock** (matching caliptra-mcu-sw's metric), so the gain is
  diluted by the host round-trip and the emulator's host-execution speed. The
  emulated-time A/B in [`../i3c_throughput_colo`](../i3c_throughput_colo) isolates
  the boundary better and shows ~35% on the raw transport. ~26% here is the
  end-to-end PLDM share of that.
- Cross-stack comparison to upstream's 1100–1300 B/s is loose (different emulator
  config); the clean signal is the internal colo-vs-3-process A/B.

## Tradeoff

Same as [`../i3c_throughput_colo`](../i3c_throughput_colo): the boundary is
fault/privilege isolation between the I3C driver and the MCTP stack, which a
Root-of-Trust wants. ~26% is the price of that isolation on PLDM download. This is
an experiment to make the number explicit, not a merge.

## Running

```bash
bazel test //target/veer/tests/pldm_throughput_colo:pldm_throughput_colo_test \
    --runs_per_test=5 --flaky_test_attempts=1 \
    --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
    2>&1 | grep -E 'PLDM_THROUGHPUT|Stats over'
```
