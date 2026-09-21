# Requests to fwmanager (from the Boot Orchestrator side)

Context: the orchestrator (`openprot/services/orchestrator/sm`) is a pure
`no_std` reducer that emits `Effect` values and consumes `Event`s; it never
touches hardware. fwmanager provides the actuation capabilities the shell calls
to carry out those effects. Wiring the two together surfaced a few contract
gaps. Each request below is self-contained — feel free to take them individually.

Guiding principle we'd like the capability traits to follow:

> **Any answer (a verdict, a readiness result, an error category) must be a
> value in the `Ok` path. The `Err` path is reserved exclusively for "the
> platform could not carry out the command."**

This one rule prevents a whole class of bugs where a legitimate "no" answer
(e.g. firmware failed verification) gets routed through the error channel and is
mistaken by the reducer for an actuation failure — which fail-closes the whole
platform to lockdown.

---

## Request 1 — Verification capability should return a verdict value, not an error

**What:** Define the firmware-verification step as a real capability trait whose
success path carries the pass/fail verdict as a value.

**Why:** Right now verification appears only as `verify_active_slot()?` in a
`BootControl` doc example. If an adapter implements it so that a failed
verification returns `Err(..)`, the reducer treats it as an actuation failure and
latches the platform to `Locked`, destroying the recover-first path (a failed
verification is supposed to enter recovery, not lockdown). Making the verdict a
value in the `Ok` path prevents this by construction.

**Suggested shape:**

```rust
pub enum VerifyOutcome { Passed, Failed }

pub trait FirmwareVerify {
    type Error: core::error::Error;
    /// `Ok(Failed)` means "verified and it did not pass" — NOT an error.
    /// `Err(..)` means "could not run the check at all."
    fn verify(&mut self, slot: Slot) -> Result<VerifyOutcome, Self::Error>;
}
```

---

## Request 2 — When we wait for a device to boot, tell us "timed out" separately from "failed"

Here's where fwmanager is at today, in plain terms. fwmanager already has most
of the pieces for this. Each device in the fwmanager config carries a
`boot_timeout` — a "how long are we willing to wait for this thing to come up"
value. And the fwmanager docs show an `await_boot(window)` example where
something waits for a device to report in. So the *idea* of "kick the device,
then wait a set amount of time for it to boot" is already in fwmanager. What's
missing is that it's only an
example in the docs, not an actual trait anyone can call, and the answer it hands
back doesn't clearly separate "it came up," "it failed to come up," and "it never
answered before the clock ran out." This request is about turning that idea into
a real callable thing with those three answers kept apart.

**What:** Give us a proper way to wait for a device to finish booting that
tells us one of three things: it booted, it failed, or it ran out of time.

**Why:** There are already hints of this — `DeviceConfig::boot_timeout` and the
`await_boot(window)` example — but there's no actual trait, and we need those
three answers kept apart. "It failed to boot" and "it never answered in time"
are different situations and we handle them differently. A device can pass
verification and then just go quiet; if that "went quiet" case is lumped in with
an outright failure, we can't tell them apart. We're adding a `Timeout` event on
our side specifically so a device that times out gets pushed into recovery.

**Suggested shape:**

```rust
pub enum BootProgress { Booted, Failed, Timeout }

pub trait BootMonitor {
    type Error: core::error::Error;
    fn await_boot(&mut self, window: core::time::Duration)
        -> Result<BootProgress, Self::Error>;
}
```

The key ask: please keep `Timeout` as its own answer, not folded into `Failed`.

---

## Request 3 — Expose error category generically, with a config-vs-runtime split

**What:** Resolve the open question in
`error-downcasting-and-category-recovery.md` so a generic consumer can recover
the error *category* without `downcast_ref` and without naming a concrete
adapter type. At minimum, give us a coarse "is this a configuration fault?"
signal.

**Why:** The orchestrator deliberately does **not** branch on fine-grained error
category — every genuine runtime actuation failure is treated the same
(fail-closed → lockdown). So relative to the fwmanager doc, this is its
**Option 3, but with a two-value vocabulary**: we want the category exposed as a
first-class trait capability (no downcast), we just don't need the full
`ErrorKind` vocabulary through the trait — only one coarse distinction:

- `InvalidResetId` (and similar) is a **board-configuration fault** — it should
  fail at bring-up, never become a runtime platform lockdown.
- `HardwareFailure` / `Timeout` / etc. are **runtime faults** — fail-closed.

Two things worth calling out, both of which keep the fwmanager leaf HAL-free and
resolve the doc's own friction:

- **Exposing it as a method kills the whole downcast problem.** The `'static`
  bound, naming a concrete `BootError<..>`, the silent "always `None`" — those
  are all artifacts of recovering the category by `downcast_ref`. A first-class
  `fault_class()` needs no downcast, so none of that applies. This request is
  really "let us stop downcasting," not "help us downcast better."
- **The `ErrorKind → FaultClass` mapping belongs in the adapter, not the leaf.**
  `FaultClass` is a *classification of* `ErrorKind`, so someone has to write the
  map (`InvalidResetId → Configuration`, `Timeout | HardwareFailure → Runtime`).
  If that map lives in `fwmanager-api`, that re-imports the HAL vocabulary the
  leaf is trying to avoid. So `fwmanager-api` owns only the two-value
  `FaultClass` type and the `ClassifiedError` trait; the mapping lives in
  `hal-adapters`, which already knows `HalError`.

One more framing note: a config fault should ideally never reach runtime at all —
`InvalidResetId` is a bad `reset_line` in the device table, which is exactly the
kind of thing a bring-up check should catch (today `validate()` checks name and
timeout but not that `reset_line` is real). So treat the runtime
`FaultClass::Configuration` branch as a **backstop**, not a recovery path we
expect to exercise.

**Suggested shape (illustrative):**

```rust
// Owned by fwmanager-api, no HAL coupling:
pub enum FaultClass { Configuration, Runtime }

pub trait ClassifiedError: core::error::Error {
    fn fault_class(&self) -> FaultClass;
}
```

---

## Request 4 — Formalize the trial-boot / commit / rollback capabilities

**What:** Turn the post-activation trial-boot lifecycle (`set_trial`, `commit`,
`rollback`, honoring `CommitPolicy`) into real capability traits, not doc prose.

**Why:** The fwmanager `CommitPolicy { Liveness, LivenessAndAttestation }` and the
trial-boot example describe a richer update flow than the reducer currently
models (the reducer today only does pre-activation authenticate/stage →
activate/discard). We're planning to extend the reducer to represent
post-activation trial + rollback, including the `LivenessAndAttestation`
re-attestation gate — but we can't wire or test it end to end until these are
callable contracts. No behavior change requested, just formalization of the
capability surface fwmanager already describes.

---

## Request 5 — One source of truth for the device list, with a build-time check

**What:** Have the device list and the orchestrator's list come from a single
shared source, or — if they stay as two lists — add a build-time check that
proves the two are exact mirrors: same number of devices, same order, same ids.
A shared id type is the cleanest way to make that check meaningful.

**Why:** There are two lists of devices today, and nothing makes sure they agree.
fwmanager just added one: the `MANAGED_DEVICES` list in
`target/mock/devices.rs`, built on the new `config.rs` schema. Its comment says
the order the devices are written in *is* the boot order — first one boots first,
and so on. The orchestrator sm has
its own list too: a `Chain` of `ComponentId`s, with its own order that also means
boot order. So both sides are independently deciding "who is device #1, who is
device #2, and what order they boot in."

The trap is that these two lists can quietly disagree. A device is identified
today only by where it sits in the list plus its `name` and `reset_line` — there
is no shared id. The fwmanager `validate()` checks that names aren't empty and
timeouts aren't zero, but it never checks that the list matches the
orchestrator's list. So if the two fall out of step — a different count, a
different order, a different device — nothing complains. It just boots or resets
the wrong device.

Since fwmanager owns the device list (the `config.rs` schema and the board
`devices.rs` tables), the shared id and the check belong on the fwmanager side —
shipping from `fwmanager-api` (or the board source it defines), with the
orchestrator's list reading those ids. The ideal is a single device source both
sides read from, so there's only ever one list; failing that, a build-time check
(a `const` assert) that fails the build when the two lists don't line up.

---

## Request 6 — Extend capability coverage to the rest of the effect surface

**What:** `BootControl` covers reset hold/release only. Please plan capability
traits (each following the verdict-vs-error rule above) for the remaining
operations the orchestrator emits:

- firmware read + verify (Request 1),
- golden-image restore / recovery,
- update stage / activate / discard (Request 4),
- attestation signing,
- lockdown latch,
- degraded-mode reporting (`ReportIsolated`, `ReportRecoveryFailed`).

**Why:** Otherwise the shell has to invent an ad-hoc contract per effect, and the
verdict-vs-error hazard from Request 1 recurs for each one. Coverage doesn't have
to land all at once — reset + verify + boot-monitor (Requests 1–2) unblock the
core boot path; the rest can follow.

---

### Priority

- **Blocking for a correct first wiring:** Requests 1, 2, 3, 5.
- **Next:** Request 4 (trial-boot/commit) and the remainder of Request 6.
