# openprot-mctp-server

Platform-independent MCTP server core, ported from the Hubris `mctp-server` crate.

## Overview

This crate implements the central MCTP server logic: listener and request handle allocation, inbound message routing, outbound message fragmentation/sending, and timeout management for pending receive calls. It is generic over transport bindings via the `mctp_lib::Sender` trait.

## Key Types

- `Server<S, N>` — the MCTP server, generic over `Sender` (transport) and `N` (config)
- `ServerConfig` — configuration constants (MAX_REQUESTS: 8, MAX_LISTENERS: 8, MAX_OUTSTANDING: 16, MAX_PAYLOAD: 1023)
- `RecvResult` — result from a receive operation

## Modules

- `dispatch` — IPC request dispatcher; decodes wire-protocol requests and calls the corresponding `Server` methods

## Architecture

The server wraps the `mctp-lib` `Router` as its packet processing engine. Inbound packets are fed via `Server::inbound()`, and `Server::update()` drives pending-recv fulfillment. The `dispatch` module bridges IPC wire-protocol messages to server operations.

## Dependencies

- `openprot-mctp-api` — API traits and wire protocol
- `mctp-lib` — `Router`, `Sender`, fragmentation, serial/I2C handlers
- `mctp` — core MCTP types (`Eid`, `MsgType`, `Tag`)
- `heapless` — `no_std` collections

## Building the MCTP Echo System Image

The MCTP echo demo is a 3-application system image for the AST1060-EVB (640KB usable SRAM):

```
Flash (512KB)                              RAM (128KB)
0x00000 ┌─────────────────────┐  0x80000 ┌──────────────────┐
        │   pw_kernel (~128KB) │          │  i2c_server 32KB │
0x20000 ├─────────────────────┤  0x88000 ├──────────────────┤
        │   i2c_server  128KB  │          │ mctp_server 32KB │
0x40000 ├─────────────────────┤  0x90000 ├──────────────────┤
        │   mctp_server 128KB  │          │  mctp_echo  32KB │
0x60000 ├─────────────────────┤  0x98000 ├──────────────────┤
        │   mctp_echo   128KB  │          │   kernel    32KB │
0x80000 └─────────────────────┘  0xA0000 └──────────────────┘
                                          (640KB = 0xA0000)
```

| App | Source | Role |
|-----|--------|------|
| `i2c_server` | `//services/i2c/server` | I2C transport backend |
| `mctp_server` | `//services/mctp/server` | MCTP router + IPC dispatcher |
| `mctp_echo` | `//target/ast1060-evb/mctp/mctp_echo.rs` | Listens for type-1 messages, echoes payload back |

### Build the full system image

```sh
bazel build --config=k_ast1060_evb //target/ast1060-evb/mctp:mctp
```

### Build just the echo binary

```sh
bazel build --config=k_ast1060_evb //target/ast1060-evb/mctp:mctp_echo
```

### Build a UART-bootable image (for physical EVB)

```sh
bazel build --config=k_ast1060_evb //target/ast1060-evb/mctp:mctp_uart
```

The output binary `mctp_uart.bin` can be flashed over the UART boot interface.

### Run the system image test

```sh
bazel test --config=k_ast1060_evb //target/ast1060-evb/mctp:mctp_test
```

> The `mctp_test` rule wraps the system image in a `system_image_test` harness that
> exercises the full kernel + I2C server + MCTP server + echo app stack.

---

## Tests

The `tests/` directory contains integration tests that run on the host (std) with **no I2C transport** — a `BufferSender` mock replaces it entirely.

### Test files

| File | What it tests |
|------|---------------|
| `tests/common/mod.rs` | Shared fixtures: `BufferSender`, `DroppingBufferSender`, `DirectClient`, `DirectListener`, `DirectRespChannel`, `DirectReqChannel` |
| `tests/echo.rs` | Full MCTP echo round-trip via `MctpClient` trait |
| `tests/dispatch.rs` | IPC wire-protocol dispatch: encodes requests, calls `dispatch_mctp_op`, verifies responses. Includes edge cases: malformed request, unknown opcode, `Recv` with no message, `Unbind` |
| `tests/server_unit.rs` | Unit tests for `Server` methods: EID management, handle allocation, `try_recv` before/after `inbound`, oversized payload, timeout via `register_recv` + `update` |
| `tests/integration.rs` | Multi-fragment reassembly, multiple concurrent listeners (no cross-talk), echo via `MctpListener` + `MctpRespChannel` traits, `MctpReqChannel` trait, `drop_handle` mid-flight, response EID/tag threading |

The echo via `MctpListener` + `MctpRespChannel` test (`echo_via_mctplistener_trait`) is the key one: it exercises the exact interface the real echo application uses, with `BufferSender` as the only transport.

### Running with Bazel

```sh
# Run all test targets
bazel test //services/mctp/server:mctp_server_echo_test \
           //services/mctp/server:mctp_server_dispatch_test \
           //services/mctp/server:mctp_server_unit_test \
           //services/mctp/server:mctp_server_integration_test

# Show test output
bazel test //services/mctp/server:mctp_server_unit_test --test_output=all
```

> **Note:** Do not use the `//services/mctp/...` wildcard — it will also pick up
> the `mctp_server` and `mctp_echo` kernel binaries, which require `kernel_config`
> (generated only during a full system image build). Target the test rules directly
> as shown above.
