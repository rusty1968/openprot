# pldm_throughput_colo: PLDM download throughput with the i3c and MCTP servers merged

[`../pldm_throughput`](../pldm_throughput) with the `i3c_server` and `mctp_server`
processes **merged into one** (`mctp_i3c_colo`, from
[`../mctp_i3c_colo`](../mctp_i3c_colo)), which removes the `mctp <-> i3c` IPC
boundary. The PLDM firmware device stays a separate, isolated process, reached
over the `mctp` channel. The FD, the host update agent and the image are the
`pldm_throughput` ones (their sources are shared), so the only difference between
the two measurements is that removed boundary.

## What the boundary costs

Every fragment crossing between the MCTP stack and the I3C driver was a
`channel_transact`: a cross-process copy, a syscall and context switches. Merging
the two replaces it with a function call into the same `i3c_server::dispatch`.

Emulator, 4 KiB, 960 byte chunks, host wall clock (single runs, so indicative):

| | `pldm_throughput` (three processes) | this (colocated) |
|---|---:|---:|
| download | 1.7 to 2.1 KB/s (1727, 1744, 2056 B/s) | about 3.6 KB/s (3593 B/s) |

That is roughly 1.7 to 2.1 times as fast. The figure is the host's wall clock, so it
is diluted by the host's own round trip; the emulator's speed also differs from the
board's. Compare runs on the same interface only. The FPGA figure is not measured
yet.

## The tradeoff

The boundary was fault and privilege isolation between the I3C driver and the MCTP
stack, which a Root of Trust wants. This image prices that isolation on a full PLDM
download. It is an experiment, not a recommendation to merge the two.

## How it stays correct in one thread

In the two-process image the i3c server's own loop services the controller's
interrupt, so a `Send` that is `Busy` (the controller has not read the previous
fragment) is released by that other process. With both in one thread nothing else
would, so the MCTP sender's wait hook services the target itself
(`i3c_server_runtime::TargetService`): it sleeps, polls the target's status, and
releases the staged response when the controller has read it. It leaves the
interrupt unacknowledged if that latched inbound frames, so the main loop sees it
and forwards them. See [`../mctp_i3c_colo`](../mctp_i3c_colo).

The first version of this image failed because the FD told the host it was ready
before it had registered its listener, and the faster transport delivered the first
command into that gap. The FD now reports ready on `FdEvent::Listening`.

## Running it

Tagged `manual`; see [`../pldm_throughput`](../pldm_throughput) for the output
format and the options.

```bash
# Emulator
bazel test //target/veer/tests/pldm_throughput_colo:pldm_throughput_colo_test_emulator \
    --nocache_test_results --test_output=all --test_arg=--nocapture \
    2>&1 | grep PLDM_THROUGHPUT

# VCK190 (needs the board's SSH key loaded)
bazel test --config=fpga //target/veer/tests/pldm_throughput_colo:pldm_throughput_colo_test_fpga \
    --nocache_test_results --test_output=streamed --test_arg=--nocapture \
    --test_env=VCK190_FPGA_HOST=runner@<board-ip>
```
