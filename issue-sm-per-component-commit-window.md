# orchestrator-sm: commit window is a single global `pending_commit`, under-models the CSA's per-component commit

## Summary

`orchestrator-sm` tracks the activated-but-not-committed state as one global
`pending_commit: bool` on `Rot`. The CSA models commit *per component* — each
component / recovery-region has its own anti-rollback SVN floor, committed
independently once that component proves healthy. A single boolean cannot record
*which* component the activation was for, so the model cannot bind a commit to
the component that was updated — it under-models the CSA's per-component commit.

## Current behavior

- `Rot` holds one `pending_commit: bool`.
- `State::Updating` + `Event::UpdateVerified` → emits `Effect::ActivateUpdate`,
  sets `pending_commit = true`, transitions to `Ready`.
- `State::Ready` + `Event::BootConfirmed(id)` → emits `Effect::CommitSvnFloor(id)`,
  sets `pending_commit = false`.
- `State::Ready` + `Event::CommitTimeout` (carries **no id**) → if
  `pending_commit` then `Locked`, else `Handled`.

## Problem

The flag and the timeout event are component-agnostic, but the underlying commit
is per-component. `Effect::ActivateUpdate` names no component, yet
`Effect::CommitSvnFloor(id)` is per-id, and the `BootConfirmed` handler binds
them by nothing:

1. **Spurious commit:** in steady-state `Ready` with no update in flight
   (`pending_commit == false`), a `BootConfirmed(id)` still emits
   `CommitSvnFloor(id)`, advancing a floor for an image that was never staged.
2. **Wrong-component commit / premature close:** with a window open, a
   `BootConfirmed(id)` for a *different* component than the one updated commits
   that component's floor and clears the single global flag, closing the real
   window — a later `CommitTimeout` then fails to lock even though the updated
   component never proved healthy.
3. **Undiagnosable timeout:** `CommitTimeout` carries no id and consults only the
   one bool, so it cannot tell *which* component's floor failed to commit. It can
   only make an all-or-nothing lock decision.
4. **Mismatch with existing granularity:** the retry budget is already per
   component (`ComponentStatus.retry` / `max_retry`); the commit window is the
   one place that regressed to a global.

A per-device commit shape is CSA-aligned; it is the sm's *single global* commit
machinery that under-models the CSA.

## Reachability

Updates are serialized — a second `UpdateRequest` while `Updating` is deferred,
and entering `State::Updating` clears `pending_commit` — so two components are
never in a commit window *simultaneously*. The under-modeling therefore does not
surface as two concurrent windows; it surfaces because the single global window
is not bound to the component the activation was for.

## Failure scenario

C0 and C1 are both up. An update is requested and activated (`ActivateUpdate`
names no component); the window opens. C1 — a component unrelated to the update —
reports `BootConfirmed(C1)`. The `Ready` handler emits `CommitSvnFloor(C1)`
(committing the wrong floor) and clears `pending_commit` (closing the window the
update was relying on). A subsequent `CommitTimeout` returns `Handled` instead of
latching `Locked`, so the updated image's floor is left in an unproven state.

## Proposed change

Move the commit window into per-component state and make the timeout
component-scoped:

- Replace `Rot.pending_commit: bool` with a per-component pending-commit bit on
  `ComponentStatus` (mirroring `retry`).
- Give `Event::CommitTimeout` a `ComponentId` (as boot `Timeout(id)` already
  has), and arm/track the commit deadline per component in the timer.
- `BootConfirmed(id)` clears only `id`'s bit and emits `CommitSvnFloor(id)`; a
  per-component commit timeout locks (or isolates per `FailurePolicy`) only that
  component.

## Acceptance criteria

- A commit is bound to the component whose update was activated: a
  `BootConfirmed(id)` for any other component (or with no update in flight) does
  not emit `CommitSvnFloor` and does not close the window.
- A commit timeout identifies the specific component and its lock/isolation
  decision is scoped to that component.
- Per-component commit state is durable across a return to `Ready`, reset only on
  `PowerOnReset` (same lifecycle as `ComponentStatus`).

## Tests to add

Both fail against the current sm (they encode the fixed behavior), so they can
land first as the regression specs the change must satisfy. They use the
existing `drive(chain, script) -> (Vec<Effect>, State)` harness in
`services/orchestrator/sm/src/tests.rs`.

- **Spurious commit is refused.** From steady-state `Ready` with no update in
  flight, `BootConfirmed(id)` must not emit `CommitSvnFloor(id)`.
- **A commit binds to the activated component.** With a window open, a
  `BootConfirmed(id)` for a *different* component must not emit
  `CommitSvnFloor(id)` and must not close the window — a following
  `CommitTimeout` must still latch `Locked`.
- **Per-component timeout scoping.** Once `CommitTimeout` carries an id, a
  timeout for the component whose window is open latches `Locked`, while a
  timeout for a component with no open window is dropped (`Handled`).
