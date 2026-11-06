# Scope: event-driven PLDM FD (wait for the UA response *or* a UA command)

Status: scoping only, nothing implemented. Written 2026-10-23 against OpenPRoT branch
`mctp-i3c-fpga-bench` at `41280405`.

## 1. Problem

`FirmwareDevice::run_until` does two jobs on one thread: it sends FD-initiated requests (one
RequestFirmwareData per chunk) and it must still answer UA commands (CancelUpdate, GetStatus...).
The MCTP client API has one *blocking* receive per handle and no "wait for either", so the FD
takes turns: send, wait for the response, then poll the responder listener with a 1 ms timeout.
`timeout 0` means "wait forever", so a true non-blocking check does not exist.

Cost on the FPGA: the poll was about 29 ms per chunk (62.2 s download with poll-every-16 vs 72.0 s
polling every step; native is 40.7 s). Poll-every-16 (`41280405`) recovered about 15/16 of that
but made a UA command wait up to 16 requests (about 3 s now) before it is answered. The aim of
this change is to get the speed *and* the responsiveness back, and to cut the other per-chunk IPC.

## 2. Current flow and IPC count per chunk (FD side)

```
 FD thread                          MCTP server (colo process)
 generate request
 send_request:
   stack.req_reusable  (handle cached, no IPC after the first chunk)
   send   ---- IPC #1 ---->         register tag, fragment, I3C send, reply
   recv   ---- IPC #2 ---->         defer reply until the response is reassembled   (blocks)
 process_initiator_response
 [every 16th step] respond_once:
   listener.recv(1 ms) -- IPC ---->  defer with deadline; wake on deadline; TimedOut
```

So 2 IPC round trips per chunk, plus one per 16 chunks. Each round trip is a kernel channel
transaction, a pass through `mctp_server_runtime::run`, and encode/decode on both sides, all at
about 58 cycles per instruction. The server runs about 11 passes per chunk in total (2 requests,
5 IRQs, the deferred completions).

## 3. Mechanics that constrain the design (read from the code)

- Both FD transports use the same kernel channel (`IpcMctpClient::new(handle::MCTP)` twice in
  `pldm_fd.rs:286-287`); the thread can have only one `channel_transact` outstanding anyway.
- The server defers a `Recv` per handle: `register_recv(handle, timeout, now)` stores a deadline in
  `outstanding`; `Server::update` resolves it when a message for that cookie arrives or the
  deadline passes; `drive_pending` encodes the reply and `server-runtime` calls
  `service.complete(handle, bytes)` for the reply token stored by `service.defer(channel, handle)`.
- Handles (cookies) are server-global, so a request on one channel can name a handle that another
  `MctpClient` allocated. A single client for both roles is cleaner.
- A handle has at most one outstanding receive (`AddrInUse` otherwise).
- `StackListener` / `StackReqChannel` borrow `&Stack`; both can live at once on one `Stack`.
- `mctp-lib` `Router::recv(cookie)` is per cookie; no multi-cookie receive. `mctp-estack` offers
  `get_deferred_bycookie(&[cookie])`, which already takes a slice, so a multi-cookie lookup is
  available below `mctp-lib`.

## 4. Options

| | Idea | IPC per chunk | UA command latency | Size |
|---|---|---|---|---|
| A | `RecvAny([handles], timeout)`: one blocking receive that completes on whichever handle gets a message first | 2 (send, recvany) | immediate | medium |
| B | `Transact`: send + wait for the response in one IPC | 1 (plus the poll, unchanged) | poll-bound | small |
| C | `SendRecvAny`: send on the request handle, then wait for the response *or* a message on the listener, one IPC | **1** | immediate | medium+ |
| D | two threads in `pldm_fd` (one per direction) | 2 | immediate | large (shared FD state, locking) |

Recommendation: C, built on A's server mechanics (A is the sub-step; B falls out of C). D is
rejected: the FD state machine (`CmdInterface`) is single-threaded by construction.

## 5. Proposed design (C, via A)

**Wire (`services/mctp/api/src/wire.rs`)**
- New op `RecvAny = 7` and `SendRecvAny = 8`. `RecvAny` request: header with the existing
  timeout field plus up to 4 handles (u32 each) in the payload. `SendRecvAny` request: the Send
  header/payload, plus the listener handle and the timeout. Response: the existing Recv response
  with the header `handle` field set to the handle that fired, so the client knows which side
  (response vs UA command) it got.
- Encode/decode helpers and round-trip tests; keep the 12-byte header unchanged.

**Server (`services/mctp/server`)**
- `PendingRecv` gains a `group: Option<Handle>` (the handle the reply token was deferred under).
- `register_recv_any(handles, timeout, now)` registers each handle with the same deadline and
  group; fails atomically (nothing registered) on `AddrInUse`/`NoSpace`.
- `try_recv_any`: check all handles for an already-queued message first (a UA command may already
  be waiting).
- `update`: when a handle in a group resolves, resolve the whole group once (remove the siblings'
  `outstanding` entries) and report which handle fired; on timeout report one `TimedOut`.
- `dispatch.rs`: handle the two new ops; `drive_pending` encodes the Recv response with the firing
  handle in the header.
- `server-runtime`: `defer(channel, group_key)` / `complete(group_key, bytes)` use the group key;
  no change to the loop structure.

**Client (`services/mctp/client-ipc`, `api`)**
- `MctpClient` gets `recv_any(&self, handles: &[Handle], timeout, buf) -> Result<(Handle,
  RecvMetadata), MctpError>` and `send_recv_any(...)`, both with default bodies returning an
  "unsupported" error, so the mock clients in the tests keep compiling.
- `Stack`: a method that takes a `StackReqChannel` and a `StackListener` and does the combined
  wait, returning an enum `Reply(meta)` / `Command(meta, resp_channel)`.

**PLDM (`services/pldm`)**
- One `MctpPldmTransport` for both roles (the borrow structure already allows it).
- `run_once_inner`: Phase 1 becomes "send the request and wait for (response | command)". If a
  command fires first, run the existing `respond_once` logic on the message already received,
  then wait again for the response (re-issue the receive on the request handle only; the request
  is already in flight). The responder poll and `set_responder_poll_interval` go away (or stay as
  a no-op for compatibility). When no request is active, behaviour is unchanged (block on the
  listener).
- `pldm_fd.rs` (colo and the other benchmark FDs): one transport.

## 6. Expected gain (honest)

- A/C versus poll-every-16: the poll is already 15/16 gone, so removing it entirely is only about
  29 / 16, roughly **2 ms per chunk**. The value there is correctness: a CancelUpdate is answered
  immediately instead of up to 16 requests late. No lost throughput.
- C merges the send and receive IPCs: **one IPC round trip fewer per chunk**. What one round trip
  costs here is unmeasured. Indirect evidence: removing the 1 KB copy plus two handle IPCs took the
  FD-side segments from 195 ms to 90 ms, and the earlier 40 ms and 15 ms windows disappeared.
  Guess: 15-30 ms per chunk, which would close a quarter to a half of the remaining 65 ms gap.
  This is a guess until Phase 0.

## 7. Plan

0. **Measure one IPC round trip** (about an hour): in `pldm_fd.rs` time 100 `get_eid` calls with
   `debug_clock_now` and log the average cycles. Also count server passes per chunk from the
   existing trace. This decides whether C is worth its size. If an IPC is about 5 ms, stop at A
   (correctness) and look elsewhere; if it is 20+ ms, do C.
1. Wire ops + tests (host).
2. Server group receive + dispatch + `drive_pending` + tests (host, existing server test
   harnesses `mctp_server_*_test`).
3. Client trait defaults + `IpcMctpClient` + `Stack` API + tests (`mctp_api_test`).
4. PLDM loop restructure + host tests: existing full-flow test (which already runs with the
   default and rate-limited poll) plus new ones: a UA command arriving while a response is
   outstanding is answered immediately; the response then still completes; a timeout resets the
   group cleanly; `AddrInUse` leaves nothing registered.
5. FPGA run, compare against 62.2 s (poll16) and 72.0 s.

Rough size: wire about 150 lines, server about 200, client about 120, PLDM about 150, tests about
500. Two to three working days of careful work including the host tests, plus board runs.

## 8. Risks

- Server group bookkeeping: a response arriving in the same `update` as the timeout, or both
  handles resolving at once (response and a command). Rule: resolve the first in handle-list order
  (response first), leave the other message queued for the next receive. Needs a test.
- Re-entrancy of the deferred reply token: `service.defer` is keyed by one handle per channel; the
  group key must be released exactly once on every path (completion, timeout, `Unbind` of either
  handle, defer failure).
- A command that arrives while the response is outstanding changes the order of replies on the
  wire; the PLDM state machine already handles UA commands between requests, but not *during* one.
  `CancelUpdate` mid-request must be answered and then the in-flight response either discarded or
  processed; define which, and test it.
- `MctpClient` is implemented by several mocks and by `IpcMctpClient`; default trait bodies avoid
  breaking them, but the unsupported path must fail loudly.
- Wire compatibility: new opcodes only; old clients unaffected.

## 9. Open questions

- What does one IPC round trip cost on this board (Phase 0)?
- Does the real UA time out on commands sent during a transfer faster than the poll-every-16
  latency? (decides whether the interim 16-step poll is safe as a stopgap)
- Is `mctp-lib` happy with two cookies being receive-registered under one deferred reply token
  (it should be: the receive is by cookie, the grouping is server-side)?
