# pldm_i3c: PLDM firmware update over MCTP over I3C

A pass/fail correctness test: the PLDM firmware device (`FirmwareDevice` from
`//services/pldm`) updates one 512 byte component, driven by a PLDM update agent
on the host, over MCTP over I3C on the Caliptra VeeR emulator and on the VCK190.

## What runs

Three processes in one image (as in [`../mctp_i3c`](../mctp_i3c), with the PLDM
firmware device in place of the echo app):

| Process | Role |
|---|---|
| `i3c_server` | Owns the I3C controller and its interrupt (`//services/i3c/server-runtime`) |
| `mctp_server` | The MCTP router over `I3cSender`/`MctpI3cReceiver` (`//services/mctp/transport-i3c`) |
| `pldm_fd` | `FirmwareDevice::run_terminus`, from [`../pldm_fd`](../pldm_fd) |

`i3c_server.rs` and `mctp_server.rs` are the `mctp_i3c` sources, shared.

The host side ([`../pldm_ua`](../pldm_ua)) is a PLDM update agent on
`caliptra_mcu_hw_model` and `BufferedStream`: `RequestUpdate`,
`PassComponentTable`, `UpdateComponent`, then it answers the FD's
`RequestFirmwareData` chunks, `TransferComplete`, `VerifyComplete` and
`ApplyComplete`, and finally sends `ActivateFirmware`. The FD then has nothing
left to do, waits out its idle timeout (2 s here) and exits PASS.

The FD caps its chunks at 180 bytes, so each chunk is one MCTP fragment and the
image takes three. The throughput images raise the cap to span fragments.

## Running it

```bash
# Emulator
bazel test //target/veer/tests/pldm_i3c:pldm_i3c_test_emulator \
    --nocache_test_results --test_output=streamed --test_arg=--nocapture

# VCK190 (needs the board's SSH key loaded)
bazel test --config=fpga //target/veer/tests/pldm_i3c:pldm_i3c_test_fpga \
    --nocache_test_results --test_output=streamed --test_arg=--nocapture \
    --test_env=VCK190_FPGA_HOST=runner@<board-ip>
```

`:no_panics_test` checks the image has no reachable panic.
