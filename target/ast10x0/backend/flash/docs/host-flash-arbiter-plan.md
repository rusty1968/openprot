# AST10x0 Host-Flash Arbiter — Implementation Plan

Status: plan (not started)
Design: [host-flash-arbiter.md](host-flash-arbiter.md)

Implements Option C: separate per-device host-flash servers (BMC on SPI1, PCH on
SPI2), each a client of a single **arbiter** service that solely owns the SCU
internal-master mux (`SCU0F0`).

## Guiding decisions

- **Split = internal trait seam, not a generic crate.** The arbiter core is
  written against a `SteeringResource` trait so its logic can be unit-tested on
  the host with a mock, but everything ships as **ast10x0-local crates under
  `target/ast10x0`**. There is no speculative platform-agnostic service crate —
  if a second board ever needs this, promote the core module then.
- **Lease granularity = one bounded operation** (matches today's per-op
  `route_bus`/`Drop`).
- **Flash op wire format is unchanged.** Device selection is by separate
  per-device server/channel; the arbiter has its own small opcode set.

## Crate layout (new)

```
target/ast10x0/backend/steering-arbiter/
    BUILD.bazel
    src/
        lib.rs        // re-exports; #![no_std]
        resource.rs   // `SteeringResource` trait (abstract seam)
        core.rs       // arbiter state machine (generic, host-testable)
        scu.rs        // ast10x0 SCU-mux `SteeringResource` impl (proprietary)
        opcode.rs     // RouteAcquire/RouteRelease/RouteGrant + Opcodes
        server.rs     // IPC server: core + resource behind a channel
        client.rs     // `SteeringClient` used by host-flash servers
    tests/
        core_tests.rs // host unit tests with a mock SteeringResource
```

Bazel targets: `steering_arbiter` (rust_library, `TARGET_COMPATIBLE_WITH`) and a
host `rust_test` for `core.rs` (no target constraint, so it runs on the build
host without QEMU/EVB).

## Step 1 — `SteeringResource` trait + arbiter core

`resource.rs`:

```rust
pub trait SteeringResource {
    type Source: Copy + Eq;
    /// Route the shared master onto `source` (program mux + save pin state).
    fn steer(&mut self, source: Self::Source) -> Result<(), ErrorCode>;
    /// Restore the default (passthrough) state. Must be infallible/idempotent.
    fn restore(&mut self);
}
```

`core.rs` — pure state machine, no hardware, no IPC:

- State: `Free` or `Held { source, generation, deadline }`, plus a small FIFO
  waiter queue keyed by client id.
- `acquire(client, source, now) -> Grant | Queued | Denied` — authorization is
  checked by the caller (server) before this; core enforces mutual exclusion,
  fairness, and issues `token = generation`.
- `release(client, token, now)` — generation check; on match `restore()` +
  `generation += 1` + wake head of queue; on mismatch, no-op (stale).
- `tick(now)` — deadline reclaim: if `Held` and `now >= deadline`, `restore()`,
  bump generation (invalidating the outstanding token), wake next.

Core calls `SteeringResource::{steer,restore}` at the grant/release/reclaim
edges. It never blocks; blocking/queueing is the server's `object_wait` loop.

## Step 2 — Host unit tests (mock resource)

`tests/core_tests.rs` with a `MockResource { steered: Option<Src>, restore_calls }`
that records calls. Cover:

- grant → release happy path; generation increments.
- second acquire while held → queued; served after release (FIFO order).
- deadline reclaim restores and bumps generation.
- stale release after reclaim → no-op, does not disturb the new holder.
- double release → no-op.
- reclaim wakes the queued waiter.

These are the race-prone paths that QEMU/EVB cannot exercise (the SPIM-mux host
path is unmodeled), so they are the primary correctness evidence.

## Step 3 — ast10x0 SCU backend

`scu.rs` — `ScuSteering { scu: ScuRegisters }` implementing `SteeringResource`
with `Source = (SpiMonitorSource, SpiMonitorInstance)`:

- `steer` → `set_spim_internal_mux(source, monitor as u8 + 1)` +
  `spim_proprietary_pre_config()` (store the returned `SpimGpioOriVal`).
- `restore` → `spim_proprietary_post_config(state)` + `clear_spim_internal_master_route()`.

This is exactly the logic currently inlined in `host.rs`'s `route_bus`/`BusRoute`,
relocated so the arbiter is the sole SCU owner. `ScuRegisters::new_global_unlocked`
is called once here (unsafe, sole-owner contract now satisfied by construction —
only the arbiter process maps the SCU).

## Step 4 — Arbiter IPC opcodes

`opcode.rs` (zerocopy structs, mirrors `services/flash/opcode.rs` style):

```rust
pub const IPC_OP_ROUTE_ACQUIRE: Opcode = Opcode::new(*b"RTAQ");
pub const IPC_OP_ROUTE_RELEASE: Opcode = Opcode::new(*b"RTRL");

struct RouteAcquire { source: u32, monitor: u32 }  // logical source id
struct RouteGrant   { token: u64 }
struct RouteRelease { token: u64 }
```

## Step 5 — Arbiter server + client

`server.rs` — owns `core::Arbiter<ScuSteering>`; `handle_one` maps
ACQUIRE/RELEASE to core calls, enforces per-client `source` authorization (from a
compile-time table keyed by which channel the request arrived on), and responds
with a grant or an error/queued status. The `object_wait` loop also drives
`tick(now)` for deadline reclaim (wake on timeout as well as READABLE).

`client.rs` — `SteeringClient { ipc: IpcHandle }` with `acquire(source,monitor)
-> Token` and `release(Token)`, plus an RAII `RouteLease<'_>` guard whose `Drop`
issues `release` (the client-side analogue of today's `BusRoute`).

## Step 6 — Arbiter binary + system.json5

- New app `steering_arbiter_bin` with `arbiter_main.rs`: constructs
  `ScuSteering` (unsafe, sole SCU owner) + `core::Arbiter`, runs the
  wait/handle/tick loop.
- `system.json5`: arbiter process maps **only** the SCU regs; declares two
  `channel_handler` objects (`route_bmc`, `route_pch`). Each host-flash server
  gets a `channel_initiator` to the matching arbiter channel.

## Step 7 — Migrate `host.rs` to the arbiter client

`Ast10x0SpiHostFlashDriver` changes:

- Remove `scu: ScuRegisters` and the `route_bus`/`BusRoute` SCU poking.
- Add a `SteeringClient` (or hold an `IpcHandle` to the arbiter).
- Each op: `let _lease = self.steering.acquire(self.source, self.monitor)?;`
  then the inlined `SpiNorFlash::from_spi_cs(...)` work; `RouteLease::drop`
  releases. Same disjoint-borrow structure as today, just against the client.
- `new` no longer takes SCU ownership; its safety contract drops the "coordinate
  SCU access" clause (the arbiter owns it) and gains "an arbiter serving
  `monitor` must be reachable on the provided channel".

## Step 8 — Two host-flash server binaries

`host_bmc_server` (SPI1) and `host_pch_server` (SPI2), each like today's
`server_main.rs` but constructing `Ast10x0SpiHostFlashDriver` with its
controller/cs/monitor and an arbiter channel. Neither maps the SCU.

## Step 9 — EVB integration test

EVB-only (no QEMU model for the SPIM-mux host path). Exercise concurrent
BMC+PCH access to prove serialization and passthrough restore; assert the host
buses return to passthrough after each op. Tag `["hardware"]`, incompatible with
`qemu_enabled`, mirroring `flash_evb_test`.

## Sequencing / dependencies

1 → 2 (core provable before any hardware). 1,3 → 4,5. 5,6 gate 7,8. 9 last.
Steps 1–2 land independently (pure logic, host-tested) and are the lowest-risk
starting point.

## Open items to resolve during implementation

- Deadline value: measure worst-case single-op (4 KiB erase / 256 B program
  WIP-poll) on EVB; set deadline = that + margin.
- Whether the kernel exposes client-channel-death signalling for immediate
  reclaim vs. relying solely on the deadline.
- Queued-grant mechanics on the existing IPC primitive (deferred response vs.
  re-notify); confirm against `util_ipc` capabilities before finalizing `server.rs`.
