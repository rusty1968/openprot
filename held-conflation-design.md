# Design note: splitting `held` — durable gating vs. per-walk skip

*Status: draft for reflection. Not committed. From the orchestrator-sm
skeptical review.*

---

## 1. Problem statement

`Rot` tracks components that the walk must not touch in a single field:

```rust
/// Components skipped because their recovery was exhausted under
/// `FailurePolicy::Isolable` or `Cascading`. Held in reset; not
/// re-verified on subsequent re-walks. Cleared on `Ready` entry.
held: heapless::Vec<ComponentId, N>,
```

That one field is asked to mean **two different things at once**:

1. **"Physically gated"** — the core has emitted `Effect::AssertReset(id)` and the
   component is held in reset *as a hardware fact*. This is durable: it must stay
   true until something explicitly restores/releases the component. It is a
   trust-boundary decision ("this part is known-bad; keep it dark").

2. **"Skip on the current walk"** — bookkeeping for the in-progress **chain
   walk** (one top-to-bottom traversal, started by entering `PreSupervision`),
   telling `advance_to_next_unheld` not to re-verify this id this time around.
   This is naturally scoped to a single walk.

The two meanings have **opposite lifetimes**, but they share one `Vec` with a
single clearing rule. `held` is cleared on `Ready` entry:

```rust
State::Ready => {
    rot.retry_count = 0;
    rot.held.clear();   // <-- clears meaning (1) as if it were meaning (2)
    rot.failed = None;
}
```

Clearing at `Ready` is correct for meaning (2) (the next walk starts fresh) but
**wrong for meaning (1)**: reaching `Ready` does not un-assert anyone's reset
line. The hardware is still holding those components down; the core just *forgets*
that it did so.

### 1.1 Where `held` is written and read today

| Site | Action | Intended meaning |
|------|--------|------------------|
| `handle_corruption` (Isolable/Cascading runtime corruption) | `AssertReset` + `held.push` | (1) durable gate |
| `cascade_hold` (root + transitive dependents) | `AssertReset` + `held.push` per component | (1) durable gate |
| `Recovering` exhaustion, `Isolable` arm | `AssertReset` + `held.push` | (1) durable gate |
| `Recovering` exhaustion, `Cascading` arm | `cascade_hold` (→ gate) | (1) durable gate |
| `advance_to_next_unheld` / `is_held` | read: skip held ids | (2) walk skip |
| `Ready` entry | `held.clear()` | (2) per-walk reset |

Every **write** establishes a durable gate (meaning 1). Every **read** is a walk
skip (meaning 2). The only clear treats the set as per-walk. So the durable
writes are silently discarded at the first `Ready`.

### 1.2 Concrete failure trace

Chain `[C0 passive_required, C1 passive_isolable]`.

1. Boot walks the chain. `CorruptionDetected(C1)` → `handle_corruption`: C1 is
   isolable → `AssertReset(C1)`, `held = {C1}`. C1 is now physically gated.
2. `CorruptionDetected(C0)` → C0 is required → `failed = Some(C0)`, `Recovering`.
3. `Restored(C0)`, then `VerificationPassed(C0)`. The re-walk skips the held C1,
   the required chain is satisfied → transition to **`Ready`**.
4. `Ready` entry runs `held.clear()`. **The record that C1 is gated is gone**,
   but `AssertReset(C1)` was never undone — hardware still holds C1 in reset,
   yet the core now believes nothing is gated.
5. A later **chain walk** — triggered when another required-component fault
   (`CorruptionDetected(C0)`) drives `Recovering`, and the ensuing `Restored(C0)`
   re-enters `PreSupervision` — walks the chain from the top. C1 is no longer in
   `held`, so the walk treats it as an ordinary component: `ReadFirmware(C1)`,
   `VerifyFirmware(C1)`, and on a pass, `ReleaseReset(C1)` — **releasing a
   component that was deliberately isolated and never restored.**

That last step is a trust-boundary violation: a component the policy said to
isolate gets re-released because the durable gate was stored in a per-walk
field. This is the same class of bug as the hold-in-reset gap
already fixed in `handle_corruption`; that fix is only *partial* while `Ready`
still wipes the set.

### 1.3 Why this is subtle

- It only manifests **across two chain walks separated by a return to `Ready`**,
  so the existing single-walk tests all pass.
- The gate *effect* (`AssertReset`) is real and durable in hardware; only the
  *core's memory* of it is lost. The divergence between "what the core thinks is
  gated" and "what is physically gated" is exactly the dangerous part.

---

## 2. Proposed design: two sets with distinct lifetimes

Replace the single `held` with two explicit sets:

```rust
/// Durable gate: components physically held in reset by a policy decision
/// (runtime corruption of a non-required part, or recovery exhausted under
/// Isolable/Cascading). Each id here has a live `AssertReset`. Persists across
/// `Ready` — only an explicit restore/release path may remove an id.
gated: heapless::Vec<ComponentId, N>,

/// Per-walk skip: ids the current chain walk should not re-verify, for
/// reasons that do NOT imply a durable gate. Cleared on return to `Ready`.
skip_walk: heapless::Vec<ComponentId, N>,
```

### 2.1 Invariants

- **INV-gated-durable:** an id in `gated` has a live `AssertReset` and is never
  removed by reaching `Ready`. Removal happens only through an explicit release
  path (see §4, open question).
- **INV-gated-effect:** every insertion into `gated` is accompanied by exactly
  one `AssertReset(id)` (idempotent: guard on `is_gated`).
- **INV-walk-transient:** `skip_walk` is cleared on return to `Ready` and
  implies nothing about hardware state.
- **INV-walk-skip:** `advance_to_next_unheld` skips an id iff it is in `gated`
  **or** `skip_walk`.

### 2.2 Code changes

| Site | Before | After |
|------|--------|-------|
| `handle_corruption` gate arm | `held.push` (guard `!is_held`) | `gated.push` (guard `!is_gated`) |
| `cascade_hold` | `held.push` | `gated.push` |
| `Recovering` Isolable/Cascading exhaustion | `held.push` / `cascade_hold` | `gated.push` / `cascade_hold` |
| `is_held` / walk skip | `held.contains` | `gated.contains || skip_walk.contains` |
| `Ready` entry | `held.clear()` | `skip_walk.clear()` **only** — leave `gated` intact |

Rename `is_held` → `is_gated` for the durable check; introduce a separate
`should_skip_walk(id)` for the walk. The `cascade_hold` worklist that currently
iterates `held` iterates `gated` instead (cascades follow the durable set).

### 2.3 Effect on the buffer-capacity bound

No change. `gated` and `skip_walk` are each `≤ N`, but a single event still
emits at most one `AssertReset` per newly-gated component plus the entry's two
effects — the `E ≥ N + 2` floor already covers it. The split is state-only.

---

## 3. Is `skip_walk` actually needed today?

**Honest answer: not yet.** Auditing the write sites (table in §1.1), *every*
current insertion into `held` is a durable gate. Nothing today populates a
"skip this walk but do not gate" case. So the immediate bug could be fixed with
the smaller change:

> **Minimal fix:** keep a single set, rename it `gated`, and simply *stop
> clearing it at `Ready`* (remove `held.clear()`).

That closes the §1.2 trace. The two-set split is worth doing anyway if we want
the *type* to encode the distinction so a future "transient skip" reason can't
accidentally reuse the durable set (the very mistake that caused this bug). It is
a clarity/robustness investment, not a functional requirement for the fix.

Recommendation for reflection: decide between

- **(A) Minimal:** one `gated` set, never cleared at `Ready`. Smallest diff,
  fixes the bug, but leaves the door open to re-conflation later.
- **(B) Two sets:** as in §2. Slightly more state and a couple more call sites,
  but the durability distinction is structural and self-documenting.

---

## 4. Open question: how does a gated component come back?

With `gated` persisting, an isolated component stays dark for the rest of the
power session. Today there is **no ungate path** — `Restored(id)` is only handled
in `Recovering` for the `failed` (required) component; a `Restored` for a gated
isolable/cascading part in `Ready` falls through to `Outcome::Super` and is
ignored.

Options to consider (each is a separate decision):

1. **Sticky until reboot (status quo semantics).** Isolation persists until
   `PowerOnReset` builds a fresh `Rot`. Safe default; matches "recovery
   exhausted → give up on this part."
2. **Explicit ungate event.** Add e.g. `Event::ReleaseHold(id)` (operator- or
   attestation-gated) that emits `ReleaseReset(id)` and removes the id from
   `gated`. Introduces an external trust surface — must be authenticated, same
   concern as unknown-id corruption.

The split does **not** require picking one now; it just makes the durable set an
honest place to hang whichever policy we choose.

---

## 5. Test plan

Extend the existing suite (the current
`isolable_runtime_corruption_holds_across_rewalk` covers only the single-walk
case):

1. **Gate survives `Ready`:** the §1.2 trace — corrupt C1 (isolable, gated),
   recover C0 (required) to `Ready`, then trigger a *second chain walk* and
   assert `ReleaseReset(C1)` is **never** emitted and C1 is never
   re-`ReadFirmware`'d.
2. **Cascading gate survives `Ready`:** same shape with a `Cascading` root plus a
   dependent; assert the whole cascade stays gated across the return to `Ready`.
3. **Per-walk skip is cleared (if design B):** a component placed in
   `skip_walk` is re-considered on the next walk (guards against the
   inverse bug — accidentally persisting a transient skip).
4. **Idempotent gating:** a second `CorruptionDetected` for an already-gated
   component emits no second `AssertReset`.
5. **Regression:** all 29 existing tests still pass; the effect-buffer bound is
   unaffected.

---

## 6. Summary

`held` conflates a **durable hardware gate** with a **per-walk skip**
under a single clear-on-`Ready` rule, which discards the gate and can later
re-release a deliberately isolated component. The real fix is a state split:
`gated` (persistent, survives `Ready`) vs. `skip_walk` (cleared on return to
`Ready`).
The smallest correct fix is "stop clearing the durable set at `Ready`"; the
two-set form additionally makes the distinction structural so it can't be
re-conflated. Neither touches the finding-#1 buffer bound. The one genuinely open
policy question is whether/how a gated component is ever un-gated before reboot.

---

## 7. Questions for the CSA authors

Whether design (B) — the `skip_walk` set — is justified turns on a single
ambiguity in the CSA model: **does a non-gated skip disposition exist at all?**
The implementation cannot decide this on its own; it needs the architecture to
say. The questions below are ordered so the first is decisive and the rest pin
down the consequences.

### 7.1 The decisive question

> In the CSA trust-chain walk, when the RoT declines to verify/release a
> component, is that decision **always** a durable isolation (the component is
> held in reset until reboot or an explicit restore), or does CSA define a
> disposition where a component is skipped for the **current chain walk** but
> must be re-evaluated on a subsequent walk **without** the RoT having gated it
> (asserted reset)?

- **"Always durable isolation"** → the transient-skip requirement is **invalid**;
  the minimal fix (one persistent `gated` set, never cleared at `Ready`) is
  correct and `skip_walk` should not exist.
- **"A deferred / pending disposition exists"** → the requirement is **valid**;
  design (B) is warranted and `skip_walk`'s clear-boundary is whatever CSA names.

### 7.2 Supporting questions (pin down the consequences)

1. **Disposition enumeration.** Does CSA define a per-component disposition
   distinct from both *released/trusted* and *isolated/held-in-reset* — e.g.
   *not-yet-evaluated / deferred / pending re-admission*? Or are those two the
   only terminal dispositions for a component within a walk?

2. **Meaning of a degraded operational state.** Is it valid for the RoT to reach
   an operational (`Ready`) state while a component is **neither** released
   **nor** held-in-reset — i.e. pending re-consideration on the next walk? Or
   must every component resolve to exactly one of {released, gated} before the
   platform is considered operational?

3. **Isolation stickiness (the paired ungate question, §4).** Once recovery is
   exhausted under `Isolable`/`Cascading` and the component is isolated, is that
   isolation defined as **sticky-until-reboot**, or does CSA define a
   re-admission path (e.g. an operator- or attestation-gated `ReleaseHold`) that
   returns the component to evaluation?

### 7.3 Why these are framed this way

- **7.1** is the crux: a "deferred/pending" disposition *is* the transient skip.
  If the model has no such disposition, `skip_walk` would be inventing state CSA
  does not sanction.
- **7.2.2** attacks the same question from the lifecycle side: if "operational
  with a pending component" is illegal, then reaching `Ready` must resolve every
  component to released-or-gated, which again means no transient skip survives a
  return to `Ready`.
- **7.2.3** is adjacent but distinct (it is the ungate path from §4); bundling it
  avoids a second round-trip, since both 7.1 and 7.2.3 turn on *"is a component's
  exclusion permanent until X?"*

### 7.4 The trap to flag explicitly

Tell the authors the code currently **conflates** durable gating and per-walk
skip, and that guessing wrong in *either* direction is a trust-boundary bug:

- guess "transient" for something CSA meant to be durable → **re-release a
  deliberately isolated component** (the finding-#2 bug);
- guess "durable" for something CSA meant to be re-evaluated → **permanently
  strand a component** CSA intended to re-admit.

A decisive answer to 7.1 (plus 7.2.3 for the ungate policy) is enough to choose
between the minimal fix and design (B) and to set the clear-boundary and
re-admission path correctly.

