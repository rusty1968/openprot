# Design plan: fold `awaiting` and `failed` into `State` variants

Hand-off spec for a fresh Claude Code instance. Assume **no prior context**.
This is a targeted refactor of the `orchestrator_sm` reducer that moves two
state-coupled fields out of the shared `Rot` storage and into the `State` enum
variants they belong to, so the type system enforces "this datum exists only in
this state." It is **behavior-preserving** with respect to emitted `Effect`s;
the only observable change is that `State` (returned by `Orchestrator::state`)
gains payloads.

Work on a scratch branch off `orchestrator-sm-no-statig` (e.g.
`git switch -c orchestrator-sm-state-payloads`).

---

## 1. Motivation (the "split state" problem)

Today the machine's real state is `State` (a fieldless enum) **times** several
side fields in `Rot`: `cursor`, `gated`, `retries`, `failed`, `awaiting`.
Invariants like "`awaiting` is meaningful only in `AwaitingReady`" and "`failed`
is always present while `Recovering`" are enforced by **discipline and tests**,
not by types. That is the single most likely home for a future bug (a handler
that leaves `awaiting` set after leaving `AwaitingReady`, or reads `failed` in
the wrong state).

Two of those fields are cleanly coupled to exactly one state each and should
move into that state's variant:

- `failed: Option<ComponentId>` → `State::Recovering(ComponentId)`. `failed` is
  set on the transition **into** `Recovering`, read by `Recovering`'s entry
  action and its `Restored` handler, and cleared on the way **out**. It is
  logically always-present while `Recovering` — so the payload is a bare
  `ComponentId`, and the `Option` disappears entirely.
- `awaiting: Option<ComponentId>` → `State::AwaitingReady(Option<ComponentId>)`.
  See §4 — this one keeps an `Option` **inside** the variant (it cannot be
  eliminated without a behavior change), but relocating it still enforces
  "`awaiting` exists only in `AwaitingReady`."

`cursor`, `gated`, and `retries` **stay in `Rot`**: they genuinely span multiple
states (the cursor and gate set persist across `PreSupervision`↔`AwaitingReady`↔
`Recovering`↔`Ready` walks; retry counts persist across recovery episodes). Do
not touch them.

---

## 2. Hard constraints (unchanged from the crate's conventions)

- `#![no_std]`, `#![forbid(unsafe_code)]`, no `alloc`.
- **No runtime panics** in library code. No `unwrap`/`expect`/`panic!`/
  `unreachable!`/panicking indexing/overflowing arithmetic. Keep the existing
  compile-time-assert + dead-`let _ =` idioms.
- Rust edition 2024, nightly toolchain.
- `State` must remain `#[derive(Clone, Copy, PartialEq, Eq, Debug)]` and
  `#[non_exhaustive]`. This is satisfied automatically: `ComponentId` is `Copy +
  Eq`, so `Option<ComponentId>` and `ComponentId` payloads keep `State: Copy +
  Eq`.
- Do **not** commit markdown.
- Format with `rustfmt --edition 2024` before finishing.

---

## 3. Decision: payloads on the public `State` (Option A) vs. internal phase (Option B)

**This refactor changes `State`, which is public and observable via
`Orchestrator::state`.** Two ways to do it:

- **Option A (recommended, and what this plan specifies): put the payloads on
  the public `State` enum.** Simpler (one enum, no mapping). `state()` now
  returns e.g. `State::Recovering(C0)`, which is *richer* telemetry, not a
  regression. Cost: ~10 assertions in `tests.rs` that compare against the
  fieldless variants must change (see §6). The crate is pre-merge with one
  consumer (the tests), so the churn is contained and mechanical, and the
  **effect-trace** assertions — the real oracle — do not change.
- **Option B (fallback, only if the public `State` contract must be frozen):**
  keep `State` fieldless for the external API; introduce a crate-private
  `enum Phase { …, AwaitingReady(Option<ComponentId>), Recovering(ComponentId),
  … }` used by the engine, handlers, and `Orchestrator.state`, and implement
  `fn state(&self) -> State` mapping `Phase → State` by dropping payloads. This
  preserves `tests.rs` byte-for-byte and the public API, at the cost of two
  parallel enums to keep in sync. The type-safety win is then internal-only.

Proceed with **Option A** unless told otherwise. The rest of this plan assumes A;
§7 notes the deltas for B.

---

## 4. Why `AwaitingReady` keeps an `Option` (do not "simplify" it away)

`AwaitingReady` has a legitimate "awaiting nothing" sub-state in the current
code. In `AwaitingReady`'s `ComponentReady` handler:

```rust
self.awaiting = None;
if (self.cursor as usize) >= self.chain.len() {
    Outcome::Transition(State::Ready)
} else {
    Outcome::Handled            // stays in AwaitingReady, awaiting now None
}
```

When the awaited active component reports ready but the chain isn't finished, the
machine **stays in `AwaitingReady` with `awaiting == None`** (readiness satisfied;
now draining speculative verifications, during which any further `ComponentReady`
is spurious). That "satisfied/draining" mode is real and must be representable,
so the faithful payload type is `Option<ComponentId>`:

- `AwaitingReady(Some(id))` — awaiting `id`'s readiness.
- `AwaitingReady(None)` — readiness satisfied, continuing the supervised walk.

Relocating the `Option` into the variant still delivers the invariant we want
(it cannot exist in `Ready`/`Updating`/`Recovering`/etc.). **Do not** try to
eliminate the `None` case by adding a new state or changing when the machine
leaves `AwaitingReady` — that is a behavior change and out of scope. It was
considered and rejected here on purpose.

---

## 5. Exact edits in `lib.rs` and `model.rs`

### 5.1 `model.rs` — the `State` enum

Change the two variants (keep every other variant, the derives, and
`#[non_exhaustive]`):

```rust
pub enum State {
    PowerOnReset,
    PreSupervision,
    AwaitingReady(Option<ComponentId>),
    Ready,
    Updating,
    Recovering(ComponentId),
    Locked,
}
```

`ComponentId` is already defined in `model.rs`, so it is in scope. Update the
doc comments on those two variants to describe the payloads.

### 5.2 `lib.rs` — `Rot` struct and `Rot::new`

Delete the two fields and their initializers:

- In `struct Rot`: remove `failed: Option<ComponentId>,` and
  `awaiting: Option<ComponentId>,` (and their doc comments).
- In `Rot::new`: remove `failed: None,` and `awaiting: None,`.

Keep `chain`, `cursor`, `gated`, `retries`, `max_retry`, `_effect_cap`.

### 5.3 `lib.rs` — `handle_corruption`

`failed` is no longer a field; the recovery target rides the transition:

```rust
fn handle_corruption(&mut self, id: ComponentId, ctx: &mut Sink<E>) -> Outcome {
    match self.gate_by_policy(ctx, id) {
        Gating::Gated => Outcome::Handled,
        Gating::NotGated => Outcome::Transition(State::Recovering(id)),
    }
}
```

### 5.4 `lib.rs` — `handle`, `State::PreSupervision` arm

`VerificationPassed`, Active branch — carry `awaiting` in the transition:

```rust
Some(ComponentKind::Active) => {
    Outcome::Transition(State::AwaitingReady(Some(*id)))
}
```
(delete the `self.awaiting = Some(*id);` line above it.)

`VerificationFailed` — carry `failed` in the transition:

```rust
Event::VerificationFailed(id) => {
    Outcome::Transition(State::Recovering(*id))
}
```
(delete the `self.failed = Some(*id);` line.)

### 5.5 `lib.rs` — `handle`, `State::AwaitingReady` arm

Bind the payload in the match arm and thread it through. **Key mechanic:** to
*change* the payload you must return `Outcome::Transition(State::AwaitingReady(
…))` — `Outcome::Handled` leaves the state (and thus the payload) unchanged.
`AwaitingReady` has no entry action, so a same-variant transition is
behavior-identical to the old "field write + `Handled`".

```rust
State::AwaitingReady(awaiting) => match event {
    Event::ComponentReady(id) => {
        if awaiting != Some(*id) {
            return Outcome::Handled; // spurious / stale (INV9)
        }
        if (self.cursor as usize) >= self.chain.len() {
            Outcome::Transition(State::Ready)
        } else {
            // Readiness satisfied, chain not done: stay supervised but now
            // awaiting nothing.
            Outcome::Transition(State::AwaitingReady(None))
        }
    }
    Event::VerificationPassed(id) => {
        self.clear_retry(*id);
        ctx.emit(Effect::ReleaseReset(*id));
        let next_idx = (self.cursor as usize).saturating_add(1);
        if self.advance_to_next_ungated(ctx, next_idx) {
            Outcome::Handled            // payload unchanged — faithful
        } else {
            Outcome::Transition(State::Ready)
        }
    }
    Event::VerificationFailed(id) => {
        Outcome::Transition(State::Recovering(*id))
    }
    _ => Outcome::Super,
},
```

Notes:
- `awaiting` is bound by value (`Option<ComponentId>`, `Copy`).
- The `VerificationPassed` arm keeps `Outcome::Handled`, which preserves the
  current payload — this matches today's behavior (awaiting was only cleared by
  `ComponentReady` or `VerificationFailed`, never by `VerificationPassed`).
- `awaiting` is used in the `ComponentReady` arm, so the binding is not "unused."

### 5.6 `lib.rs` — `handle`, `State::Recovering` arm

`failed` is now the bound payload — always present, so the defensive
`.unwrap_or(self.max_retry)` branch and the `self.failed = None` bookkeeping
**disappear** (a real dead-path removal):

```rust
State::Recovering(failed) => match event {
    Event::Restored(_) => {
        // Count this attempt against the specific component in recovery, not a
        // global budget (CSA: exhaustion is per-device).
        let attempts = self.bump_retry(failed);
        if attempts < self.max_retry {
            Outcome::Transition(State::PreSupervision)
        } else {
            // Retries exhausted: gate via the same `gate_by_policy` the
            // runtime-corruption path uses. Gated → continue the walk;
            // NotGated (Required/unknown) → lock down.
            match self.gate_by_policy(ctx, failed) {
                Gating::Gated => {
                    self.clear_retry(failed);
                    Outcome::Transition(State::PreSupervision)
                }
                // `Required`, or an unknown/missing id: lock down.
                Gating::NotGated => {
                    ctx.emit(Effect::Emit(Event::RecoveryFailed));
                    Outcome::Handled
                }
            }
        }
    }
    Event::RecoveryFailed => Outcome::Transition(State::Locked),
    _ => Outcome::Super,
},
```

Update the stale comment that said "`failed` is always `Some` … treat a missing
id as exhausted defensively" — the type now guarantees presence.

### 5.7 `lib.rs` — `entry_action`

- `State::PreSupervision`: delete `self.awaiting = None;` (no such field now).
  The arm becomes just `let _ = self.advance_to_next_ungated(ctx, 0);`.
- `State::Recovering` → `State::Recovering(failed)`; the body no longer needs the
  `if let Some`:
  ```rust
  State::Recovering(failed) => {
      ctx.emit(Effect::RestoreGoldenImage(failed));
  }
  ```
- `State::Ready`: delete `self.failed = None;` (keep `self.retries.clear();`).
- `State::AwaitingReady` has no entry action; it stays in the `_ => {}` arm. Do
  **not** add one (adding an entry action would change transition semantics for
  the same-variant `AwaitingReady(None)` transition in §5.5).

### 5.8 `lib.rs` — `Orchestrator::is_supervised`

Match the new variant shapes:

```rust
const fn is_supervised(state: State) -> bool {
    matches!(
        state,
        State::AwaitingReady(_) | State::Ready | State::Updating | State::Recovering(_)
    )
}
```

### 5.9 `lib.rs` — everything else

`Orchestrator::new` (`state: State::PowerOnReset`), `state()`, `step`, and
`dispatch_with` need **no** logic changes — they move `State` values around
opaquely. `step` binds `Outcome::Transition(target)` and passes `target` (still
`Copy`) to `entry_action`; that continues to work with payloaded variants.

---

## 6. `tests.rs` changes (Option A only)

The **only** test edits are the state-equality assertions against the two
now-payloaded variants. Effect-trace assertions do **not** change. Replace exact
equality with a pattern match (closest to the original intent, which did not care
about the id):

- `assert_eq!(state, State::AwaitingReady);`
  → `assert!(matches!(state, State::AwaitingReady(_)));`
- `assert_eq!(orch.state(), State::AwaitingReady);`
  → `assert!(matches!(orch.state(), State::AwaitingReady(_)));`
- `assert_eq!(state, State::Recovering);`
  → `assert!(matches!(state, State::Recovering(_)));`

Affected tests (verify by compiling — the list should be exhaustive):
`active_component_gates_on_component_ready`, `spurious_component_ready_is_ignored`,
`attestation_in_awaiting_ready`, `speculative_read_effects_are_emitted_together`,
`corruption_during_presupervision_selfloop_triggers_recovery`,
`boot_failure_required_enters_recovering`,
`required_failure_in_awaiting_ready_enters_recovering`,
`corruption_in_awaiting_ready_triggers_recovery`,
`corruption_in_updating_triggers_recovery`,
`required_runtime_corruption_triggers_recovery`.

Optional strengthening (nice, not required): where the awaited/failed id is known,
assert it precisely — e.g. `assert_eq!(state, State::Recovering(C0));` in
`boot_failure_required_enters_recovering` — to lock in the payload. Do this only
where it reads clearly; otherwise prefer `matches!`.

Do not touch any other line of `tests.rs`.

---

## 7. Option B deltas (only if chosen)

- Add crate-private `enum Phase { PowerOnReset, PreSupervision,
  AwaitingReady(Option<ComponentId>), Ready, Updating, Recovering(ComponentId),
  Locked }` (derive `Clone, Copy, PartialEq, Eq, Debug`).
- Use `Phase` everywhere §5 uses payloaded `State`: `Rot::handle`,
  `handle_supervising`, `entry_action`, `handle_corruption` return `Outcome`
  over `Phase`, `Orchestrator.state: Phase`, `is_supervised(Phase)`, and the
  local `Outcome::Transition(Phase)`.
- Keep `State` fieldless and unchanged in `model.rs`.
- Implement `fn state(&self) -> State` mapping `Phase → State` (drop payloads).
- `tests.rs` is unchanged. The public API is unchanged.

---

## 8. Validation

Run from `/home/antrocha/work/apps/typeconstructor/openprot`:

1. `rustfmt --edition 2024 services/orchestrator/sm/src/lib.rs services/orchestrator/sm/src/model.rs services/orchestrator/sm/src/tests.rs`
2. `bazelisk test //services/orchestrator/sm:orchestrator_sm_test --test_output=errors --nocache_test_results`
3. All tests must pass. For Option A, the *only* diff in `tests.rs` is the state
   assertions in §6; `model.rs`/`lib.rs` effect logic is behavior-preserving, so
   every effect-trace assertion must pass untouched. If an effect-trace test
   fails, a transition dropped or duplicated a payload — most likely a
   `Handled`-vs-`Transition` slip in §5.5.
4. Sanity: `grep -n "self.failed\|self.awaiting\|rot.failed\|rot.awaiting" services/orchestrator/sm/src/lib.rs`
   returns nothing (both fields fully removed).
5. `grep -n "unwrap\|expect\|panic!\|unreachable!" services/orchestrator/sm/src/lib.rs`
   shows no new panics.

**Operational note:** if a `bazelisk` command triggers an SSH passphrase prompt,
stop and surface it — do not answer it.

---

## 9. Risk / gotcha checklist

1. **`Handled` cannot mutate a payload.** Any place that used to write
   `self.awaiting = …` while *staying* in `AwaitingReady` must become a
   same-variant `Transition(AwaitingReady(new))`. This is safe only because
   `AwaitingReady` has no entry action — do not add one. (§5.5, §5.7.)
2. **Do not eliminate `AwaitingReady(None)`.** It is the real "readiness
   satisfied, draining" mode. (§4.)
3. **`Recovering(_)` payload is always present** — delete, don't preserve, the
   old defensive `unwrap_or(max_retry)` and `failed = None` lines. (§5.6, §5.7.)
4. **`cursor`, `gated`, `retries` stay in `Rot`.** They span states; folding them
   in is out of scope and would be wrong.
5. **`State` stays `Copy + Eq + non_exhaustive`.** Guaranteed by `ComponentId`
   being `Copy + Eq`; if a derive error appears, check `ComponentId`'s derives
   rather than removing a derive from `State`.
6. **Behavior preservation is on effects, not on `state()`.** `state()` output
   changes shape by design (Option A); the effect traces do not. That asymmetry
   is expected and is why only the §6 assertions change.

---

## 10. Deliverable

- A single-commit diff touching `services/orchestrator/sm/src/model.rs`,
  `services/orchestrator/sm/src/lib.rs`, and (Option A) the §6 assertions in
  `services/orchestrator/sm/src/tests.rs`. Option B leaves `tests.rs` untouched.
- Suggested terse commit message:
  `orchestrator-sm: fold awaiting/failed into State variants`
- Report back: the net field/line delta on `Rot`, confirmation the effect-trace
  tests passed unchanged, and the exact list of `tests.rs` assertions edited.
```
