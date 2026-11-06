# pldm_throughput_fdcolo — the FD, MCTP server and I3C driver in one process

`pldm_throughput_colo` merges the i3c and mctp servers but keeps the PLDM
FirmwareDevice in its own process, talking to the MCTP server over a kernel
channel. This variant also moves the FD into the same process: its MCTP client
(`ColoClient`) calls the in-process `Server` directly. There is no channel and no
IPC. The FD code is unchanged (`FirmwareDevice::run_terminus` over
`MctpPldmTransport`).

One thread does everything, so the client's blocking calls run the event loop:
`recv` / `recv_any` check the server and, while nothing has arrived, wait on the
I3C interrupt (the caller's timeout is the deadline), service it, and check again.

Why: on the FPGA one IPC round trip costs about 21 ms (the cheapest operation),
and two hops sit on the critical path of every chunk (waking the FD with the
reassembled message, and the FD's send). See `fpga-test-debug.md` sections 17-18.

Tradeoff: the process boundary was fault and privilege isolation between the FD,
the MCTP stack and the I3C driver. This is an experiment to price that isolation,
not a recommendation to merge.

```bash
bazel test //target/veer/tests/pldm_throughput_fdcolo:pldm_throughput_fdcolo_test
bazel build --config=k_veer --//target/veer:target_type=fpga \
  //target/veer/tests/pldm_throughput_fdcolo:pldm_throughput_fdcolo_fpga_runner
```
