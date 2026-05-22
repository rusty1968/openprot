# A demo task that echoes MCTP messages.

This demo configures the MCTP stack for EID 8 and listens for MCTP message type `1` (PLDM).
Received messages are echoed through the response channel.

## Run the demo

From `userspace_runtime`:

```bash
bazel test --config=virt_ast10x0 //target/ast10x0/tests/mctp_echo:mctp_echo_test --test_output=streamed --test_timeout=10
```

This test package contains:
- `mctp_server_boot`: server-side smoke app
- `mctp_echo_client`: Stack API echo loop
- `mctp_shutdown_app`: clean test termination app
