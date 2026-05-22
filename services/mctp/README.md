# MCTP Service

This directory contains the OpenPRoT MCTP service stack: API traits and wire protocol, IPC client, server core, and serial transport abstractions.

## Directory Layout

- `api/`:
  - Crate: `openprot_mctp_api`
  - Defines shared MCTP types, error model, traits, and wire format.
  - Includes the high-level `Stack` facade in `api/src/stack.rs`.
- `client-ipc/`:
  - Crate: `openprot_mctp_client_ipc`
  - Pigweed IPC implementation of the `MctpClient` trait.
- `server/`:
  - Crate: `openprot_mctp_server`
  - Platform-independent server core and dispatch logic.
- `transport-serial/`:
  - Crate: `openprot_mctp_transport_serial`
  - Common serial transport traits plus direct and IPC-focused modules.
- `host/`:
  - Reserved for host-side tooling/examples (currently empty).

## Architecture

```text
Application
  |
  | uses high-level traits
  v
openprot_mctp_api::Stack<C>
  |
  | C: MctpClient
  v
openprot_mctp_client_ipc::IpcMctpClient (or another MctpClient impl)
  |
  | IPC request/response wire protocol
  v
openprot_mctp_server::dispatch + openprot_mctp_server::Server
  |
  | Sender trait binding
  v
Transport (serial/I2C/etc.)
```

## Crate Responsibilities

### `api` (`//services/mctp/api:mctp_api`)
- Public API contracts:
  - `MctpClient`
  - `MctpListener`
  - `MctpReqChannel`
  - `MctpRespChannel`
- Common data and errors:
  - `Handle`
  - `RecvMetadata`
  - `MctpError`, `ResponseCode`
- Wire encoding/decoding for IPC operations in `api/src/wire.rs`.
- High-level facade in `api/src/stack.rs`:
  - `Stack<C>`
  - `StackListener`
  - `StackReqChannel`
  - `StackRespChannel`

### `client-ipc` (`//services/mctp/client-ipc:mctp_client_ipc`)
- Implements `MctpClient` as `IpcMctpClient` over Pigweed IPC.
- Uses Pigweed `channel_transact` for synchronous IPC.
- Encodes requests and decodes responses with `openprot_mctp_api::wire`.

### `server` (`//services/mctp/server:mctp_server_lib`)
- Owns listener/request allocation, routing, recv timeout handling, and send path integration.
- Bridges IPC wire operations through `dispatch`.
- Generic over `mctp_lib::Sender` for transport binding.

### `transport-serial` (`//services/mctp/transport-serial:mctp_transport_serial`)
- Defines reusable serial transport traits and serial error model.
- Organizes platform style split:
  - `direct` for direct HAL access
  - `ipc` for microkernel/IPC-managed serial access

## Build and Test

From workspace root:

```sh
# Embedded-safe MCTP production crates
bazelisk build --config=virt_ast10x0 //services/mctp:mctp_embedded_all

# Host-side MCTP tests
bazelisk test //services/mctp:mctp_host_tests
```

Avoid using a broad wildcard with embedded config (for example, `//services/mctp/...`) because that can include host `rust_test` targets that require `std`/`test`.

## Integration Notes

- All crates are `no_std`.
- The server crate is intentionally platform-agnostic; target-specific runtime wiring belongs under `target/` packages.
- Application code should prefer programming against trait interfaces from `openprot_mctp_api`, not IPC details.
- `Stack<C>` is the main entry point for application-facing usage.

## Related Documentation

- API details: `services/mctp/api/README.md`
- Server details and tests: `services/mctp/server/README.md`
