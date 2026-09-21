# Implementation plan: remove `statig` from `orchestrator_sm`

Hand-off spec for a fresh Claude Code instance. Assume **no prior context**. This
replaces the `statig` state-machine library with a hand-written reducer in one
file, changing **no observable behavior**. The existing test suite is the
correctness oracle and must pass **unchanged**.

---

## 0. Context (what this crate is)

`services/orchestrator/sm` is a `no_std`, `forbid(unsafe_code)` pure-reducer
state machine for an eRoT boot sequence. It describes side effects as `Effect`
data values (never performs them); a shell carries them out via a `Platform`
trait. Today the state transitions are expressed through the `statig` 0.4.1
blocking hierarchical-state-machine library. `statig` is used in **exactly one
file** — `services/orchestrator/sm/src/lib.rs` — and never leaks into the public
API. It is being introduced for the first time on this branch and has not yet
merged to `main`; the goal is to not onboard it as a dependency at all.

**Branch strategy:** do this on a scratch branch off the current
`orchestrator-sm` branch (e.g. `git switch -c orchestrator-sm-no-statig`) so the
two `lib.rs` versions can be diffed side by side before deciding.

---

## 1. Hard constraints (do not violate)

- `#![no_std]` and `#![forbid(unsafe_code)]` stay. No `alloc`.
- **No runtime panics** in library code. Overflow/never-happens branches are
  proved away at compile time (`const _: () = assert!(...)`) and their dead
  `Err`/`else` arms are dropped with `let _ = ...` plus a comment explaining why
  they are unreachable. Follow the pattern already in `Sink::emit` and
  `dispatch_with`. Do **not** introduce `unwrap`, `expect`, `panic!`,
  `unreachable!`, indexing that can panic, or arithmetic that can overflow.
- Rust edition 2024, nightly toolchain (see `rust-toolchain.toml`).
- **Public API must stay byte-for-byte compatible.** Public items:
  `Orchestrator`, `Orchestrator::new`, `Orchestrator::state`,
  `Orchestrator::dispatch`, `Orchestrator::dispatch_with`, `Platform`,
  `EffectError`, and everything re-exported from `model.rs` (`Effect`, `Event`,
  `State`, `ComponentId`, `ComponentAttrs`, `Chain`, `ChainError`, etc.).
- **Do not edit** `src/model.rs` or `src/tests.rs`. `tests.rs` is statig-free and
  is the oracle; if a change forces a test edit, the change is wrong.
- Do not commit markdown files. Do not stage `*.md`.
- Format with `rustfmt --edition 2024` before finishing.

---

## 2. The exact `statig` surface to remove

All in `src/lib.rs`:

1. Imports:
   ```rust
   use statig::Outcome;
   use statig::blocking::{
       IntoStateMachine, IntoStateMachineExt as _, State as StatigState, StateMachine,
       Superstate as StatigSuperstate,
   };
   ```
2. `pub enum Superstate<'sub> { SupervisingPlatform(PhantomData<&'sub ()>) }`
   and its `#[derive(Debug)]`.
3. `impl IntoStateMachine for Rot<N, E>` (defines `Event`/`Context`/`State`/
   `Superstate` assoc types and `fn initial() -> State { State::PowerOnReset }`).
4. `impl StatigState<Rot<N, E>> for State` — contains `call_handler`,
   `call_entry_action`, `superstate`.
5. `impl StatigSuperstate<Rot<N, E>> for Superstate<'_>` — contains the
   superstate `call_handler`.
6. In `Orchestrator`: the field `machine: StateMachine<Rot<N, E>>`, the
   `Rot::new(...).state_machine()` call in `new`, `self.machine.state()` in
   `state()`, and `self.machine.handle_with_context(&ev, &mut buf)` in
   `dispatch_with`.
7. `Outcome::{Handled, Transition, Super}` usages throughout the handler bodies.

Keep everything else unchanged: `Sink`, the `Rot` struct and all its helper
methods (`attrs_of`, `is_gated`, `bump_retry`, `clear_retry`,
`advance_to_next_ungated`, `gate_by_policy`, `handle_corruption`,
`cascade_hold`), the `EFFECT_CAP_OK` const assert, `PENDING_CAP` + its assert,
the `EffectError`/`Platform` definitions, and the `dispatch_with` settle loop
(only its inner call site changes). `use core::marker::PhantomData;` stays —
`Rot` still uses `PhantomData<[u8; E]>`.

---

## 3. Replacement design

### 3.1 Local `Outcome`

Define a crate-private enum to replace `statig::Outcome`:

```rust
/// Result of dispatching one event to a state (or its superstate).
enum Outcome {
    /// Event consumed; state unchanged; no entry action runs.
    Handled,
    /// Change to this state and run its entry action.
    Transition(State),
    /// Not handled here; defer to the superstate (or discard if none).
    Super,
}
```

### 3.2 Move the three `statig` match bodies to plain `Rot` methods

Convert the trait methods into inherent methods on `Rot<N, E>`. **The match
bodies are copied verbatim**; only the plumbing changes:

- `StatigState::call_handler(&mut self /* State */, rot, event, ctx)` becomes
  `fn handle(&mut self /* Rot */, state: State, event: &Event, ctx: &mut Sink<E>) -> Outcome`.
  Mechanical rename inside the body: the match subject `match self {` (the
  `&mut State`) becomes `match state {`, and every `rot.` becomes `self.` (the
  receiver is now the `Rot`). `Outcome::*` variants now refer to the local enum.
- `StatigSuperstate::call_handler` becomes
  `fn handle_supervising(&mut self /* Rot */, event: &Event, ctx: &mut Sink<E>) -> Outcome`.
  It has a single `SupervisingPlatform` arm today; drop the enum wrapper and keep
  the inner `match event { ... }` directly.
- `StatigState::call_entry_action(&mut self /* State */, rot, ctx)` becomes
  `fn entry_action(&mut self /* Rot */, state: State, ctx: &mut Sink<E>)`. Same
  `match self` → `match state` and `rot.` → `self.` rename.
- `StatigState::superstate` is replaced by a small predicate; the only
  superstate covers four states:
  ```rust
  fn is_supervised(state: State) -> bool {
      matches!(
          state,
          State::AwaitingReady | State::Ready | State::Updating | State::Recovering
      )
  }
  ```
  This mirrors the current `superstate()` mapping (those four return
  `Some(SupervisingPlatform)`; `PowerOnReset`, `PreSupervision`, `Locked` return
  `None`).

### 3.3 The dispatch engine (replaces `statig`'s runtime)

`Orchestrator` now owns the `Rot` and the current `State` directly:

```rust
pub struct Orchestrator<const N: usize, const E: usize> {
    rot: Rot<N, E>,
    state: State,
}
```

`new` builds the `Rot` and sets the initial state (statig's `initial()`):

```rust
pub fn new(chain: Chain<N>, max_retry: u8) -> Self {
    Self {
        rot: Rot::new(chain, max_retry),
        state: State::PowerOnReset,
    }
}

pub fn state(&self) -> State {
    self.state
}
```

Add one private method implementing statig's per-event semantics, and call it
from the `dispatch_with` settle loop in place of
`self.machine.handle_with_context(&ev, &mut buf)`:

```rust
fn step(&mut self, event: &Event, ctx: &mut Sink<E>) {
    // 1. Dispatch to the current (leaf) state.
    let mut outcome = self.rot.handle(self.state, event, ctx);
    // 2. On Super, defer to the superstate if this state has one; otherwise the
    //    event is discarded (statig behavior when superstate() is None).
    if let Outcome::Super = outcome {
        if Self::is_supervised(self.state) {
            outcome = self.rot.handle_supervising(event, ctx);
        }
    }
    // 3. Apply a transition: change state and run the target's entry action.
    //    (This machine defines no exit actions and no superstate entry/exit
    //    actions, so a transition's only side effect is the target leaf's entry
    //    action — faithful to statig here.) Handled / unhandled-Super do nothing.
    if let Outcome::Transition(target) = outcome {
        self.state = target;
        self.rot.entry_action(target, ctx);
    }
}
```

The `dispatch_with` inner call becomes:

```rust
let mut buf = Sink::<E>::new();
self.step(&ev, &mut buf);
```

`is_supervised` may be a `const fn` associated with `Orchestrator` or a free
function; keep it next to the engine.

---

## 4. Faithful-semantics checklist (the parts that can silently break)

Verify each against the golden tests; these are the behaviors statig gave you
for free that the engine must reproduce:

1. **Entry action runs only on `Transition`, never on `Handled`.** The
   `PreSupervision` cursor walk deliberately returns `Outcome::Handled` (not a
   self-transition) precisely so its entry action — which resets the cursor via
   `advance_to_next_ungated(ctx, 0)` — does **not** re-run. If you ever run entry
   actions on `Handled`, the chain walk breaks. Tests:
   `cold_boot_walks_chain_in_order`, `custom_capacity_walks_full_chain`.
2. **`Super` fallthrough routes to the superstate for supervised states and
   discards for the rest.** `EffectFailed` from `AwaitingReady`/`Ready`/
   `Updating`/`Recovering` must reach `SupervisingPlatform` and transition to
   `Locked`; from `Locked` it must be discarded (no superstate) so the machine
   does not loop. Tests: `effect_failure_latches_lockdown`,
   `failed_isolation_actuation_latches_lockdown`,
   `failed_restore_actuation_latches_lockdown`,
   `failed_lockdown_actuation_does_not_loop`, `locked_is_terminal`.
3. **Attestation + required-corruption handled across all four supervised
   states.** These live only in `handle_supervising`; the leaf states reach them
   via `Super`. Tests: `attestation_shared_across_supervising_platform_states`,
   `attestation_in_awaiting_ready`, `corruption_in_awaiting_ready_triggers_recovery`,
   `corruption_in_updating_triggers_recovery`.
4. **`PreSupervision` handles `CorruptionDetected` directly but discards
   `AttestationChallenge`** (it is intentionally *not* supervised). Tests:
   `corruption_during_presupervision_selfloop_triggers_recovery`.
5. **Transition entry effects land in the same `Sink` as the handler that caused
   the transition.** e.g. `Recovering` exhaustion → gate cascade (up to `N`
   `AssertReset`) in the handler + `Transition(PreSupervision)` whose entry emits
   `ReadFirmware`/`VerifyFirmware`, all in one buffer. The `E >= N + 2` bound
   depends on this. Test: `speculative_read_effects_are_emitted_together` plus
   the cascade tests.
6. **Initial entry action.** statig runs the initial state's entry action once
   before the first event; `PowerOnReset` has **no** entry action, so setting
   `state = PowerOnReset` in `new` without running anything is faithful. If a
   test regresses on the very first event, this is the suspect.
7. **The superstate handler's own outcome is applied** (it returns `Handled` or
   `Transition(Locked)`/`Transition(Recovering)`; it never needs a grandparent).
   The engine above applies its `Transition` correctly; a returned `Super` from
   the superstate is discarded (correct — `SupervisingPlatform` has no parent).

---

## 5. Dependency removal (do this LAST, after code + tests pass)

Keep `statig` available until the rewrite is green, then remove it:

1. `services/orchestrator/sm/BUILD.bazel`: delete the dep line
   `"@rust_crates//:statig",`.
2. `third_party/crates_io/Cargo.toml`: delete
   `statig = { version = "0.4.1", default-features = false }`.
3. Regenerate the crate lock so `statig` (and any transitive-only deps) leave
   `third_party/crates_io/Cargo.lock`. Use the repo's documented crate-universe
   repin process (check `CONTRIBUTING.md` / `MODULE.bazel` — likely a
   `CARGO_BAZEL_REPIN=1 bazelisk sync --only=rust_crates` style command or an
   equivalent `bazelisk run` repin target). Do not hand-edit `Cargo.lock` if a
   repin command exists.
4. Confirm no other crate references `statig` (there are none today):
   `grep -rn statig` across `services/`, `third_party/`, and `**/BUILD.bazel`
   should return nothing after removal.

---

## 6. Validation (the oracle)

Run from `/home/antrocha/work/apps/typeconstructor/openprot`:

1. Format:
   `rustfmt --edition 2024 services/orchestrator/sm/src/lib.rs`
2. Full test target (wraps all 42 unit tests; reports "1 test passes"):
   `bazelisk test //services/orchestrator/sm:orchestrator_sm_test --test_output=errors --nocache_test_results`
3. All tests must pass with **`tests.rs` unmodified**. If any test needs editing
   to pass, the reducer engine is not faithful — fix the engine, not the test.
4. Sanity: `grep -rn "statig" services/ third_party/ **/BUILD.bazel` is empty;
   `grep -n "unwrap\|expect\|panic!\|unreachable!" services/orchestrator/sm/src/lib.rs`
   shows only intended, commented dead-code-free constructs (ideally none).

**Operational note:** if a `bazelisk` command triggers an SSH passphrase prompt,
stop and surface it — do not attempt to answer it.

---

## 7. Deliverable

- A single-commit diff on the scratch branch touching only:
  `services/orchestrator/sm/src/lib.rs`,
  `services/orchestrator/sm/BUILD.bazel`,
  `third_party/crates_io/Cargo.toml`,
  `third_party/crates_io/Cargo.lock`.
- `model.rs` and `tests.rs` untouched.
- Suggested terse commit message:
  `orchestrator-sm: replace statig with hand-written reducer`
- Report back: the final `lib.rs` line count delta, confirmation the 42 tests
  pass unchanged, and any place where statig's semantics were non-obvious to
  reproduce (for the reviewer's attention).
```
