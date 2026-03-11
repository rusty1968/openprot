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

The `tests/` directory contains two integration tests that run on the host (std):

| File | What it tests |
|------|---------------|
| `tests/echo.rs` | Full MCTP echo round-trip: server A listens, server B sends a request, verifies the echoed response via a mock `BufferSender` transport. |
| `tests/dispatch.rs` | IPC wire-protocol dispatch: encodes a request with `wire::encode_*`, calls `dispatch_mctp_op`, and verifies the decoded response. |

### Running with Bazel (primary)

This is a Bazel project. Use these commands from the workspace root:

```sh
# Run both integration tests via the mctp_server_test rule
bazel test //services/mctp/server:mctp_server_test

# Run alongside the API tests
bazel test //services/mctp/server:mctp_server_test //services/mctp/api:mctp_api_test

# Show test output
bazel test //services/mctp/server:mctp_server_test --test_output=all
```

> **Note:** Do not use the `//services/mctp/...` wildcard — it will also pick up
> the `mctp_server` and `mctp_echo` kernel binaries, which require `kernel_config`
> (generated only during a full system image build). Target the test rule directly
> as shown above.

### Running with Cargo (host-only convenience)

Because the integration tests are `std`-only, they can also be run with Cargo for quick iteration without a full Bazel setup:

```sh
# Run all tests for this crate
cargo test -p openprot-mctp-server

# Run only the echo test
cargo test -p openprot-mctp-server --test echo

# Run only the dispatch test
cargo test -p openprot-mctp-server --test dispatch

# Show stdout from passing tests
cargo test -p openprot-mctp-server -- --nocapture
```
