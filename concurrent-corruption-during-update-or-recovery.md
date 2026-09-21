# Concurrent corruption during update or recovery

## Question

Even if only one device is ever updated at a time, §5.3.3 background integrity
polling can report corruption of **device B** while **device A** is mid-update
or mid-recovery. What does the orchestrator reducer do then?

## Short answer

It depends entirely on **B's `FailurePolicy`**. One branch is clean; the other
has two sharp edges.

Both `Updating` and `Recovering` are *supervised* states
(`Orchestrator::is_supervised`), so a `CorruptionDetected(B)` in either falls
through the leaf handler to `handle_supervising` → `handle_corruption(B)`, which
dispatches on B's policy via `gate_by_policy`.

## Branch 1 — B is `Isolable` or `Cascading`: contained, no disruption

`handle_corruption` emits `AssertReset(B)` + `ReportIsolated(B)` (and, for
`Cascading`, the same for B's transitive dependents), adds them to the `gated`
set, and returns `Outcome::Handled`.

- The machine **stays exactly where it was** — A's update or recovery is
  untouched.
- Retry budgets are per-component (`bump_retry` is keyed by id), so isolating B
  does not perturb A's counters.
- A later re-walk skips gated B (`advance_to_next_ungated`).

This is the path the design is built for, and it is correct.

## Branch 2 — B is `Required` (or unknown): preemption, with two gaps

`handle_corruption` returns `Outcome::Transition(State::Recovering(B))`. The
machine drops what it was doing and recovers B. Prioritizing a required device
over an in-flight update/recovery is defensible, but two gaps surface, both
rooted in the single-slot `Recovering(ComponentId)` payload.

### Gap 1 — A mid-update (`Updating`)

The transition to `Recovering(B)` abandons the update with **no `DiscardStaged`**.

- A's `StageUpdate` was already actuated on `Updating` entry; nothing walks it
  back → the staged image is orphaned.
- After B restores, the machine re-walks `PreSupervision → … → Ready` and never
  returns to `Updating`. A later `UpdateVerified` / `UpdateRejected` for A lands
  in a state that does not handle it and is silently dropped → lost update
  outcome.

### Gap 2 — A mid-recovery (`Recovering(A)`)

The single-slot payload is overwritten A → B.

- A's recovery episode is silently discarded.
- The `Restored` handler ignores its id (`Event::Restored(_)`) and uses the
  state payload, so a `Restored(A)` arriving after the clobber is counted
  against **B's** retry budget and treated as B's restore.
- The machine cannot represent two concurrent recovery episodes:
  last-corruption-wins, first is lost.

## Root cause

Both gaps trace to the same two limitations:

1. **One recovery slot** — `State::Recovering` carries a single `ComponentId`.
2. **`Restored` is id-blind** — the handler matches `Restored(_)` and trusts the
   payload rather than the event's id.

Plus the absence of **update-preemption cleanup** (no `DiscardStaged` when
leaving `Updating` for a reason other than the update's own outcome).

The containable policies (`Isolable` / `Cascading`) mask all of this; a
`Required` B during an update or recovery is where it shows.

## Note on the `rot-reducer` (statig) machine

The `rot-reducer` `state-machine.md` describes the older statig machine, whose
`Operational` superstate does `CorruptionDetected(id) → Recovering` *unconditionally*
with no policy gating. That machine therefore **always** clobbers — even for the
isolable case — so it does not even get Branch 1's clean behavior.

## Possible fixes (if §5.3.3 can realistically fire on a `Required` device)

- Emit `DiscardStaged` on update preemption when leaving `Updating` for
  recovery.
- Make `Restored` id-checked (ignore a `Restored` whose id != the recovery
  target), **or** carry a small pending-recovery set so a second `Required`
  corruption does not overwrite the first.
