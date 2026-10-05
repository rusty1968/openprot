# `services/mctp/server-runtime` review

Review from 2026-09-29 of `services/mctp/server-runtime/src/lib.rs`.

Checked against the panic-free / error-handling checklist (`no unwrap/expect/
panic/indexing`, checked arithmetic, `no_std`, no heap allocation) and against
the sibling `i2c/server-runtime` crate this crate's doc comment says it
mirrors. The crate is panic-free (no `unwrap`/`expect`/indexing, `no_std`, no
heap allocation, `checked_add_duration` + `unwrap_or(Instant::MAX)`, and
`Instant - Instant` saturates rather than panics on underflow). One
significant architectural issue was found.

## Main finding: one client's IPC error kills the whole multi-client server

The doc comment promises this loop lets "one `mctp_server` process answer any
number of client channels ... without duplicating the loop." But nearly every
per-channel syscall uses `?` and propagates out of `run`, which returns
`Result<()>` instead of `!`:

- `lib.rs:121` — `channel_read(ch.handle, ...)?`
- `lib.rs:144` — `channel_respond(ch.handle, ...)?`
- `lib.rs:159` — `wait_group_remove(wg, ch.handle)?`
- `lib.rs:206-213` — same pair inside `retry_pending`

Any single misbehaving/disconnected client (e.g. `channel_read`/
`channel_respond` failing because that one client tore down its channel)
tears down the *entire* dispatch loop, terminating service for every other
channel it's multiplexing.

Compare with `i2c/server-runtime`'s `run`, which returns `!` and logs-and-
`continue`s on every per-bus syscall error (`pw_log::error!(...)`) so one
bus's fault never affects the others — that's the established convention in
this codebase for exactly this kind of topology-agnostic multi-client loop,
and this crate doesn't follow it (it also has no `pw_log` dependency in
`BUILD.bazel`).

A related compounding bug in `retry_pending` (`lib.rs:200-216`): once the
closure passed to `drive_pending` hits one `channel_respond`/`wait_group_add`
error, it sets `result = Err(e)` and short-circuits (`if result.is_err() {
return; }`) for all *subsequent* pending channels in the same call — those
channels are silently dropped without a response and without being re-added
to the WaitGroup, so even if propagation weren't fatal, they'd hang forever.

**Suggested fix:** make `run` return `!` (or keep only true setup/fatal
errors propagating) and, for the per-channel operations, log via
`pw_log::error!`/`debug!` and `continue`/skip that channel instead of using
`?`, matching `i2c_server_runtime::run`.

## Minor observations (not blocking)

- No unit tests exist for this crate (consistent with it being
  `kernel`-tagged and syscall-dependent — same as `i2c/server-runtime`, so
  not a gap unique to this crate).
- `channels`/handle-uniqueness is documented ("must be non-empty with
  distinct handles") but unenforced — again mirrors the existing
  `i2c/server-runtime` convention, so not a new issue.

## Testing plan: exercise `run()` without real I2C

Goal: cover the currently-untested loop mechanics (channel multiplexing,
deferred `Recv`, timeout, `retry_pending`) on QEMU, no EVB/Pi required.

Precedent: `target/ast10x0/tests/mctp/ipc_client` is already a `qemu_only`
(`--config=virt_ast10x0`), transport-free MCTP test — but it fakes the whole
server. We want the mirror image: real `mctp_server_runtime::run()` + real
`Server`/`dispatch`, fake only the transport.

**New test dir:** `target/ast10x0/tests/mctp/server_runtime/`, BUILD.bazel/
system.json5 skeleton copied from `ipc_client` (`target_codegen`,
`linker_script`, `target` kernel binary, `system_image`, `system_image_test`
tagged `qemu_only`).

**Topology (all IPC, zero I2C, single `client_test` thread — no second
"sender" app, no fixed-delay synchronization):**
- `server_test` app — runs the real `mctp_server_runtime::run()` + real
  `Server`, wired to a new `LoopbackSender` (tiny, no_std, inline in
  `server_main.rs`, implements `mctp_lib::Sender`) instead of `I2cSender`.
  `send()` just stashes the last outbound frame in a fixed buffer. Two
  client channels (`mctp_a`, `mctp_b`) plus a `transport` channel_handler
  standing in for the I2C IRQ.
- `client_test` app — one thread, two channel initiators (`mctp_a`,
  `mctp_b`) plus `transport`. Uses `util_ipc::AsyncTransaction` (already
  precedented in `target/ast10x0/tests/util_ipc/async_transaction` and
  `pw_kernel/tests/wait_group`) to start a non-blocking `Recv` on channel A,
  then keep running on the same thread instead of blocking — this is what
  removes the need for a second process entirely. `IpcHandle::
  set_peer_user_signal(true)` (wrapping `object_set_peer_user_signal`, a
  *sticky* signal, not a pulse) pokes the `transport` handle.

**Test cases (0% coverage today), all driven by strict single-thread
sequencing — deterministic, no races:**
1. `set_eid`/`get_eid`/`listener`/`req`/`unbind` via blocking
   `IpcMctpClient` calls on channel A — sanity check of the non-deferred
   `Reply` path through the real multiplexed loop.
2. Deferred recv + transport delivery:
   a. `listener(5)` → `L` (blocking, channel A idle again after).
   b. Manually encode a `Recv` request (`wire::encode_recv(buf, L.0,
      timeout)`) and `AsyncTransaction::start()` it on channel A —
      non-blocking; server dispatches `Recv` → no data → genuinely
      `Pending`, channel A removed from the server's WaitGroup.
   c. `send(None, 5, Some(OWN_EID), None, false, b"hi")` (blocking) on
      channel B — populates `LoopbackSender`'s stash. Channel B is
      independent of A, so this can't collide with A's in-flight
      transaction (mirrors the "each channel serves one client at a time"
      invariant already documented in `retry_pending`).
   d. `IpcHandle::new(handle::TRANSPORT).set_peer_user_signal(true)` —
      wakes `run()`'s transport wait *whenever it next polls*, since the
      signal is sticky. Triggers `on_transport` → `server.inbound(...)` →
      `retry_pending` resolves A's `Pending` and re-arms it in the `wg`.
   e. Poll `try_recv()` on A's `AsyncTransaction` until it completes,
      decode the response (`wire::decode_response_header` +
      `get_response_payload`), assert msg_type/eid/payload.
   Covers `Pending` → `drive_pending` → re-add to `wg`, which nothing
   exercises today.
3. `recv()` with a short timeout and no transport poke → confirm
   `ResponseCode::TimedOut` once `earliest_deadline`/`object_wait`'s deadline
   expires. Covers the `Instant`/`Duration` math and the `DeadlineExceeded`
   branch. No `AsyncTransaction` needed — plain blocking `recv()` is fine
   since we *want* to block until the timeout response arrives.
4. Two client channels, one deliberately misbehaving → reproduces the "one
   client's error kills the whole server" bug from the main finding above;
   doubles as its regression test once that's fixed.

**Run command:**
`bazelisk test --config=virt_ast10x0 //target/ast10x0/tests/mctp/server_runtime:server_runtime_qemu_test`
