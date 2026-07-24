# State Machine Transitions

This document narrates **every transition** in the orchestrator state machine,
one small section at a time. Where the [State Machine](./orchestrator-machine.md)
reference presents the transitions as tables and the
[Walkthrough](./orchestrator-sm-walkthru.md) tells the story end to end, this
document is the per-edge companion: each transition gets its own short prose
entry describing what triggers it, what guard it depends on, what effects it
emits, and where it lands.

Transitions are grouped by their **source state**. Within each state, the
"anything else" fall-through is described last. Throughout, *the platform* is the
surrounding runtime that executes the core's effects and delivers events back to
it; the core itself never touches hardware.

---

## From `PowerOnReset`

The machine's initial state. It waits for the platform's first event, which is
always `PowerGood`, carrying the result of the eRoT's power-on self-check.

### `PowerGood(Provisioned)` → `VerifyingPlatform`

The eRoT is provisioned and passed its own self-verification, so it is entitled
to vouch for the rest of the platform. The machine leaves the gate and begins
walking the trust chain. No effects are emitted by the transition itself; the
`VerifyingPlatform` entry action starts the first verification.

### `PowerGood(Unprovisioned)` → `Locked`

The eRoT has no provisioning, so there is no reference to verify components
against. Rather than proceed blindly, the machine goes straight to `Locked` and
latches lockdown. This is a permanent dead end for this power cycle.

### `PowerGood(SelfVerificationFailed)` → `Locked`

The eRoT's own integrity check failed. Because the entire chain of trust hangs
off the anchor, an untrustworthy anchor must not release anything downstream. The
machine locks down immediately.

### anything else → discarded

`PowerOnReset` sits outside the `Operational` superstate, so any event other than
`PowerGood` falls through to the top level and is discarded. The machine does not
answer attestation or act on corruption before it has even begun verifying.

---

## From `VerifyingPlatform`

Walks the trust chain component by component. The entry action points the cursor
at the first component not already in `held` and asks the platform to read and
verify its firmware. The transitions below react to the platform's verdicts.

### `VerificationPassed` [more components, current is `Passive`] → `VerifyingPlatform` (self)

The current component is a symbiont device with no root of trust of its own, and
its single eRoT-side check just passed. The machine releases it (`ReleaseReset`),
asks the platform to read and verify the next component
(`ReadFirmware` · `VerifyFirmware`), and advances the cursor — all while staying
in `VerifyingPlatform`. This is the self-loop that rolls the walk forward through
symbiont devices. It uses `Outcome::Handled` rather than a real self-transition so
the entry action does not re-run and reset the cursor.

### `VerificationPassed` [more components, current is `Active`] → `AwaitingReady`

The current component is a SoC with its own integrated iRoT, and its eRoT-side
check passed. The machine releases it, speculatively starts the *next*
component's eRoT check, records the component in `awaiting`, and moves to
`AwaitingReady` to wait for the component's own root of trust to report in. The
release plus the next read/verify are emitted here; the wait happens in the
destination state.

### `VerificationPassed` [chain done] → `Ready`

The component that just passed was the last one in the chain. The machine
releases it and transitions to `Ready`: every component has been verified and
released, so the platform is up and the trust chain is established.

### `VerificationFailed` (any policy) → `Recovering`

A component's firmware failed its eRoT-side check. Regardless of the component's
recovery-failure policy, the machine records it in `failed` and enters
`Recovering` to attempt restoration. The component is **not** skipped here — it is
held in reset (never released, so it never runs unverified code) and given a
recovery attempt first. The `Isolable`/`Cascading`/`PlatformHalt` decision is
deferred until recovery has actually failed.

### held components → skipped (no transition)

Not an event-driven edge: while advancing the cursor, any component already in
`held` (one whose recovery was previously exhausted) is skipped without
verification. No `ReadFirmware`/`VerifyFirmware` is emitted for it and it stays in
reset; the cursor simply moves past it to the next candidate.

### anything else → discarded

`VerifyingPlatform` also sits outside `Operational`, so unrelated events fall
through to the top level and are discarded. Attestation challenges and corruption
reports are not serviced during the initial chain walk.

---

## From `AwaitingReady`

Reached when an `Active` component clears its eRoT check. The machine waits here
for the component's iRoT to signal readiness, while the next component's
speculative eRoT check may still be in flight. Two things must resolve — the
awaited `ComponentReady` and the pending `VerificationPassed` — and they can
arrive in either order.

### `ComponentReady` [id ≠ `awaiting`] → `AwaitingReady` (self)

The readiness signal is for some component other than the one being awaited — a
stale or spurious report. The machine ignores it (`Outcome::Handled`) and stays
put, so a late or duplicated signal can never push the walk forward incorrectly.

### `ComponentReady` [id = `awaiting`, cursor in bounds] → `AwaitingReady` (self)

The awaited component's iRoT has come up. The machine clears `awaiting` to record
that the readiness gate is satisfied, but stays in `AwaitingReady` because the
next component's eRoT verdict is still outstanding.

### `ComponentReady` [id = `awaiting`, cursor past end] → `Ready`

The awaited component's iRoT came up and there is nothing left to verify — the
cursor has already advanced past the end of the chain (the last component was
skipped or resolved). With both gates now clear, the machine transitions to
`Ready`.

### `VerificationPassed` [more components] → `AwaitingReady` (self)

The speculative eRoT check for the next component passed. The machine releases
that component, starts reading and verifying the one after it, and advances the
cursor — mirroring the `VerifyingPlatform` walk — while remaining in
`AwaitingReady` because it may still be waiting on an iRoT readiness signal.

### `VerificationPassed` [chain done] → `Ready`

The speculative check resolved and it was the last component in the chain. The
machine releases it and transitions to `Ready`; the walk is complete.

### `Timeout` [id = `awaiting`] → `Recovering`

The platform's boot-progress watchdog fired: the awaited component did not report
readiness within its window. The machine treats this as a verification failure —
records the component in `failed`, clears `awaiting`, and enters `Recovering`.
This realizes the CSA boot-progress checkpointing mechanism, where a missed
checkpoint is treated as a failure that initiates recovery.

### `Timeout` [id ≠ `awaiting`] → `AwaitingReady` (self)

A watchdog fired for a component the machine is not currently awaiting — stale.
The machine ignores it and stays put.

### `VerificationFailed` (any policy) → `Recovering`

The speculative eRoT check for the next component failed. As during the initial
walk, recovery is attempted first: the machine records the component in `failed`,
clears `awaiting` (abandoning the in-flight readiness wait), and enters
`Recovering`.

### anything else → `Operational`

`AwaitingReady` is one of the four operational states, so unrelated events fall
through to the `Operational` superstate — which answers attestation challenges
and acts on corruption reports even while the platform is still coming up.

---

## From `Ready`

Steady state: the whole chain is verified and released. The entry action clears
the recovery bookkeeping (`retry_count`, `held`, `failed`), since arriving here
means the platform booted clean.

### `UpdateRequest` → `Updating`

The platform has requested a firmware update. The machine transitions to
`Updating`, whose entry action begins authenticating and staging the new image.

### anything else → `Operational`

Everything else `Ready` does — answering attestation, handling corruption — is
inherited from the `Operational` superstate via fall-through.

---

## From `Updating`

An update is in progress. The entry action emits `AuthenticateUpdate` and
`StageUpdate`; the machine then waits for the platform's verdict.

### `UpdateVerified` → `Ready`

The staged image authenticated successfully. The machine emits `ActivateUpdate`
to switch to the new image and returns to `Ready`.

### `UpdateRejected` → `Ready`

The staged image failed authentication. The machine emits `DiscardStaged` to
throw it away and returns to `Ready`, continuing to run the image it already had.
A rejected update is deliberately **not** treated as corruption — nothing trusted
was damaged, so there is no reason to enter recovery.

### anything else → `Operational`

Attestation and corruption handling during an update come from the `Operational`
superstate.

---

## From `Recovering`

Attempting to restore a failed or corrupted component. The entry action emits
`RestoreGoldenImage(failed)`, which the platform applies to the failed
component's entire recovery region (all components sharing its `RegionId`). The
transitions below fire on `Restored` and branch on how many attempts remain and,
once exhausted, on the component's recovery-failure policy.

### `Restored` [`retry_count + 1 < max_retry`] → `VerifyingPlatform`

The restore completed and attempts remain. The machine re-walks the chain from
the top to re-verify — the restored image may now pass. Re-verifying end to end
(rather than resuming at the failed component) re-establishes trust across the
whole platform, which is the conservative reading of the "no component executes
unverified firmware" principle.

### `Restored` [cap reached, `failed` is `Isolable`] → `VerifyingPlatform`

Restore attempts are exhausted and the recovery image still fails, and the
component's policy is `Isolable`. The machine gives up on this one component
only: it emits `AssertReset(failed)` to keep it held, adds it to `held`, clears
`failed`, and re-walks to continue booting the rest of the platform. The re-walk
skips the now-`held` component.

### `Restored` [cap reached, `failed` is `Cascading`] → `VerifyingPlatform`

As with `Isolable`, but the failed component's dependents go down with it. The
machine emits `AssertReset` for the component and each component whose
`depends_on` names it, adds them all to `held`, clears `failed`, and re-walks to
continue booting the remainder.

### `Restored` [cap reached, `failed` is `PlatformHalt`] → `Recovering` (self, then `Locked`)

Restore attempts are exhausted and the component's policy is `PlatformHalt`,
meaning the platform cannot safely continue without it. Rather than jump straight
to lockdown, the machine emits `Effect::Emit(RecoveryFailed)` — a follow-up event
— and returns `Handled`. The orchestrator re-dispatches `RecoveryFailed`
immediately (see the next transition), so lockdown appears in the effect trace as
a discrete event rather than a hidden jump.

### `RecoveryFailed` → `Locked`

The follow-up event emitted above (or any `RecoveryFailed`) drives the machine to
`Locked`. Routing lockdown through this single event means `Locked` is only ever
entered one way, no matter where the give-up decision originated.

### anything else → `Operational`

`Recovering` is an operational state, so attestation and corruption events fall
through to the `Operational` superstate and are handled even mid-recovery.

---

## From `Locked`

Terminal. The entry action emits `LatchLockdown`, instructing the platform to
hold every component in reset permanently.

### any event → discarded (self)

`Locked` handles nothing and has no superstate, so every event falls through to
the top level and is discarded. The machine remains in `Locked` for the rest of
the power cycle.

---

## From the `Operational` superstate

`Ready`, `Updating`, `Recovering`, and `AwaitingReady` share this parent. When
one of them returns `Outcome::Super`, these handlers run. Centralizing them here
guarantees the two platform-wide behaviors apply identically in all four states.

### `AttestationChallenge` → `Operational` (no transition)

The machine emits `SignAttestation` to answer the challenge and stays exactly
where it is. Answering an attestation challenge never changes state, so it is safe
to service from any operational state — including mid-boot (`AwaitingReady`) and
mid-recovery (`Recovering`).

### `CorruptionDetected` [component required] → `Recovering`

A trusted-critical component was reported corrupt at runtime. The machine records
it in `failed` and drops into `Recovering`, re-entering the same two-stage
recovery flow used at boot.

### `CorruptionDetected` [component not required] → `Operational` (no transition)

The corrupt component is not required for the platform to run. Rather than tear
down the platform, the machine emits `AssertReset` to gate the component (put it
back in reset) and stays in its current state. The component is no longer trusted,
but the platform keeps operating.

### anything else → discarded

Any event the superstate does not recognize falls through to the top level and is
discarded.
