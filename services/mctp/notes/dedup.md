# MCTP Server Recv Dedup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the hand-rolled, duplicate `Recv`-op handling in `target/veer/tests/pldm_i3c/mctp_server.rs` and route every MCTP op — including `Recv` — through the shared `dispatch_mctp_op`/`drive_pending` path that the rest of the ops already use, so there is one implementation of the wire format instead of two.

**Architecture:** `services/mctp/server/src/server.rs` already tracks outstanding blocking `Recv` calls (`register_recv`/`update`, a `LinearMap<u32, PendingRecv, OUTSTANDING>`) and `services/mctp/server/src/dispatch.rs` already wraps that in `dispatch_mctp_op` (returns `DispatchOutcome::Pending` when no message is ready) and `drive_pending` (resolves it later). `mctp_server.rs` never calls any of this for `Recv` — it reimplements a single-slot pending-recv itself, bypassing `dispatch_mctp_op` and hand-encoding the response. This plan deletes that duplicate and wires the process loop to the shared functions instead. The one piece of real platform glue needed is a millis↔`Instant` bridge, because `Server`'s API is expressed in a portable `now_millis: u64` domain while the kernel's clock (`userspace::time::Instant`) is tick-based — bridged via a loop-start `epoch: Instant` and `millis_since(epoch)`.

**Tech Stack:** Rust, `no_std`/`no_main`, `pw_kernel` userspace syscalls (`object_wait`, `channel_read`/`channel_respond`, `wait_group_add`/`remove`), the host-buildable `openprot_mctp_server`/`openprot_mctp_api` crates.

**Spec:** none — scoped directly from conversation analysis of `mctp_server.rs` vs. `services/mctp/server/src/{server,dispatch}.rs`. No separate design doc; this file's Architecture section is the spec.

## Global Constraints

- `no_std` only in `mctp_server.rs` — no `Vec`/`String`/`Box`; existing fixed-size buffers stay as-is.
- Panic-free — no `unwrap`/`expect`/`panic!`/direct indexing; match the existing `?`/`match`-based error handling style in this file.
- Checked arithmetic — the one new arithmetic op (`now_millis + timeout_millis`) must use `saturating_add`, not bare `+` (CLAUDE.md's coding-constraints rule; note the shared `server.rs:136` still does bare `+` internally — that's pre-existing code, out of scope here).
- `mctp_server.rs` is `tags = ["kernel"]` and has no host-buildable unit tests of its own (`#![no_std] #![no_main]`, real `pw_kernel` syscalls) — the only way to exercise this file is the on-target emulator test `//target/veer/tests/pldm_i3c:pldm_i3c_test`. There is no failing-unit-test-first cycle available for this task; verification is "does the existing on-target test still pass," since this is a pure internal refactor with no intended behavior change.
- Single-client topology: `target/veer/tests/pldm_i3c/system.json5` wires exactly one MCTP client (`pldm_fd`) to `mctp_server`'s `mctp` channel handler. The refactor must preserve today's single-outstanding-recv behavior exactly — it is not adding multi-client support (that would need a second `channel_handler`/`channel_initiator` pair in `system.json5`, a distinct, separate change).

---

## File Structure

- **Modify:** `target/veer/tests/pldm_i3c/mctp_server.rs` — only file touched. No new files; no changes to `services/mctp/server/*` (the shared API already has everything needed) or to `system.json5`/`BUILD.bazel`.

### Task 1: Route `Recv` through `dispatch_mctp_op`/`drive_pending`

**Files:**
- Modify: `target/veer/tests/pldm_i3c/mctp_server.rs`

**Interfaces:**
- Consumes: `openprot_mctp_server::dispatch::{dispatch_mctp_op, drive_pending, DispatchOutcome}` (existing, unchanged), `openprot_mctp_server::{Sender, Server}` (existing, unchanged), `openprot_mctp_api::wire::get_recv_timeout(buf: &[u8]) -> u32` (existing, unchanged), `userspace::time::{Instant, Duration, Clock, SystemClock}` (existing, unchanged).
- Produces: nothing new for other files — this is a leaf process binary.

- [ ] **Step 1: Update imports**

  Change:
  ```rust
  use openprot_mctp_api::wire::{
      self, MctpOp, MctpRequestHeader, MAX_PAYLOAD_SIZE, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE,
  };
  use openprot_mctp_api::{Handle, ResponseCode};
  ```
  to:
  ```rust
  use openprot_mctp_api::wire::{self, MAX_PAYLOAD_SIZE, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE};
  use openprot_mctp_api::ResponseCode;
  use openprot_mctp_server::{Sender, Server};
  ```
  `MctpOp`, `MctpRequestHeader`, and `Handle` become unused once Steps 2-4 land (dispatch_mctp_op does the header parse and op-matching internally, and the platform no longer stores a `Handle`) — removing them now avoids an unused-import clippy failure at the end of this task. `Sender`/`Server` are added so Step 3's helper function and Step 2's `Server::<_, 16>::new(...)` call site don't need fully-qualified paths.

- [ ] **Step 2: Simplify server construction and pending-recv state**

  Change:
  ```rust
  let mut server = openprot_mctp_server::Server::<_, 16>::new(mctp::Eid(OWN_EID), 0, sender);
  ```
  to:
  ```rust
  let mut server = Server::<_, 16>::new(mctp::Eid(OWN_EID), 0, sender);
  ```

  Change:
  ```rust
  struct PendingRecv {
      handle: Handle,
      deadline: Instant,
  }
  let mut pending_recv: Option<PendingRecv> = None;
  ```
  to:
  ```rust
  let epoch = SystemClock::now();
  let mut pending_deadline: Option<Instant> = None;
  ```

  There is exactly one outstanding recv possible in this topology (single client, synchronous IPC), so the platform only needs to remember *whether* one is pending and *when* it times out — not which `Handle` it was for (the shared `Server` already tracks that internally, keyed off its own `outstanding` map).

- [ ] **Step 3: Add the millis↔Instant bridge and a drive-and-respond helper**

  Add above `mctp_server_loop`:
  ```rust
  /// Bridge the kernel's tick-based clock to the `Server`'s portable
  /// `now_millis: u64` domain, measured from a loop-start epoch.
  fn millis_since(epoch: Instant) -> u64 {
      (SystemClock::now() - epoch).as_millis() as u64
  }

  /// Drive any pending recv to completion and, if it resolved, send the
  /// response over the MCTP channel. Returns whether a response was sent —
  /// the caller should clear its own pending-tracking state only when this
  /// returns `true`.
  fn drive_pending_and_respond<S: Sender, const N: usize>(
      server: &mut Server<S, N>,
      epoch: Instant,
      recv_buf: &mut [u8],
      response_buf: &mut [u8],
  ) -> Result<bool> {
      let mut fired_len: Option<usize> = None;
      dispatch::drive_pending(server, millis_since(epoch), recv_buf, response_buf, |_, len| {
          fired_len = Some(len);
      });
      match fired_len {
          Some(len) => {
              syscall::channel_respond(handle::MCTP, &response_buf[..len])?;
              Ok(true)
          }
          None => Ok(false),
      }
  }
  ```
  This only ever sees 0 or 1 ready handles per call (the single-outstanding-recv invariant above) — `drive_pending`'s `on_ready` closure can't read `response_buf` itself (it's exclusively borrowed for the duration of the call), so the helper records the length and reads the buffer after `drive_pending` returns, matching the pattern already used by `services/mctp/server/tests/dispatch.rs`'s own `drive_pending` tests. This does not generalize to multiple simultaneously-ready handles; that's fine here but would need `drive_pending`'s closure signature widened (e.g. to `FnMut(Handle, &[u8])`) if this topology ever grows a second client.

- [ ] **Step 4: Replace the `DeadlineExceeded` arm**

  Change:
  ```rust
  Err(pw_status::Error::DeadlineExceeded) => {
      if pending_recv.take().is_some() {
          let resp = wire::MctpResponseHeader::error(ResponseCode::TimedOut);
          response_buf[..wire::MctpResponseHeader::SIZE]
              .copy_from_slice(&resp.to_bytes());
          let _ = syscall::channel_respond(
              handle::MCTP,
              &response_buf[..wire::MctpResponseHeader::SIZE],
          );
          let _ = syscall::wait_group_add(
              handle::WG,
              handle::MCTP,
              Signals::READABLE,
              0usize,
          );
      }
      continue;
  }
  ```
  to:
  ```rust
  Err(pw_status::Error::DeadlineExceeded) => {
      if pending_deadline.take().is_some() {
          if !drive_pending_and_respond(&mut server, epoch, &mut recv_buf, &mut response_buf)? {
              pw_log::error!("pending recv deadline fired but nothing was ready");
          }
          syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize)?;
      }
      continue;
  }
  ```
  `drive_pending_and_respond` is expected to always return `true` here — our `pending_deadline` and the `Server`'s internal deadline are computed from the identical `now_millis`/`timeout_millis` arithmetic (Step 6), so by the time our `Instant` deadline elapses, `Server::update`'s `now_millis >= pending.deadline` check is guaranteed to also be true. The `pw_log::error!` is a defensive trip-wire, not expected behavior; the wait-group is re-added unconditionally (matching the original code) so the channel is never stranded even if that invariant is ever violated by a future change.

- [ ] **Step 5: Replace the "satisfy a deferred recv" block in the `ev.user_data == 1` (I3C frame) arm**

  Change:
  ```rust
  if let Some(pending) = pending_recv.as_ref() {
      if let Some(meta) = server.try_recv(pending.handle, &mut recv_buf) {
          let payload = &recv_buf[..meta.payload_size];
          let response_len = wire::encode_recv_response(
              &mut response_buf,
              meta.msg_type,
              meta.msg_ic,
              meta.remote_eid,
              meta.msg_tag,
              payload,
          )
          .unwrap_or_else(|_| {
              wire::encode_error_response(&mut response_buf, ResponseCode::InternalError)
                  .unwrap_or(0)
          });
          syscall::channel_respond(handle::MCTP, &response_buf[..response_len])?;
          pending_recv = None;
          syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize)?;
      }
  }
  ```
  to:
  ```rust
  if pending_deadline.is_some()
      && drive_pending_and_respond(&mut server, epoch, &mut recv_buf, &mut response_buf)?
  {
      pending_deadline = None;
      syscall::wait_group_add(handle::WG, handle::MCTP, Signals::READABLE, 0usize)?;
  }
  ```

- [ ] **Step 6: Replace the `ev.user_data == 0` (MCTP request) arm**

  Change the whole arm from:
  ```rust
  let len = syscall::channel_read(handle::MCTP, 0, &mut request_buf)?;
  if pending_recv.is_some() {
      let resp = wire::MctpResponseHeader::error(ResponseCode::InternalError);
      response_buf[..wire::MctpResponseHeader::SIZE].copy_from_slice(&resp.to_bytes());
      syscall::channel_respond(
          handle::MCTP,
          &response_buf[..wire::MctpResponseHeader::SIZE],
      )?;
      continue;
  }

  if len < MctpRequestHeader::SIZE {
      let resp = wire::MctpResponseHeader::error(ResponseCode::BadArgument);
      response_buf[..wire::MctpResponseHeader::SIZE].copy_from_slice(&resp.to_bytes());
      syscall::channel_respond(
          handle::MCTP,
          &response_buf[..wire::MctpResponseHeader::SIZE],
      )?;
      continue;
  }

  // Recv op: try immediately; defer the response if no message is ready.
  if MctpRequestHeader::from_bytes(&request_buf[..len])
      .and_then(|h| h.operation())
      .is_some_and(|op| matches!(op, MctpOp::Recv))
  {
      let header = MctpRequestHeader::from_bytes(&request_buf[..len]).unwrap();
      let recv_handle = Handle(header.handle);
      let payload = wire::get_request_payload(&request_buf[..len]);
      if payload.len() < 4 {
          let resp = wire::MctpResponseHeader::error(ResponseCode::BadArgument);
          response_buf[..wire::MctpResponseHeader::SIZE]
              .copy_from_slice(&resp.to_bytes());
          syscall::channel_respond(
              handle::MCTP,
              &response_buf[..wire::MctpResponseHeader::SIZE],
          )?;
          continue;
      }

      let timeout_millis = u32::from_le_bytes(payload[..4].try_into().unwrap());
      match server.try_recv(recv_handle, &mut recv_buf) {
          Some(meta) => {
              let payload = &recv_buf[..meta.payload_size];
              let response_len = wire::encode_recv_response(
                  &mut response_buf,
                  meta.msg_type,
                  meta.msg_ic,
                  meta.remote_eid,
                  meta.msg_tag,
                  payload,
              )
              .unwrap_or_else(|_| {
                  wire::encode_error_response(
                      &mut response_buf,
                      ResponseCode::InternalError,
                  )
                  .unwrap_or(0)
              });
              syscall::channel_respond(handle::MCTP, &response_buf[..response_len])?;
          }
          None => {
              let deadline = if timeout_millis == 0 {
                  Instant::MAX
              } else {
                  SystemClock::now()
                      .checked_add_duration(Duration::from_millis(timeout_millis as u64))
                      .unwrap_or(Instant::MAX)
              };
              pending_recv = Some(PendingRecv {
                  handle: recv_handle,
                  deadline,
              });
              let _ = syscall::wait_group_remove(handle::WG, handle::MCTP);
          }
      }
  } else {
      let response_len = match dispatch::dispatch_mctp_op(
          &request_buf[..len],
          &mut response_buf,
          &mut server,
          &mut recv_buf,
          0,
      ) {
          DispatchOutcome::Reply(n) => n,
          DispatchOutcome::Pending { .. } => unreachable!("Recv handled above"),
      };
      syscall::channel_respond(handle::MCTP, &response_buf[..response_len])?;
  }
  ```
  to:
  ```rust
  let len = syscall::channel_read(handle::MCTP, 0, &mut request_buf)?;
  if pending_deadline.is_some() {
      // Unreachable in steady state: while a recv is pending, `handle::MCTP`
      // is removed from the wait group below, and this is a single-client,
      // synchronous channel — the client cannot issue a second request
      // before its first is answered. Kept as a defensive guard.
      let resp = wire::MctpResponseHeader::error(ResponseCode::InternalError);
      response_buf[..wire::MctpResponseHeader::SIZE].copy_from_slice(&resp.to_bytes());
      syscall::channel_respond(
          handle::MCTP,
          &response_buf[..wire::MctpResponseHeader::SIZE],
      )?;
      continue;
  }

  let now_millis = millis_since(epoch);
  match dispatch::dispatch_mctp_op(
      &request_buf[..len],
      &mut response_buf,
      &mut server,
      &mut recv_buf,
      now_millis,
  ) {
      DispatchOutcome::Reply(n) => {
          syscall::channel_respond(handle::MCTP, &response_buf[..n])?;
      }
      DispatchOutcome::Pending { .. } => {
          let timeout_millis = wire::get_recv_timeout(&request_buf[..len]);
          pending_deadline = Some(if timeout_millis == 0 {
              Instant::MAX
          } else {
              epoch + Duration::from_millis(now_millis.saturating_add(timeout_millis as u64))
          });
          syscall::wait_group_remove(handle::WG, handle::MCTP)?;
      }
  }
  ```
  `dispatch_mctp_op` parses the header itself (`MctpRequestHeader::from_bytes`, which already rejects anything shorter than `MctpRequestHeader::SIZE` with `BadArgument`) and dispatches every op — `SetEid`/`GetEid`/`Listener`/`Req`/`Send`/`Unbind` unchanged from before, `Recv` now going through the same path instead of a hand-rolled duplicate. The explicit `len < MctpRequestHeader::SIZE` check and the double `MctpRequestHeader::from_bytes` parse are gone because `dispatch_mctp_op` already does both once.

- [ ] **Step 7: Build the image**

  Run: `bazel build //target/veer/tests/pldm_i3c:pldm_i3c_image --build_tag_filters=-hardware,-disabled,-verilator`
  Expected: builds clean, no clippy/unused-import warnings for `mctp_server.rs`.

- [ ] **Step 8: Run the on-target regression test**

  Run: `bazel test //target/veer/tests/pldm_i3c:pldm_i3c_test --test_tag_filters=+emulator --test_output=all`
  Expected: PASS — same firmware-update flow (`RequestUpdate` → `PassComponentTable` → `UpdateComponent` → download/verify/apply loop) succeeds exactly as before. This is the only available regression signal for this file (see Global Constraints); a pass here is the acceptance bar for this refactor, since no behavior change is intended.

- [ ] **Step 9: Commit**

  ```bash
  git add target/veer/tests/pldm_i3c/mctp_server.rs
  git commit -m "target/veer/tests/pldm_i3c: route Recv through dispatch_mctp_op/drive_pending"
  ```

---

## Self-Review

- **Coverage:** the single behavior discussed (dedup the `Recv` path onto the shared `dispatch_mctp_op`/`drive_pending` API) is fully covered by Task 1; no other files need changes per the File Structure section.
- **Placeholders:** none — every step shows the literal before/after code.
- **Type consistency:** `pending_deadline: Option<Instant>` is introduced in Step 2 and used identically in Steps 4-6; `drive_pending_and_respond`'s signature (Step 3) matches its two call sites (Steps 4-5) exactly (`&mut server, epoch, &mut recv_buf, &mut response_buf`, returning `Result<bool>`); `millis_since` is defined once (Step 3) and used at both remaining call sites (Steps 4-6, via `drive_pending_and_respond`, and directly in Step 6).
