<!-- Licensed under the Apache-2.0 license -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# mctp server

Platform-independent MCTP server core (`openprot_mctp_server`). Ported from
the Hubris `mctp-server`, with OS-specific IPC (`sys_reply`, `Leased`,
`RecvMessage`) stripped out so the router/handle/timeout logic can be
host-tested without a kernel.

```
 target/<soc>/tests/.../main.rs          ← platform loop (Pigweed IPC, I2C, timers)
        │  wraps this crate via services/mctp/server-runtime
        ▼
 server/        Server<S: Sender, OUTSTANDING> — no_std, no OS deps
        │  dispatch.rs: decode wire::MctpRequestHeader → call Server → encode response
        ▼
 server.rs       mctp-lib::Router — handle allocation, routing, timeouts
        │  Sender (from mctp-lib)
        ▼
 transport-i2c/ (or any Sender impl)
```

`server`/`dispatch` never touch IPC, syscalls, or a clock source directly —
the platform layer drives everything through `now_millis` and callback
closures. See [`services/mctp/server-runtime`](../server-runtime) for the
Pigweed kernel loop that wraps this crate.

## Modules

| Module | Role |
|--------|------|
| `server.rs` (`Server<S, OUTSTANDING>`) | Wraps `mctp_lib::Router`. `req`/`listener`/`set_eid`/`get_eid`/`send` mirror the wire ops directly. `try_recv` + `register_recv` implement non-blocking-then-deferred receive; `update(now_millis, recv_buf)` drains the router and resolves any outstanding receives that are ready or timed out. |
| `dispatch.rs` (`dispatch_mctp_op`, `drive_pending`) | Decodes a raw IPC request (`wire::MctpRequestHeader`) and calls the matching `Server` method, encoding the reply in place. `dispatch_mctp_op` returns `DispatchOutcome::Reply(n)` when a response is ready immediately, or `DispatchOutcome::Pending { handle }` when a `Recv` had to be deferred. `drive_pending` is called later (on inbound transport data or a timer tick) to resolve deferred receives; it calls `on_ready(handle, response_bytes)` once per handle that became ready. |

## Key invariants

- **No heap, no OS primitives.** `#![no_std]`; `MAX_LISTENERS`/`MAX_REQUESTS`/
  `MAX_OUTSTANDING` (`ServerConfig`) bound all allocation via `heapless`.
- **One response buffer, reused per call.** `drive_pending`'s `on_ready`
  callback must send `response_bytes` before returning — the buffer is
  overwritten for the next ready handle.
- **Protocol-agnostic.** Any protocol that implements `mctp::MsgType`
  framing (SPDM, PLDM, echo) goes through the same `Server`/`dispatch` path;
  nothing here is aware of SPDM or PLDM specifically.

## Test Coverage

```bash
bazelisk test //services/mctp/server:mctp_server_dispatch_test
bazelisk test //services/mctp/server:mctp_server_unit_test
bazelisk test //services/mctp/server:mctp_server_integration_test
bazelisk test //services/mctp/server:mctp_server_echo_test
```

All four are host-buildable (no kernel/QEMU); `tests/common/mod.rs` provides
shared fixtures (`BufferSender`, `DirectClient`, etc.). See
[`services/mctp/README.md`](../README.md) for the full MCTP host test suite.
