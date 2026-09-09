# Claude Code task: PLDM ↔ Orchestrator notify-protocol prototype

You are implementing a control-plane IPC prototype in the OpenPRoT firmware
codebase (`no_std` Rust, Bazel build, pw_kernel microkernel). Work **phase by
phase**. After each phase you **MUST STOP** and wait for a human "continue"
before starting the next phase. Do not run ahead.

---

## 0. Mission

Add a notification/coordination channel between the **PLDM FirmwareDevice**
(server / handler) and the **Orchestrator** (supervisor / client / initiator).
Roles, deliberately, invert the i2c client/server mapping only in *which*
service plays "server":

- **PLDM = server / handler** — same shape as `services/i2c` server-runtime.
- **Orchestrator = client / initiator** — it subscribes, then drives the update
  on **its own timed `object_wait` loop**.

The central design property: the orchestrator is a **deadline-driven
supervisor**. Its update-coordination poll is just one more timer folded into
the loop it already runs for boot watchdogs and the anti-rollback commit window.
Rollback/commit deadlines must fire **even if PLDM never sends a byte**.

Design reference (read it first):
`docs/src/design/orchestrator/pldm-orchestrator-ipc-alt.md`.

---

## 1. Hard constraints (apply to ALL code you write)

From `.github/copilot-instructions.md` — these are non-negotiable and will be
reviewed:

- **Panic-free.** No `unwrap`/`expect`/`panic!`/direct indexing. Use
  `match`/`?`/`.get(...).ok_or(...)?`.
- All fallible operations return `Result` or `Option`.
- Integer ops use `checked_`/`saturating_`/`wrapping_` where overflow is possible.
- **`no_std`, no heap.** No `Vec`/`String`/`Box`/`HashMap`. Use fixed arrays and
  `heapless`.
- **No `Send`/`Sync` supertrait bounds on traits.** Thread-affinity belongs on
  concrete types via `PhantomData<*const ()>`, never on a trait.
- Hardware/MMIO access via HAL traits and volatile ops only.
- Don't leak sensitive data in error messages.
- Don't add docs/comments/annotations to code you didn't change. One-line
  comments only, and only for what code can't show.

Build/test everything with Bazel. Discover exact target names from the
`BUILD.bazel` files; do not guess. Host-testable crates must pass
`bazel test //...` for their target on the host config.

---

## 2. Repo grounding (mirror these, don't reinvent)

Template pattern — the i2c 5-crate seam:

- `services/i2c/api/src/protocol.rs` — `#[repr(u8)]` op enum + zerocopy
  request/response headers. **Mirror this style.**
- `services/i2c/api/src/transport.rs` — the `Transport` trait seam
  (`transact(req, resp) -> Result<usize, _>`). **Reuse this exact seam.**
- `services/i2c/client/src/lib.rs` — marshalling client generic over
  `T: Transport`, no syscalls (host-testable).
- `services/i2c/client-ipc/src/lib.rs` — kernel-only `IpcTransport` calling
  `channel_transact(handle, req, resp, deadline)`. **NOTE: i2c uses
  `Instant::MAX`. Our client-ipc MUST use a bounded deadline instead.**
- `services/i2c/server-runtime/src/lib.rs` — the WaitGroup loop. Study the
  slave-notification path: on event it raises `object_set_peer_user_signal(
  channel, true)`; in the `SlaveReceive` handler it **clears the signal at the
  TOP of the handler, before draining the latched data** (race-safe). **Mirror
  that clear-before-drain ordering.**

Existing orchestrator pieces to integrate with (do not duplicate):

- `services/orchestrator/sm/src/model.rs` — `Event`, `Effect`, `State`.
  Relevant: `Event::{UpdateRequest, UpdateVerified, UpdateRejected, Timeout,
  CommitTimeout}`, `Effect::{StageUpdate, AuthenticateUpdate, ActivateUpdate,
  DiscardStaged}`, `State::{Ready, Updating}`.
- `services/orchestrator/sm/src/lib.rs` — `Orchestrator::dispatch(&mut self,
  platform, event)` and the `Platform` trait.
- `services/orchestrator/server/src/runtime.rs` — `BootWatchdogs` with
  `wait_deadline() -> Instant` and `poll_expired() -> Option<Event>`. This is
  the loop the PLDM poll timer folds into.
- `services/orchestrator/timer/` — the generic `TimerManager`.
- `services/pldm/src/firmware_device.rs` — `FirmwareDevice::run_terminus(...)`,
  the terminus loop the notify server-runtime hooks into.

Syscall surface (from `@pigweed//pw_kernel/userspace`, `userspace::syscall`):
`wait_group_add`, `object_wait`, `channel_read`, `channel_respond`,
`channel_transact`, `object_set_peer_user_signal`, `interrupt_ack`.

New crates live under `services/pldm/notify-*`. Only the two `*-ipc` /
`*-runtime` crates are kernel-tagged (depend on `@pigweed//pw_kernel/userspace`);
everything else is host-testable.

---

## PHASE 1 — Host-testable protocol (no kernel)  ⟵ START HERE

Deliverables:

1. **`services/pldm/notify-api`** (host-testable)
   - `Op` enum `#[repr(u8)]`: `Subscribe`, `Poll`, `Decision`, `PushStatus`.
   - Zerocopy request/response headers (mirror `i2c` header style).
   - Re-export / define the `Transport` seam (same signature as i2c's).
   - Payload types: `Pending` (`UpdateRequested`, `Offer { target, total }`,
     `Complete { written }`, `Activate`, `Abort`), `Decision`
     (`Accepted`/`Rejected`), `Phase` (`Verifying`/`Staging`/`Staged`/`Failed`/
     `Activating`/`Idle`). All plain data, naturally `Send`/`Sync` — do NOT add
     marker bounds.

2. **`services/pldm/notify-server`** (host-testable, pure)
   - `NotifyState { notify_armed: bool, latched: Option<Pending> }`.
   - `dispatch(state: &mut NotifyState, req: &[u8], resp: &mut [u8]) -> usize`.
   - `Subscribe` sets `notify_armed`. `Poll` **clears the pending-notify
     condition first, then drains `latched`** (clear-before-drain). `Decision`
     and `PushStatus` update state and ack.

3. **`services/pldm/notify-client`** (host-testable)
   - `PldmLink<T: Transport>` with `subscribe()`, `poll() -> Result<Option<
     Pending>, _>`, `decide(Decision)`, `push_status(Phase)`. Marshalling only,
     no syscalls.

4. **Host integration test** (in `notify-client` or a `tests` crate)
   - A fake in-memory `Transport` that routes `transact` straight into
     `notify_server::dispatch` against a shared `NotifyState`.
   - Test A: subscribe → server latches `UpdateRequested` → `poll()` drains it →
     `decide(Accepted)`.
   - Test B (race): event latched **before** the poll is still delivered by the
     next `poll()` (proves the level-latched semantics).
   - Test C (timeout): a `Transport` that never answers → client surfaces a
     timeout/error variant the runtime can later treat as "unhealthy peer".

Acceptance: `bazel test` green for all Phase-1 targets on the host config;
zero panics/unwraps; no `Send`/`Sync` bounds on any trait.

**STOP. Report what you built, the test output, and any deviations. Wait for
"continue" before Phase 2.**

---

## PHASE 2 — Orchestrator time integration + kernel wiring

Do NOT start until Phase 1 is reviewed and approved.

5. **Extend `services/orchestrator/timer`** — add a poll/liveness deadline class
   alongside boot + commit timers, so `wait_deadline()` folds it in and
   `poll_expired()` can yield a new event (e.g. `PldmPollDue` /
   `PldmUnhealthy`). Host-testable; add unit tests.

6. **`services/orchestrator/server` runtime** — register the PLDM channel on the
   WaitGroup for `Signals::USER`. Keep a **single** `object_wait(wg, mask,
   wait_deadline())`. Both a USER wake and a poll-timer expiry lead to a
   `PldmLink::poll()`.

7. **Map `Pending` → `sm::Event`** (`UpdateRequest`/`UpdateVerified`/
   `UpdateRejected`) and call `orch.dispatch(&mut driver, event)`. Map the
   results of `Effect::{StageUpdate, AuthenticateUpdate, ActivateUpdate}` back
   out via `push_status`.

8. **Bounded transact + unhealthy-peer path** — every Orchestrator→PLDM
   `transact` uses the deadline from step 5. On timeout: drive
   `Effect::DiscardStaged`, release staging, stop polling the dead peer. This is
   a first-class runtime path, not an appendix.

9. **`services/pldm/notify-client-ipc`** (kernel-only) — `IpcTransport` impl of
   `Transport` calling `channel_transact` with the **bounded** deadline
   (explicitly NOT `Instant::MAX`). This is the one intentional divergence from
   the i2c client-ipc template — comment it as such (one line).

10. **`services/pldm/notify-server-runtime`** (kernel-only) — hook the PLDM
    terminus loop: when `run_terminus` latches a `Pending` transition, call
    `object_set_peer_user_signal(orch_channel, true)` gated on `notify_armed`.
    Register the orch channel for `READABLE`; handle `Subscribe`/`Poll`/
    `Decision`/`PushStatus` via `notify_server::dispatch`. **Clear USER at the
    top of the `Poll` handler, before draining** (mirror i2c `SlaveReceive`).

Acceptance: kernel crates build under the kernel config; host crates still green;
the timer + mapping logic has host unit tests; bounded-timeout path is covered
by a test that simulates a silent PLDM.

**STOP. Report and wait for final review.**

---

## Working rules

- One phase at a time. **Hard stop** between phases; summarize and wait.
- Prefer editing existing crates over inventing new abstractions.
- Do not stage or commit anything unless explicitly told to.
- If a constraint above conflicts with what you find in the repo, stop and ask
  rather than guessing.
