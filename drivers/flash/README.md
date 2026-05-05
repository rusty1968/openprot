# Flash Driver Model

This document describes the architecture of the layered flash userspace driver
under drivers/flash and how it integrates with platform server bindings.

## 1. Layer Overview

```
┌──────────────────────────────────────────────────────────┐
│  Application / Client Task                               │
│  FlashClient  (drivers/flash/client)                     │
│  channel_transact(request) -> response                   │
└────────────────────────┬─────────────────────────────────┘
                         │  Pigweed IPC channel
                         ▼
┌──────────────────────────────────────────────────────────┐
│  Server Binary (platform binding)                         │
│  rust_app wires handles + backend + runtime loop         │
└────────────────────────┬─────────────────────────────────┘
                         │
                         ▼
┌──────────────────────────────────────────────────────────┐
│  Server Library (platform binding)                       │
│  dispatch_request: protocol -> backend translation        │
│  runtime loop: channel_read -> dispatch -> respond        │
└────────────────────────┬─────────────────────────────────┘
                         │  FlashBackend trait
                         ▼
┌──────────────────────────────────────────────────────────┐
│  Platform Backend (platform binding)                      │
│  PlatformFlashBackend : FlashBackend                     │
└────────────────────────┬─────────────────────────────────┘
                         │
                         ▼
┌──────────────────────────────────────────────────────────┐
│  Controller Driver (SMC/FMC or equivalent)               │
│  Raw MMIO or HAL-level flash controller implementation    │
└──────────────────────────────────────────────────────────┘
```

## 2. Crate Map

| Bazel target | Crate | Role |
|---|---|---|
| //drivers/flash/api:flash_api | flash_api | Wire protocol, error/status model, geometry types, backend trait contract |
| //drivers/flash/client:flash_client | flash_client | Userspace IPC facade for read/write/erase/discovery |

The API/client layers are target-agnostic within Pigweed kernel userspace
targets.

## 3. Wire Protocol (flash_api::protocol)

Operations are encoded by FlashOp in FlashRequestHeader (16 bytes,
repr(C, packed), little-endian), with an optional payload up to
MAX_PAYLOAD_SIZE (256 bytes).

| Op | Value | Request shape | Response shape |
|---|---|---|---|
| Exists | 0x01 | header only | value = 0/1 |
| GetCapacity | 0x02 | header only | value = capacity bytes |
| Read | 0x03 | address + length | payload = bytes read, value = byte count |
| Write | 0x04 | address + length + payload | value = byte count |
| Erase | 0x05 | address + length | success/error only |
| GetGeometry | 0x06 | header only | payload = FlashGeometry |

FlashResponseHeader (8 bytes) carries status (FlashError), payload length,
and an op-specific value word.

## 4. Backend Contract (flash_api::backend)

The server-side backend contract is defined by FlashBackend:

- info(route_key) -> FlashInfo
- exists(route_key) -> Result<bool, BackendError>
- read(route_key, address, out) -> Result<usize, BackendError>
- write(route_key, address, data) -> Result<usize, BackendError>
- erase(route_key, address, length) -> Result<(), BackendError>
- enable_interrupts() / disable_interrupts()

Geometry discovery is exposed via FlashGeometryProvider, which can derive a
default geometry from info() or be overridden by backends that need richer
semantics.

## 5. Client Library (flash_client)

FlashClient is a synchronous userspace facade that:

- serializes request headers/payloads into caller-provided request buffers,
- performs channel_transact with optional timeout,
- parses/validates response headers and payload lengths,
- maps transport and wire errors into ClientError.

Current client behavior and constraints:

- no_std, blocking IPC calls.
- single call in flight per client instance (&mut self API).
- per-call data cap is FlashClient::chunk_size() (MAX_PAYLOAD_SIZE).
- explicit support for discovery calls: exists, capacity, geometry.

For detailed usage and method-level behavior, see:

- drivers/flash/api/README.md
- drivers/flash/client/README.md

## 6. Error Surface

Wire-level status is FlashError. Common categories:

- protocol misuse: InvalidOperation, InvalidAddress, InvalidLength
- runtime contention: Busy, Timeout
- media/policy failures: IoError, NotPermitted
- fallback: InternalError

ClientError wraps three classes of failure:

- IpcError(pw_status::Error): transport syscall failure
- ServerError(FlashError): valid response reporting flash-level failure
- InvalidResponse: malformed/truncated response frame
- BufferTooSmall: local request/response buffer constraints

## 7. Integration Model

drivers/flash is split so protocol and client can evolve independently from
platform backend/server bring-up:

- Keep wire schema and trait contract stable in flash_api.
- Keep userspace ergonomics and parsing hardening in flash_client.
- Implement server runtime and concrete backend per target tree.

This separation allows host-side validation of protocol and client logic before
hardware-specific bindings are ready.

## 8. Extension Points

- New backend: implement FlashBackend (and optionally FlashGeometryProvider)
  in a platform crate, then bind server channel handles to route keys.
- New operation: add FlashOp variant, define header/payload semantics, extend
  backend trait and server dispatch.
- New geometry flags semantics: preserve wire compatibility by treating flags
  as opaque at protocol level and documenting interpretation at backend level.

## 9. Testing Focus

Recommended tests by layer:

- flash_api: wire layout, endian correctness, enum/status round-trips,
  short-buffer decode rejection.
- flash_client: response validation paths, timeout behavior, chunk-size checks,
  and retry handling for Busy.
- platform server/backend: operation dispatch, alignment rules, erase granule
  correctness, and hardware fault mapping to FlashError.
