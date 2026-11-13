# pldm_throughput: PLDM firmware-update throughput benchmark

A benchmark, not a gate: it runs a full PLDM firmware update over MCTP over I3C
and reports the **download throughput in bytes per second**. The same flow and
the same three processes as [`../pldm_i3c`](../pldm_i3c) (`i3c_server`,
`mctp_server`, `pldm_fd`), with two changes:

- **Large chunks.** The FD asks for 960 bytes at a time, so each
  `RequestFirmwareData` response spans about five inbound MCTP fragments. The i3c
  server's inbound ring (`RX_RING`, 6) queues them and the FD's MCTP stack
  reassembles a message of about 965 bytes (the MCTP stack's limit is 1032). This
  needs `pldm-common`'s `MAX_TRANSFER_SIZE` raised to 960, which a crate patch in
  `MODULE.bazel` does.
- **A bigger image**, 4 KiB on the emulator and 64 KiB on the VCK190 (the size
  caliptra-mcu-sw's own benchmark uses). `PLDM_BENCH_IMAGE_SIZE` overrides it.

## What it measures

The host's wall clock, from the first `RequestFirmwareData` the FD sends to its
`TransferComplete`, as caliptra-mcu-sw measures its own PLDM transfer speed. So
the figure is dominated by the per-chunk round trip (the host's pacing and the
transport), not the firmware's raw speed.

The packets of one message the update agent writes are paced 5 ms apart on the
emulator and 25 ms on the FPGA, where the controller's receive FIFO holds 256
bytes and the model adds its own delay to every write. That pause is part of the
measured time, so **compare runs on the same interface only**; an emulator number
and a board number are not comparable.

## Running it

The test is tagged `manual` and may run for minutes on the board, so wildcard
builds skip it. Passing tests capture output; `--nocapture` shows the figure:

```bash
# Emulator
bazel test //target/veer/tests/pldm_throughput:pldm_throughput_test_emulator \
    --nocache_test_results --test_output=all --test_arg=--nocapture \
    2>&1 | grep PLDM_THROUGHPUT
# PLDM_THROUGHPUT: 4096 bytes in 2.371 s (1727.6 B/s, 1.7 KB/s), 5 chunks, emulator

# VCK190 (needs the board's SSH key loaded)
bazel test --config=fpga //target/veer/tests/pldm_throughput:pldm_throughput_test_fpga \
    --nocache_test_results --test_output=streamed --test_arg=--nocapture \
    --test_env=VCK190_FPGA_HOST=runner@<board-ip>
```

Nothing asserts on the number, only that the update completed.

## Reference result

Emulator, 4 KiB, 960 byte chunks: about 1.7 KB/s (1727 and 1744 B/s on two runs).
The FPGA figure is not measured yet.
