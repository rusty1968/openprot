# MCTP Server Test Plan (without I2C transport)

Branch: ocp-emea-demo

## Core Insight
`Server<S: Sender, const N>` is generic over `mctp_lib::Sender`.
I2C transport is just one `Sender` impl + a caller of `server.inbound()`.
No mocking framework, feature flags, or cfg(test) shims needed.

---

## File Layout

```
services/mctp/server/tests/
├── common/
│   └── mod.rs         ← shared BufferSender, transfer(), DirectClient
├── echo.rs            ← already exists; refactor to use common/
├── dispatch.rs        ← already exists; add missing cases
├── server_unit.rs     ← NEW: Layer 2 unit tests
└── integration.rs     ← NEW: Layer 4 multi-fragment / concurrency
```

---

## Layer 1 — Shared Test Fixtures (common/mod.rs)

- [ ] Extract `BufferSender<'_>` from echo.rs and dispatch.rs into `tests/common/mod.rs`
- [ ] Add `DroppingBufferSender` (discards writes, always returns Ok) for tests that only care about inbound routing
- [ ] Extract `transfer(from, to)` helper into common
- [ ] Extract `DirectClient<'a, S, N>` into common (wraps &RefCell<Server> as MctpClient)

---

## Layer 2 — Server Unit Tests (server_unit.rs)

- [ ] `req()` + `unbind()` — handle allocation/deallocation
- [ ] `listener()` duplicate msg_type — expect AlreadyBound error
- [ ] `try_recv()` before any `inbound()` — returns None
- [ ] `inbound(raw_pkt)` + `try_recv()` — full routing path (use mctp_lib::fragment::Fragmenter to build raw pkts)
- [ ] `register_recv()` + `update(now + timeout)` — timeout fires RecvResult::TimedOut
- [ ] `set_eid()` / `get_eid()` — EID round-trip
- [ ] `send()` with payload > MAX_PAYLOAD — expect NoSpace error

---

## Layer 3 — Dispatch Unit Tests (dispatch.rs additions)

- [ ] Malformed wire request → BadArgument
- [ ] `MctpOp::Send` via response path (no handle, HAS_EID flag, explicit tag)
- [ ] `MctpOp::Unbind` for never-allocated handle → error
- [ ] `MctpOp::Recv` when no message ready → TimedOut (gap noted in code comment)

---

## Layer 4 — Integration Tests (integration.rs)

- [ ] Multi-fragment roundtrip: set `get_mtu()=64`, send 200-byte payload, verify reassembly
- [ ] Multiple concurrent listeners: two msg_type values, cross-deliver, verify no cross-talk
- [ ] Response-without-handle: verify tag & EID threading through echo
- [ ] Interleaved requests from two senders: tag collision avoidance

---

## Layer 5 — MctpClient Trait Tests (via DirectClient in echo.rs)

- [ ] `MctpListener::recv()` — called after inbound, returns payload
- [ ] `MctpReqChannel::send()` + `recv()` — full request-response cycle
- [ ] `drop_handle` mid-flight — verify outstanding entry is cleared

---

## Out of Scope (belongs in other crates)

| Concern | Owner crate |
|---|---|
| MCTP-over-I2C framing/PEC | openprot-mctp-transport-i2c |
| MctpI2cReceiver::decode | openprot-mctp-transport-i2c |
| I2cClientBlocking mock | i2c service tests |
| IpcI2cClient / handle::I2C wiring | target/platform integration |
