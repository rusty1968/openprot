# MCTP Service

This directory contains the OpenPRoT MCTP service stack: API traits and wire protocol, IPC client, server core, and serial transport abstractions.

## Directory Layout

- `api/`:
  - Crate: `openprot_mctp_api`
  - Defines shared MCTP types, error model, traits, and wire format.
  - Includes the high-level `Stack` facade in `api/src/stack.rs`.
- `client/`:
  - Crate: `openprot_mctp_client`
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
openprot_mctp_client::IpcMctpClient (or another MctpClient impl)
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

### `client` (`//services/mctp/client:mctp_client`)
- Implements `MctpClient` as `IpcMctpClient`.
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
# Build core crates
bazel build //services/mctp/api:mctp_api \
            //services/mctp/client:mctp_client \
            //services/mctp/server:mctp_server_lib \
            //services/mctp/transport-serial:mctp_transport_serial

# API unit tests
bazel test //services/mctp/api:mctp_api_test

# Server tests
bazel test //services/mctp/server:mctp_server_test \
           //services/mctp/server:mctp_server_echo_test \
           //services/mctp/server:mctp_server_dispatch_test \
           //services/mctp/server:mctp_server_unit_test \
           //services/mctp/server:mctp_server_integration_test
```

## Integration Notes

- All crates are `no_std`.
- The server crate is intentionally platform-agnostic; target-specific runtime wiring belongs under `target/` packages.
- Application code should prefer programming against trait interfaces from `openprot_mctp_api`, not IPC details.
- `Stack<C>` is the main entry point for application-facing usage.

## Related Documentation

- API details: `services/mctp/api/README.md`
- Server details and tests: `services/mctp/server/README.md`
