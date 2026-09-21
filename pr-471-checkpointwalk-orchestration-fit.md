# PR #471 — `CheckpointWalk` and where it sits in the orchestrator

PR: https://github.com/OpenPRoT/openprot/pull/471
Author: Christina Quast (chrysh) · Base: main · Stacked on #475 (probe rename)
+846 / −110 across 9 files, 3 commits.

This document traces PR #471 through the orchestrator's layers — what problem
it closes, which crates it touches, how it changes the event vocabulary the
state machine consumes, and what it deliberately leaves for later.

---

## 1. The three commits

1. **`orchestrator: Add CheckpointWalk, the concrete BootWatch`** — new
   `services/orchestrator/adapters/walk` crate. `CheckpointWalk<R, P>` walks a
   device's `BootCheckpoint<P>` list in declaration order, polling an
   `EvidenceReader<P>` at each step, judging per-checkpoint deadlines against a
   caller-injected `now_millis`. Replaces the QEMU runtime itest's hand-rolled
   checkpoint loop for scenarios 1–2 (scenarios 3–5, `BootWatchdogs`
   multiplexing and the commit watchdog, are untouched).
2. **`orchestrator: review fixes, document u64::MAX sentinel, clean up style`**
   — doc/test polish on the same crate (unarmed/post-terminal poll sentinel,
   `Box::leak` → `const`, wording).
3. **`orchestrator: Add Event::BootFailed, preserve walk failure cause`** —
   `services/orchestrator/sm`: new `BootFailureKind` enum and
   `Event::BootFailed { id, checkpoint, kind }`. `services/orchestrator/driver`:
   maps `WalkVerdict::Failed { checkpoint, cause }` to `BootFailed` instead of
   collapsing it to `Event::Timeout`.

## 2. Where the new code lives

```
services/orchestrator/
├── capabilities/            (pre-existing) BootWatch, EvidenceReader, BootStatus,
│                             WalkVerdict, FailureCause — the trait vocabulary
├── config/                  (pre-existing) BootCheckpoint<G>, DeviceConfig — board-declared
│                             checkpoint lists and per-checkpoint windows
├── adapters/
│   ├── hal/                 (pre-existing) GPIO/signal HAL bindings
│   └── walk/                ← NEW in this PR: CheckpointWalk<R, P>, the one
│                              concrete BootWatch impl
├── driver/                  PlatformDriver — routes SM effects to capabilities,
│                             polls boot walks, maps verdicts to Events
│                             (CHANGED: Failed → BootFailed, not Timeout)
├── sm/                      the pure state machine
│                             (CHANGED: + BootFailureKind, + Event::BootFailed)
├── server/                  BootWatchdogs — the *other*, coarser timeout path
│                             (untouched by this PR)
└── timer/                   TimerManager — clock-free deadline bookkeeping
                              (untouched by this PR)
```

`CheckpointWalk` is a **new implementation of an existing trait**
(`BootWatch`, in `capabilities/src/boot_watch.rs`), not a new seam. Everything
above and below that trait — `PlatformDriver::poll_boot_walks`, the SM's event
vocabulary, `EvidenceReader`, `BootCheckpoint` — already existed. This PR fills
in the one box (`adapters/walk`) that had a documented trait and no
implementation.

## 3. The gap this closes

Two design docs already in this tree flagged the exact gap PR #471 fills.

**`docs/src/design/orchestrator/orchestrator-model.md` §7**, "What This Model
Does Not Cover":

> Multiple intermediate boot-progress checkpoints per component: the CSA
> architecture allows platform policy to require multiple intermediate
> readiness signals before a component is considered fully booted. This model
> simplifies that to a single `ComponentReady` event per `Active` component.
> **The platform is responsible for aggregating any intermediate signals and
> delivering `ComponentReady` only once all platform-policy checkpoints have
> been satisfied.**

`CheckpointWalk` is that aggregator. It is the concrete thing that turns a
board's declared list of checkpoints (`BootCheckpoint<P>`, e.g. `bl1` →
`kernel` → `service`) into the single terminal verdict — `Complete` or
`Failed { checkpoint, cause }` — that the driver then turns into the one SM
event (`ComponentReady`/`Booted`/`BootFailed`) the model above documents as
the simplification boundary.

**`boot-watchdog-design-derivation.md`**, "Open item" (end of that document):

> The mechanism is complete and validated; the remaining work is **wiring**:
> the dispatch loop must translate the sm's effect trace into `arm_boot` (on
> `ReleaseReset`), `cancel_boot` (on `ComponentReady`/`Booted`), …

That "open item" was about the *coarse*, single-deadline watchdog
(`BootWatchdogs`/`TimerManager`, in `server`/`timer`). PR #471 doesn't wire
that one — it builds a second, richer path that sits at the same seam
(`BootWatch`) but judges *within* a component's boot, not just at its outer
edge. Both remain true simultaneously in the tree right now (see §6).

## 4. Data flow: from a board's checkpoint table to a state transition

```
target/<board>/devices.rs
  DeviceConfig::new("bmc", id, &[BootCheckpoint::new("bl1", sig, 500ms),
                                  BootCheckpoint::new("kernel", sig, 5s),
                                  BootCheckpoint::new("service", sig, 30s)])
        │  (compiled in, &'static)
        ▼
services/orchestrator/adapters/walk
  CheckpointWalk<R, P>::new(reader, checkpoints)
    .arm()                     — on release_reset(id)
    .poll(now_millis)          — polled by the driver every runtime tick
        │
        │  reader.read(checkpoint.probe()) → BootStatus
        │    Booting        → WalkVerdict::Waiting { deadline_millis }
        │    Booted         → advance cursor, new deadline, or Complete
        │    FailedRetriable→ WalkVerdict::Failed { checkpoint, DeviceRetriable }
        │    FailedFatal    → WalkVerdict::Failed { checkpoint, DeviceFatal }
        │    (now ≥ deadline)→ WalkVerdict::Failed { checkpoint, TimedOut }
        │    Err(_)         → treated as Booting (silence, not failure)
        ▼
services/orchestrator/driver  (PlatformDriver::poll_boot_walks)
  WalkVerdict::Complete        → Event::ComponentReady(id) | Event::Booted(id)
                                  (by ComponentKind: Active | Passive)
  WalkVerdict::Failed { checkpoint, cause }
                                → Event::BootFailed { id, checkpoint, kind }   ← NEW
                                  (cause →1:1→ BootFailureKind, driver::driver.rs)
        ▼
services/orchestrator/sm  (Rot::dispatch)
  AwaitingReady / SupervisingPlatform handlers:
    Event::BootFailed { id, .. } | Event::Timeout(id) =>
        if is_awaiting_boot(id) → Transition(Recovering(id))
        else                    → Handled (stale, dropped)
```

The important line is the last one: **`BootFailed` and `Timeout` are matched
in the same arm, with the same guard, to the same transition.** The SM does
not yet distinguish `BootFailureKind::DeviceFatal` from
`BootFailureKind::TimedOut` — both enter `Recovering` identically today, and
both go through the same two-stage recovery (`Recovering` → retry cap →
`Isolable`/`Cascading`/`Required` policy) documented in
`orchestrator-machine.md`. The PR's own commit message and a `TODO` it
replaces are explicit about this being deliberate, staged work:

> Both events enter recovery identically today. The checkpoint and kind
> fields let the SM skip retries on `DeviceFatal` or log the failing
> checkpoint without a second round-trip.

The old code this replaces literally carried a `TODO` making the same point
in the other direction (`driver/src/tests.rs`, pre-PR): *"the SM only knows
Timeout, so `DeviceFatal` still spends retry budget. Add a fatal,
unrecoverable-error event to the SM in a later PR."* PR #471 is that later
PR, for the *event*; the policy consumer (skip-retry-on-`DeviceFatal`) is
still a follow-up — `BootFailureKind` is diagnostic-only metadata for now, not
yet read by any handler beyond `Event::id()`.

## 5. Why `Event::BootFailed` needed to exist at all

Before this PR, every boot-progress failure — a hung device, a device that
actively reports "I failed," any of it — collapsed to one event,
`Event::Timeout(id)`, before it ever reached the SM. That collapsing happened
in the driver, not the SM: `WalkVerdict::Failed { .. }` (already
cause-tagged, at the `capabilities`/`walk` layer) was mapped to `Timeout(id)`
regardless of `cause`. The cause existed at the walk layer and was thrown
away one hop later.

`Event::BootFailed { id, checkpoint, kind }` moves that boundary one layer
further in: the cause (`TimedOut` / `DeviceRetriable` / `DeviceFatal`) and the
checkpoint name now cross into the SM's own event vocabulary
(`BootFailureKind` in `sm/src/model.rs`, deliberately *mirroring*
`capabilities::FailureCause` rather than importing it — the SM crate still
does not depend on `capabilities`, keeping the core's dependency graph as
thin as the platform-boundary design in `orchestrator-model.md` §6 requires).
This is the same "feedback as data" principle the machine doc names for
`RecoveryFailed`: information that used to vanish at a layer boundary is now
carried through explicitly, visible in the effect/event trace, so a later PR
can branch on it without re-plumbing the driver.

## 6. Two watchdogs, one event today

The tree currently has **two independent mechanisms** that can each end a
component's boot-wait and drive it into `Recovering`, and PR #471 only builds
one of them out further:

| | `BootWatchdogs` (`server`/`timer`) | `CheckpointWalk` (`adapters/walk`, this PR) |
|---|---|---|
| Granularity | one deadline per **component**, armed on `ReleaseReset` | one deadline per **checkpoint**, re-armed as the device progresses |
| Judges | elapsed time only | elapsed time **and** device-reported evidence (`BootStatus`) |
| Failure signal | `Event::Timeout(id)` | `Event::BootFailed { id, checkpoint, kind }` |
| Wiring status | mechanism complete, **not yet wired into a run loop** (`server/README.md` "Status") | wired into `PlatformDriver::poll_boot_walks`, exercised by the QEMU itest (scenarios 1–2) |
| SM handling | `Event::Timeout(id)` arm | `Event::BootFailed { id, .. } \| Event::Timeout(id)` — same arm, OR-matched |

The SM's OR-pattern (`Event::BootFailed { id, .. } | Event::Timeout(id) =>`)
is the seam where these two converge today. It reads as an explicit
placeholder for a fleet-level backstop timeout (`BootWatchdogs`, for whichever
board composition doesn't route boot-progress through a per-checkpoint
`EvidenceReader`) sitting alongside the richer, per-checkpoint path this PR
adds. Nothing in the PR removes or wires up `BootWatchdogs`; that remains the
open item named in `boot-watchdog-design-derivation.md` and
`server/README.md`, untouched by #471.

## 7. Relationship to the state machine's states

Concretely, in the topology from `orchestrator-machine.md`:

- `AwaitingReady --> Recovering : Timeout(id) [id == awaiting]` becomes, after
  this PR, also reachable via `BootFailed { id, .. } [id == awaiting]` — same
  target state, same effects (`RestoreGoldenImage`).
- The `SupervisingPlatform` superstate's fallback timeout handling (shared by
  `Ready`, `Updating`, `Recovering`, `AwaitingReady`) gets the same addition.
- Once in `Recovering`, nothing downstream (the retry cap, `FailurePolicy`
  branching, `RegionId` restore scope) changes — `BootFailed` is
  indistinguishable from `Timeout` past the entry edge, by design, until a
  follow-up PR teaches a handler to read `kind`.

No new states, no new transitions, no change to `ComponentAttrs`,
`FailurePolicy`, or the retry-cap arithmetic. This PR only enriches the
*input* to an edge that already existed.

## 8. What CheckpointWalk itself guarantees (from its own tests)

Worth noting because these are the invariants a future "differentiate on
`BootFailureKind`" PR will build against:

- **Deadlines are relative to first poll, not to `arm()`** — time between
  arming and the runtime's next tick doesn't count against the window.
- **A lapsed window beats a late-arriving pass** — if `now_millis >= deadline`
  the walk reports `Failed { TimedOut }` even if the device's status would
  read `Booted` on that same poll ("timeout checked before reading").
- **Read errors are silence, not failure** — an `Err` from `EvidenceReader`
  is treated as `BootStatus::Booting`; a transient bus glitch cannot fail a
  healthy boot.
- **`arm()` always rewinds to checkpoint 0**, including mid-walk — a retry
  re-release gets a fresh attempt from the start of the checkpoint list, not
  a resume.
- **Construction panics on an empty checkpoint list** — an unwatchable device
  is a build-time bug, not a runtime state to handle (mirrors
  `DeviceConfig::new`'s own compile-time check in `config`).

## 9. What's explicitly left for later

The PR is scoped tightly and says so in its own description and commit
bodies:

1. `BootFailureKind` is carried but not yet *read* by any SM handler beyond
   `Event::id()` — no retry-skipping on `DeviceFatal` yet.
2. `BootWatchdogs`/`TimerManager` (the coarser, fleet-level path) is not
   wired into an actual run loop; `CheckpointWalk` does not replace it, it
   coexists with it.
3. QEMU runtime itest scenarios 3–5 (`BootWatchdogs` multiplexing, the commit
   watchdog) are unchanged — only scenarios 1–2 were refactored onto
   `CheckpointWalk`.
4. The PR is stacked on #475 (a probe rename) — its own diff should be read
   as the last 3 commits only, per the PR description.

## 10. One-paragraph summary

PR #471 fills in the one concrete `BootWatch` implementation
(`CheckpointWalk`) that the orchestrator's capability layer had a trait for
but no board-usable code behind, closing the gap the verification-model doc
names as "aggregating intermediate signals into one `ComponentReady`." To
carry the extra information that aggregation now produces — *which*
checkpoint failed and *why* — it threads a new event, `Event::BootFailed`,
through the driver into the state machine, deliberately kept behaviorally
identical to the existing `Event::Timeout` for now (same transition, same
recovery path) so that a later PR can teach the SM to act on `BootFailureKind`
without this PR having to guess the right policy. It changes nothing about
the state topology, the recovery machinery, or the still-unwired fleet-level
`BootWatchdogs` path — it only makes one seam (per-device, per-checkpoint
boot supervision) real, richer, and tested.
