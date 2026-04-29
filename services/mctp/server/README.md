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

## Target Integration

This package is intentionally generic and provides only reusable core pieces:

- `:mctp_server_lib` (server core)
- host-side tests under `tests/`

Platform runtime packaging (kernel app binary, system config wiring, transport
binding selection) should be defined in target-specific packages under `target/`.
This allows each vendor target (for example AST10x0, Earlgrey, Veer) to compose
its own runtime without introducing target binding into this package.

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
