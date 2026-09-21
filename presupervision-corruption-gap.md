# The `PreSupervision` corruption gap — a long-form writeup

This document walks through, from first principles, a gap discovered while
studying `services/orchestrator/sm/src/lib.rs`: `CorruptionDetected` events
targeting an already-released component can be silently discarded while the
machine is still walking the rest of a multi-component chain. It explains the
architecture that leads to this, why it's easy to miss, how it was confirmed
with a test, and what the possible resolutions are.

---

## 1. Background: two "modes," one superstate

The orchestrator state machine has seven states:
`PowerOnReset`, `PreSupervision`, `AwaitingReady`, `Ready`, `Updating`,
`Recovering`, `Locked`.

Four of them — `Ready`, `Updating`, `Recovering`, `AwaitingReady` — share a
**superstate** called `SupervisingPlatform`. In `statig` (the hierarchical
state machine library this code uses, without proc-macros), a superstate is a
fallback handler: if a leaf state's `call_handler` returns `Outcome::Super`,
`statig` walks up to that leaf's declared parent (via `fn superstate(&mut self)
-> Option<Superstate<'_>>`) and tries the parent's handler instead.

`SupervisingPlatform` centralizes exactly two rules that must hold identically
across all four of its child states:

```rust
/// - Attestation challenges are always answered.
/// - Corruption of a required component always triggers recovery.
```//services/orchestrator/sm/src/lib.rs:274-275

The design docs (`orchestrator-machine.md`) describe this as the "supervision
contract," and frame the machine's lifecycle as having two mutually exclusive
**modes**:

- **Mode one** — `PreSupervision`: supervision contract *off*. The eRoT walks
  the trust chain, verifying and releasing components, but does not yet answer
  attestation challenges or act on corruption.
- **Mode two** — `SupervisingPlatform` (i.e. one of its four child states):
  supervision contract *on*. Attestation is always answered; corruption always
  triggers recovery.

Critically: `PreSupervision`'s own `superstate()` implementation returns `None`:

```rust
fn superstate(&mut self) -> Option<Superstate<'_>> {
    match self {
        State::Ready | State::Updating | State::Recovering | State::AwaitingReady => {
            Some(Superstate::SupervisingPlatform(PhantomData))
        }
        _ => None,
    }
}
```

So `PreSupervision` (along with `PowerOnReset` and `Locked`) is **not** linked
to `SupervisingPlatform` at all. Any event `PreSupervision` doesn't explicitly
handle falls all the way to the top level and is discarded — there is no
parent to catch it.

---

## 2. Where the docs say supervision begins

`orchestrator-sm-walkthru.md` makes an explicit claim about *when* the
supervision contract must start:

> The threshold is one firmware verification result — passed or failed — not
> all components having passed verification. CSA does not define a safe window
> before supervision begins. A remote verifier may issue an attestation
> challenge as soon as the first component's measurements exist. A corruption
> can be detected the moment a component is running. **The supervision
> contract — attestation always answered, corruption always acted on — must
> therefore hold continuously from the first result onward**, including during
> recovery before anything has been released. Deferring supervision until
> `Ready` would leave a gap that CSA does not permit.

And the code comment on `Superstate` echoes this:

> Superstate entered on the eRoT's first component release and held until
> `State::Locked`.

Both of these claims are stronger than what the code actually does. They
describe supervision as continuous from the *first result/release* onward.
But that's only true if the *first* verification result immediately moves the
machine out of `PreSupervision` and into a `SupervisingPlatform` child state —
which is **not guaranteed** when the chain has more than one component.

---

## 3. Walking through the actual transition table

Look at `PreSupervision`'s handler (`orchestrator-machine.md`, mirrored exactly
in `lib.rs`):

| Event | Guard | Effects | Next state |
|---|---|---|---|
| `VerificationPassed(id)` | more, current `Passive` | `ReleaseReset` · `ReadFirmware(next)` · `VerifyFirmware(next)` | `Handled` (cursor ++, **stays in `PreSupervision`**) |
| `VerificationPassed(id)` | more, current `Active` | `ReleaseReset` · `ReadFirmware(next)` · `VerifyFirmware(next)` | `AwaitingReady` |
| `VerificationPassed(id)` | chain done | `ReleaseReset(id)` | `Ready` |
| `VerificationFailed(id)` | — | — | `Recovering` |
| anything else | — | — | `Outcome::Super` (top level — **discarded**) |

Take a three-component, all-`Passive` chain: `C0`, `C1`, `C2`.

1. Machine boots, enters `PreSupervision`.
2. `VerificationPassed(C0)` fires. `C0` is released (`ReleaseReset(C0)`). More
   components remain, and `C0` is `Passive`, so the machine takes the
   `Handled` branch: cursor advances to `C1`, **but the machine stays in
   `PreSupervision`.**
3. At this exact moment, `C0` is a released, running component — exactly the
   kind of component the supervision contract is supposed to protect. If `C0`'s
   firmware becomes corrupted right now (say, a runtime integrity monitor
   fires `CorruptionDetected(C0)`), what happens?
4. The machine is in `PreSupervision`. `CorruptionDetected` isn't one of the
   events `PreSupervision` explicitly handles, so it falls to `anything else`
   → `Outcome::Super`. `PreSupervision`'s `superstate()` returns `None`. There
   is nowhere further to fall. **The event is silently dropped.** No
   `RestoreGoldenImage` effect. No transition to `Recovering`. `rot.failed` is
   never set. The machine continues on to verify `C1` and `C2` as if nothing
   happened.

This is the gap: for any chain with **two or more components**, there is a
window — from the moment the first component is released until the moment the
*entire* chain finishes (`VerificationPassed` on the *last* component, which is
the only path that reaches `Ready`) — during which `CorruptionDetected` events
targeting already-released components are unconditionally discarded.

For a single-component chain, this window doesn't exist: the first
`VerificationPassed` is simultaneously the *last* one, so the machine jumps
straight to `Ready` and the gap never opens.

---

## 4. Why this is easy to miss

A few things about the design conspire to hide this:

- **The self-loop looks harmless.** `PreSupervision --> PreSupervision` reads
  like "still walking, nothing has changed yet." But something *has* changed:
  a component was just released and is now running untrusted-until-corrupted
  code, which is exactly the situation the supervision contract exists for.

- **The tests don't cover it.** Every existing corruption test
  (`corruption_in_awaiting_ready_triggers_recovery`,
  `corruption_in_updating_triggers_recovery`,
  `runtime_corruption_targets_component_and_rewalks`,
  `required_runtime_corruption_triggers_recovery`) fires `CorruptionDetected`
  only *after* the chain has already fully advanced into `Ready`,
  `AwaitingReady`, or `Updating`. None of them fire it while a multi-component
  chain is still self-looping inside `PreSupervision`. The all-`Passive`,
  two-component test (`runtime_corruption_targets_component_and_rewalks`) uses
  `VerificationPassed(C0)` *and* `VerificationPassed(C1)` before the
  `CorruptionDetected` — by then the chain is done and the machine has already
  moved to `Ready`. So the exact window described above was never exercised.

- **The docs assert continuity in prose, not in a table.** The "threshold"
  paragraph is a narrative claim about intent; nothing in the transition
  tables cross-checks it. Nothing forces `PreSupervision`'s per-row behavior
  to be re-derived from that paragraph.

- **`AwaitingReady` masks the same shape of problem for `Active` components.**
  For an `Active` component, the *next* `VerificationPassed` moves the machine
  to `AwaitingReady` — which *is* linked to `SupervisingPlatform`. So chains
  that hit at least one `Active` component before the end will "accidentally"
  close the gap for any corruption reported after that point. The gap is
  specifically about runs of two or more trailing `Passive` components, which
  is a plausible and common configuration (e.g. a chain of simple symbiont
  devices with no integrated RoT).

---

## 5. Confirming it with a test

To turn the hypothesis into a fact, this test was added directly beside the
existing corruption tests in `lib.rs`:

```rust
#[test]
fn corruption_during_presupervision_selfloop_is_dropped() {
    let (effects, state) = drive(
        passive_required(&[C0, C1, C2]),
        &[
            BOOT,
            Event::VerificationPassed(C0), // released; walk continues (still PreSupervision)
            Event::CorruptionDetected(C0), // C0 already released
        ],
    );
    assert_eq!(state, State::PreSupervision);
    assert!(!effects.contains(&Effect::RestoreGoldenImage(C0)));
}
```

Running it (`bazelisk test //services/orchestrator/sm:orchestrator_sm_test
--test_filter=corruption_during_presupervision_selfloop_is_dropped`) passes:
the machine stays in `PreSupervision` and no `RestoreGoldenImage` effect is
ever emitted. The corruption event on `C0` had zero observable effect on the
machine. This confirms the gap is real, reproducible, and currently silent
(no panic, no error — the event is just dropped, which is arguably the most
dangerous kind of gap, since nothing signals that anything went wrong).

---

## 6. Is this a bug, or an intentional simplification?

This is genuinely ambiguous without more context on the shell/orchestrator
contract, and depends on what `CorruptionDetected` is actually supposed to
represent:

**Argument that it's a real bug:**
- The code's own doc comment says "corruption of a required component always
  triggers recovery" with no qualifier about which states this applies to
  beyond "all four sub-states" of `SupervisingPlatform" — but the design intent
  described in the walkthrough doc is broader than that: continuous from first
  release.
- CSA's core principle (echoed repeatedly in the docs) is "no component
  executes unverified firmware" — but the flip side, implicit in that
  principle, is that a component that *becomes* corrupted after being verified
  and released must not be allowed to keep running unnoticed either. A
  released-but-then-corrupted component sitting invisible to the state machine
  for the rest of a multi-component boot walk is precisely the risk that
  principle is meant to close off.
- It only takes one `Active` component being absent from the tail of the chain
  for this to become a real, exploitable-in-principle window, not just a
  theoretical corner case.

**Argument that it's intentional / acceptable:**
- Maybe `CorruptionDetected` is defined (in the shell's actual event-producing
  logic, outside this crate) to only ever be emitted once the platform is
  fully in a "supervising" state — i.e., the shell itself gates when it will
  ever send this event, and it simply never does so mid-walk. If that's a
  hard shell-side guarantee, the state machine's behavior here is dead code
  protecting against an event that structurally cannot occur.
- The re-walk-from-scratch design elsewhere in the machine (`Recovering -->
  PreSupervision`, which always restarts verification at `cursor = 0`) already
  reflects a philosophy of "full re-verification trumps incremental
  correctness" — one could argue that a corruption on an already-passed
  component will eventually be caught if it also fails to boot fully / trips
  something else, though this is a much weaker guarantee than closing the gap
  directly.

Neither of these can be confirmed by reading `lib.rs` alone — it requires
knowing the actual shell implementation's guarantees about when
`CorruptionDetected` can fire.

---

## 7. Possible resolutions, ranked by invasiveness

1. **Do nothing, document the limitation.** Add a note to
   `orchestrator-machine.md` under `PreSupervision` explicitly stating that
   `CorruptionDetected` is not handled while still walking a multi-component
   chain, and that this relies on the shell never emitting it during that
   window. Cheapest, but leaves the actual behavior unchanged.

2. **Link `PreSupervision` to `SupervisingPlatform`.** Change its
   `superstate()` to return `Some(Superstate::SupervisingPlatform(...))`
   instead of `None`. This is the most direct fix — it makes supervision
   continuous from the *first release* exactly as the docs already claim.
   Requires checking: does this change any *other* event's behavior in
   `PreSupervision`? `AttestationChallenge` would now also be answered
   mid-walk (probably desirable, and consistent with the "measurements can be
   collected during this sequence" CSA quote), and `CorruptionDetected` would
   route to `Recovering`. Need to re-verify this doesn't conflict with
   `PreSupervision`'s own `VerificationFailed` handling (a different event, so
   no direct clash) or introduce any new invariant violations — this would
   need new/updated tests, including the one added in §5 above (whose
   assertion would need to flip).

3. **Add an explicit `CorruptionDetected` row to `PreSupervision`'s own
   table**, handling it directly rather than via the superstate. Functionally
   similar to option 2 but keeps `PreSupervision` self-contained instead of
   inheriting the shared superstate rule — arguably less consistent with the
   "single source of truth" rationale in the "Centralizing the Supervision
   Contract" section of `orchestrator-machine.md`, which specifically argues
   *against* copy-pasting event handling into individual states.

Option 2 is the most consistent with the documented design intent and the
project's own stated preference (avoid duplicating the corruption rule per
state) — but it's a behavioral change to the core state machine, not just a
docs fix, so it needs sign-off before implementing, plus a broader test pass
to make sure nothing else assumes `PreSupervision` never routes to
`Recovering` via the superstate path.

---

## 8. Summary

- `PreSupervision` is architecturally isolated from `SupervisingPlatform`
  (`superstate()` returns `None`).
- For chains with 2+ components, there's a window — after the first
  component is released, before the whole chain finishes — during which
  `CorruptionDetected` on an already-released component is silently dropped.
- This contradicts the documented claim that supervision is continuous "from
  the first result onward."
- Confirmed via a new test,
  `corruption_during_presupervision_selfloop_is_dropped`, added to
  `services/orchestrator/sm/src/lib.rs`.
- Whether this is a bug or an intentional (if under-documented) simplification
  depends on external shell-level guarantees not visible in this crate alone.
- The most architecturally consistent fix, if it is deemed a bug, is linking
  `PreSupervision` into the `SupervisingPlatform` superstate rather than
  special-casing the event locally.
