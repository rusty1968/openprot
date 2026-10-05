# Hubris deferred-reply vs pw_kernel channels

Analysis from 2026-09-29, done in the `attestation/pigweed` repo (branch `veer-pic-qemu-config`).

## Hubris's actual mechanism (corrected vs a garbled AI-search summary)

Hubris IPC is a synchronous rendezvous call — `SEND` always blocks the caller
until someone replies; there's no fire-and-forget queuing. "Deferred reply" is
a server-side pattern: a server that `RECV`s a message is not required to
`REPLY` before calling `RECV` again. It can stash the caller's identity, go
handle other messages (or wait on hardware/interrupts), and `REPLY` whenever
ready. This is safe because of one kernel invariant: the original caller stays
parked and can't send anything else until replied to, so the server can never
confuse it with a different message. `REPLY` returns no status (a dead/timed-out
caller makes it a silent no-op), and `RECV` hands the server the caller's real
buffer capacity up front so it can fault early instead of overflowing.
Source: https://hubris.oxide.computer/reference/

Hubris-derived kernels have been adding epoch-tagged reply tokens to fix a race
where a caller times out and re-calls the same server before a stale deferred
reply lands.

## pw_kernel's channel mechanism

`pw_kernel/kernel/object/channel.rs`:
- `channel_read` (~line 81) copies out of `transaction.send_buffer` without
  requiring an immediate `channel_respond`.
- `channel_respond` (~line 95) is the only thing that clears
  `active_transaction` and wakes the initiator — nothing forces it to happen
  synchronously after `channel_read`.
- This is structurally the same "receive now, reply whenever" pattern as
  Hubris, and it's already kernel-tested: `pw_kernel/tests/wait_group/user/handler.rs`
  holds multiple `ChannelHandlerObject`s in a `WaitGroup`, receives from
  whichever is ready, and can leave others pending before responding.

## Verdict: yes, mostly — one real structural gap

The core deferred-reply pattern (decoupled receive/respond, safe interleaving,
multiplexed pickup via `WaitGroup`) already exists in pw_kernel, no new kernel
work needed.

The gap is **addressing**, not IPC semantics:
- Hubris: one server has a single receive-any endpoint; any task can `SEND` to
  its fixed `TaskId` at runtime with no pre-established object between them.
- pw_kernel: `ChannelInitiatorObject`/`ChannelHandlerObject` are a fixed 1:1
  pair, provisioned statically in a target's `system.json5`
  (`ChannelInitiatorConfig`/`ChannelHandlerConfig` in
  `tooling/system_generator/system_config.rs`, e.g.
  `target/mps2_an505/wait_group/system.json5`). The initiator entry names its
  peer via `handler_process`/`handler_object_name`; the generator
  (`tooling/system_generator/templates/system.rs.jinja` +
  `templates/objects/channel_{initiator,handler}.rs.jinja`) emits a single
  shared `Channel::new(...)` object per handler and wraps `ForeignRc` clones
  of it in both a `ChannelHandlerObject::new(...)` and a
  `ChannelInitiatorObject::new(...)` — the pairing is resolved at
  build/codegen time, not via any runtime "connect" call. To serve N clients
  you must provision N channel pairs at image-build time and fan them into one
  server task's `WaitGroup` — not a dynamic any-task mailbox.

For a statically-configured embedded system (pw_kernel's whole model, same as
Hubris's own static `app.toml` task table) this is a shape difference, not a
blocker — it only actually fails if you need a genuinely late-bound peer that
wasn't wired at boot.

Minor secondary gap: Hubris exposes the caller's reply-buffer capacity at
`RECV` time so a server can fault early on an oversized reply. pw_kernel's
equivalent check only happens inside `channel_respond`
(`response_buffer.size() > transaction.recv_buffer.size()` →
`Error::OutOfRange`, ~channel.rs:100) — a handler currently has no way to query
`transaction.recv_buffer.size()` before doing the work to build a response.
Not a blocker, just means possible wasted work before discovering a reply
doesn't fit.
