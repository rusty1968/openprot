# USART Driver Model

This document describes the architecture of the layered USART userspace driver
under `drivers/usart/` and how it integrates with platform bindings in a
target-agnostic way.

## 1. Layer Overview

```
┌──────────────────────────────────────────────────────────┐
│  Application / Client                                    │
│  UsartClient  (drivers/usart/client)                     │
│  channel_transact(request) → response                    │
└────────────────────────┬─────────────────────────────────┘
                         │  Pigweed IPC channel
                         ▼
┌──────────────────────────────────────────────────────────┐
│  Server Binary                                           │
│    (target/<plat>/tests/usart:usart_server_bin)          │
│  rust_app — wires codegen handles + backend + runtime    │
│  wait_group_add ×N  →  runtime::run                      │
└────────────────────────┬─────────────────────────────────┘
                         │
                         ▼
┌──────────────────────────────────────────────────────────┐
│  Server Library  (drivers/usart/server:usart_server)     │
│  runtime::run — object_wait → channel_read               │
│               → dispatch_request → channel_respond       │
│  dispatch_request — pure protocol→backend translator     │
└────────────────────────┬─────────────────────────────────┘
                         │  UsartBackend trait
                         ▼
┌──────────────────────────────────────────────────────────┐
│  Platform Backend  (target/<plat>/backend/usart)         │
│  PlatformUsartBackend : UsartBackend                     │
│  pub type Backend = PlatformUsartBackend                 │
└────────────────────────┬─────────────────────────────────┘
                         │  embedded-io / embedded-hal-nb
                         ▼
┌──────────────────────────────────────────────────────────┐
│  PAC-level UART driver  (platform peripherals crate)     │
│  Usart — raw MMIO handle over vendor PAC RegisterBlock   │
└──────────────────────────────────────────────────────────┘
```

## 2. Crate Map

| Bazel target | Crate | Role |
|---|---|---|
| `//drivers/usart/api` | `usart_api` | Wire protocol + backend trait contract |
| `//drivers/usart/server:usart_server` | `usart_server` | Dispatch + runtime loop library (platform-binding-agnostic within Pigweed kernel targets) |
| `//drivers/usart/client:usart_client` | `usart_client` | Client facade library (platform-binding-agnostic within Pigweed kernel targets) |
| `//target/ast10x0/tests/usart:usart_server_bin` | binary | Reference platform smoke-test server binding |
| `//target/ast10x0/tests/usart:usart_client_app` | binary | Reference platform smoke-test client binding |
| `//target/ast10x0/backend/usart` | `usart_backend` | Reference platform backend implementation |
| `//target/ast10x0/peripherals` | `<plat>_peripherals` | Reference platform raw MMIO UART driver |

## 3. Wire Protocol  (`usart_api::protocol`)

Operations are identified by a 1-byte opcode in `UsartRequestHeader` (8 bytes, `repr(C, packed)`):

- `flags` low nibble carries protocol version (`0` currently).

| Op | Value | Args |
|---|---|---|
| `Configure` | 0x01 | payload = `{ baud_rate: u32, parity: u8, stop_bits: u8, reserved: u16 }` |
| `Write` | 0x02 | payload bytes (submitted, not guaranteed drained) |
| `Read` | 0x03 | `arg0` = max bytes to read |
| `GetLineStatus` | 0x04 | — |
| `EnableInterrupts` | 0x05 | `arg0` = `IrqMask` bits — **server-internal**, not exposed by `UsartClient` |
| `DisableInterrupts` | 0x06 | `arg0` = `IrqMask` bits — **server-internal**, not exposed by `UsartClient` |
| `TryRead` | 0x07 | `arg0` = max bytes to read (non-blocking backend read with IRQ-assisted completion) |
| `Drain` | 0x08 | wait for TX idle completion (IRQ-assisted) |

`UsartResponseHeader` (4 bytes) carries a `UsartError` status code plus a payload length.
All structures implement `zerocopy` traits for zero-copy serialization.

## 4. Backend Trait  (`usart_api::backend`)

```rust
pub trait UsartBackend {
    fn configure(&mut self, config: UsartConfig) -> Result<(), BackendError>;
    fn write(&mut self, data: &[u8]) -> Result<usize, BackendError>;
    fn read(&mut self, out: &mut [u8]) -> Result<usize, BackendError>;
  fn try_read(&mut self, out: &mut [u8]) -> Result<usize, BackendError>;
    fn line_status(&self) -> Result<LineStatus, BackendError>;
    fn enable_interrupts(&mut self, mask: IrqMask) -> Result<(), BackendError>;
    fn disable_interrupts(&mut self, mask: IrqMask) -> Result<(), BackendError>;
}
```

`BackendError` maps 1-to-1 onto `UsartError` via `From<BackendError> for UsartError`.

## 5. Per-Target Binding

`drivers/usart/` ships only platform-agnostic libraries (`usart_api`,
`usart_server`, `usart_client`). Every binary that names a specific
`system.json5` lives next to that config under the platform's tree. In this
repository, the concrete example binding is `target/ast10x0/tests/usart/`,
which packages a self-contained smoke-test image.

```python
# target/ast10x0/tests/usart/BUILD.bazel  (excerpt)
rust_app(
    name = "usart_server_bin",
    srcs = ["server_main.rs"],
    codegen_crate_name = "app_usart_server",
    system_config = ":system_config",
    deps = [
        "//drivers/usart/server:usart_server",
        "//target/ast10x0/backend/usart:usart_backend_ast10x0",
        "@pigweed//pw_kernel/userspace",
    ],
)

rust_app(
    name = "usart_client_app",
    srcs = ["client_main.rs"],
    codegen_crate_name = "app_usart_client",
    system_config = ":system_config",
    deps = [
        "//drivers/usart/client:usart_client",
        "@pigweed//pw_kernel/userspace",
        "@pigweed//pw_log/rust:pw_log",
    ],
)
```

Each backend target uses `crate_name = "usart_backend"` and exports
`pub type Backend: UsartBackend`, so `server_main.rs` can stay generic across
backends:

```rust
use usart_backend::Backend;
let mut backend = Backend::new();
```

## 6. Server Library (`usart_server`)

Two pieces, both platform-binding-agnostic within Pigweed kernel targets:

- `dispatch_request<B: UsartBackend>(...) -> DispatchOutcome` —
  pure protocol→backend translator. No IPC, no OS dependency. Returns either
  an immediate response length or `Queued` when a `TryRead` request is parked
  pending an RX interrupt.
- `runtime::run<B>(backend, wg, irq, irq_signals) -> !` — the dispatch loop.
  Topology-agnostic: the binary registers each channel with its handle as
  `user_data`, and the loop derives the channel handle from
  `wait_return.user_data` directly.

The binary owns every `wait_group_add` call because only the codegen-aware
binding knows which handles exist.

## 7. System Image

The smoke-test image bundles server + client + kernel:

```
//target/ast10x0/tests/usart:usart  (system_image)
  kernel = :target
  apps   = [:usart_server_bin, :usart_client_app]
  system_config = :system_config   (target/ast10x0/tests/usart/system.json5)
```

Companion test rules:

- `:usart_test` — boots the image under QEMU and reports the semihosting
  exit code (run with `bazel test --config=virt_ast10x0`).
- `:no_panics_test` — host-side panic-path detector over the linked binary.

`system.json5` defines each app's memory layout, the `usart` IPC channel
handle, the UART5 MMIO mapping (`0x7e784000`), and the server thread stack
size. The handle constant `handle::USART` is generated by
`rust_app`/`app_usart_server` codegen from the object named `usart`.

## 8. Extension Points

- **New platform**: create `target/<plat>/tests/usart/` (or a non-test
  binding directory if it's a real deployment) with a `system.json5`,
  `server_main.rs`, and a `rust_app` that depends on
  `//drivers/usart/server:usart_server` plus the platform's backend.
  Nothing under `drivers/usart/` changes.
- **New operation**: add opcode to `UsartOp`, extend `UsartBackend`, add arm
  to `dispatch_request`, update protocol doc.
- **New backend**: implement `UsartBackend` in a `rust_library` with
  `crate_name = "usart_backend"` exporting `pub type Backend`.

