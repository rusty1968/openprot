# Orchestrator ↔ fwmanager Reconciliation — Design Spec

## Problem statement

The boot-supervision design is split across two components that were developed
in separate trees and have never been wired together:

- **Orchestrator** (`openprot/services/orchestrator/sm`) — a pure `no_std`
  reducer. It *decides* what to do and in what order, emitting `Effect` values
  (`AssertReset`, `ReleaseReset`, `VerifyFirmware`, `RestoreGoldenImage`,
  `ActivateUpdate`, …). It reads the world only through injected `Event`s and
  touches no hardware.
- **fwmanager** (`9eleems/services/fwmanager`) — the actuation layer. It exposes
  device-facing capability traits (`BootControl::hold_in_reset`/`release`),
  HAL-backed adapters (`HalBootControl`), per-board device config
  (`DeviceConfig`, `CommitPolicy`, `boot_timeout`), and a rich error vocabulary
  (`ErrorKind`).

Conceptually the reducer's effects map onto fwmanager's capabilities, but the
two contracts do **not** line up cleanly. The shell that eventually binds them
must bridge five impedance mismatches, and several of them fail *silently and
dangerously* if bridged naively. Because the two crates are in different
workspaces with no compile-time edge, nothing today forces the reconciliation or
catches a wrong mapping. This spec defines the reducer-side changes and the
adapter contract needed to make the seam correct, explicit, and testable.

### The mismatches

1. **Verification verdict vs actuation error are fused onto one `Result`.**
   fwmanager capabilities return `Result<(), Error>` synchronously. The reducer
   is Kind-A: `Platform::execute(VerifyFirmware(id)) -> Result<(), EffectError>`
   reports only whether the verify *dispatched*; the pass/fail **verdict must
   return later as a `VerificationPassed`/`VerificationFailed` event**. If an
   adapter maps "firmware failed verification" onto `Err(EffectError)`, the
   reducer latches `Locked` instead of entering `Recovering`, destroying the
   recover-first guarantee. (Highest-risk mismatch.)

2. **Boot-progress `Timeout` has no reducer event.** fwmanager carries
   `boot_timeout` and models `await_boot(window) -> Booted | Failed | Timeout`.
   The reducer's `AwaitingReady` waits on `ComponentReady` with no timeout arm
   and no `Event::Timeout`. A hung device leaves the reducer stuck forever unless
   the shell synthesizes some other event by unwritten convention.

3. **Rich `ErrorKind` is discarded at the reducer boundary.** fwmanager
   preserves error category (`Timeout`, `InvalidResetId`, `HardwareFailure`, …);
   the reducer's `EffectError` is a unit struct and every failure is blanket
   fail-closed → `Locked`. A config-level fault (`InvalidResetId`) becomes a
   runtime platform lockdown, indistinguishable from a hardware attack.

4. **Trial-boot / commit lifecycle is richer than the reducer's Update model.**
   fwmanager has `CommitPolicy { Liveness, LivenessAndAttestation }` and a
   *post-activation* trial flow (set-trial → release → await-boot → commit or
   roll back to the previous slot). The reducer's `Updating` is *pre-activation*
   only (`AuthenticateUpdate`/`StageUpdate` → `ActivateUpdate`/`DiscardStaged`),
   with no trial-observation state, no watchdog window, no automatic
   post-activation rollback, and no re-attestation gate.

5. **Two independent id spaces.** The reducer's `Chain` of `ComponentId(u8)` and
   fwmanager's `DeviceConfig` table (`reset_line`, registry index) are not
   guaranteed to agree in count, order, or ids. Drift is caught only by board
   discipline, not the compiler.

### What already lines up

`BootControl` forbids `reset_pulse` and refuses to query line state — sequencing
is owned by the orchestrator, hold/release are distinct steps. That matches the
reducer's distinct `AssertReset`/`ReleaseReset` and release-is-gated invariant
exactly. The only standing assumption is that the reset line *latches* (a hold
persists across the reducer's two settles / an IPC round-trip). No change needed;
stated here so it is not silently relied upon.

## Goals

- Make the effect/verdict/error channels at the seam explicit and impossible to
  mis-map without a compile error or a failing test.
- Give the reducer a first-class way to consume a boot-progress timeout.
- Preserve the reducer's deliberate blanket fail-closed policy for genuine
  runtime actuation faults, while keeping configuration faults out of the
  runtime-lockdown path.
- Provide a path (phased) for the reducer to model the post-activation
  trial-boot/commit lifecycle fwmanager already supports.
- Guarantee the reducer `Chain` and the fwmanager device table cannot drift
  unnoticed.

## Non-goals

- Merging the two workspaces or introducing a direct crate dependency between
  them. The seam stays a contract, not a linkage.
- Changing fwmanager's HAL-free leaf design or its `ErrorKind` vocabulary.
- Redefining the reducer as a synchronous (Kind-B) machine. The Kind-A
  effect/event split is deliberate and stays.

## Design

### D1 — Adapter channel contract (fixes #1, part of #3)

No reducer code change; this is a **normative adapter specification** that the
shell binding must follow, backed by tests on the shell side.

- **Actuation outcome → `Result`.** A capability call that fails to *perform the
  action* (reset line stuck, bus error, hardware fault) returns
  `Err(EffectError)` from `Platform::execute`, which the reducer turns into
  `EffectFailed` → `Locked`. This is the *only* thing that flows through the
  effect-error channel.
- **Verification/readiness verdict → `Event`.** The *result* of a
  `VerifyFirmware`/`ReadFirmware` operation (pass/fail) and a device's readiness
  are never expressed as `execute` errors. The adapter injects
  `VerificationPassed(id)` / `VerificationFailed(id)` / `ComponentReady(id)` as
  events. A failed verification is a normal event, not an actuation error.
- **Rule of thumb:** `execute` errors mean "the platform could not carry out the
  command"; they never mean "the command was carried out and the answer was no."

State this contract in the `Platform` trait docs in `lib.rs` (the trait already
warns about honest/complete feedback; extend it with the verdict-vs-error rule).

### D2 — `Event::Timeout(ComponentId)` (fixes #2)

Add a first-class timeout event, mirroring the make-it-explicit philosophy used
for `ReportIsolated`:

```rust
// model.rs, Event
/// The shell's boot-progress watchdog fired: `id` did not report readiness
/// within its configured `boot_timeout`. Treated as a verification failure.
Timeout(ComponentId),
```

Handling in `lib.rs`, `AwaitingReady`:

- `Timeout(id)` where `id` matches the awaited component → transition to
  `Recovering(id)` (recover-first: a missed boot checkpoint is a failure).
- `Timeout(id)` for any other id → `Outcome::Handled` (stale/spurious; ignore),
  matching the existing `ComponentReady` stale-id treatment.

The shell owns the timer (fed by `DeviceConfig::boot_timeout`) and injects
`Timeout(id)` when the window elapses. Rationale for an explicit event over
reusing `VerificationFailed`: it is observable in the effect/event trace and
testable in isolation, and it distinguishes "verified fine but never came up"
from "failed verification" for telemetry.

### D3 — Configuration-fault carve-out (fixes #3)

Keep `EffectError` a unit struct and the blanket runtime lockdown policy — that
is deliberate and correct for genuine faults. The carve-out is for
*configuration* categories that indicate a board-bring-up bug, not a runtime
compromise:

- The adapter treats `ErrorKind::InvalidResetId` (and equivalent config-time
  categories) as a **hard init-time fault**: it fails at board bring-up /
  registry construction, before the reducer is ever stepped — not as a runtime
  `EffectError`.
- All remaining categories (`Timeout`, `HardwareFailure`, …) collapse to
  `EffectError` → `Locked` as today.

This is an adapter contract plus a startup-validation step (see D5); no reducer
change. Document that the reducer intentionally does not branch on `ErrorKind`
and why (answers the open question in fwmanager's
`error-downcasting-and-category-recovery.md` with "Option 4, runtime; validated
at init").

### D4 — Trial-boot / commit lifecycle (fixes #4, phased)

This is a genuine feature gap and the largest change; propose it as **Phase 2**,
separable from D1–D3, D5.

Extend the update model so the reducer represents post-activation trial and
rollback:

- New events: `UpdateActivated` (the new slot was made the trial boot target and
  released), `TrialBootObserved` (device came up and, per policy, re-attested),
  `TrialBootFailed` (watchdog/attestation failed in the trial window).
- New effects: `CommitSlot` (make trial permanent) and `RollbackSlot` (restore
  the previous slot).
- New state or `Updating` sub-payload to hold the trial window / awaited device,
  parameterized by `CommitPolicy` (`Liveness` vs `LivenessAndAttestation`) so the
  `LivenessAndAttestation` path additionally requires a re-attestation event
  before `CommitSlot`.

Deferred until D1–D3 land; specified here so the update model is not "finished"
prematurely against a partial view of fwmanager's commit semantics.

### D5 — Chain / device-table bijection check (fixes #5)

The board is the single owner of both the reducer `Chain` and the fwmanager
device table. Add a startup reconciliation the board runs before stepping the
reducer:

- Assert a bijection between `Chain` `ComponentId`s and `DeviceConfig` entries
  (same count; every `ComponentId` resolves to exactly one device; every device
  is named by exactly one chain entry).
- Fail bring-up (not runtime lockdown) on any mismatch — same class as the D3
  config-fault carve-out.

Where feasible, generate both tables from one board source so drift is a build
error rather than a startup assertion.

## Alternatives considered

- **Map `Timeout` onto existing `VerificationFailed` (no new event).** Smaller,
  but loses trace observability and conflates "never booted" with "failed
  verification." Rejected in favor of D2's explicit event.
- **Widen `EffectError` to carry `ErrorKind`.** Lets the reducer branch on
  category. Rejected: it re-opens the deliberate blanket fail-closed decision,
  couples the reducer to the HAL vocabulary, and the only category that actually
  wants different handling (config faults) is better caught at init (D3/D5).
- **Make the reducer synchronous (consume verdicts as `execute` return
  values).** Would remove #1 by construction but discards the Kind-A design,
  its testability, and its IPC-friendliness. Rejected (non-goal).

## Validation

- D2: unit tests in `orchestrator_sm_test` — `timeout_awaited_enters_recovering`,
  `timeout_stale_id_ignored`, and a full `AwaitingReady` timeout→recover→rewalk
  path. `rustfmt` + `bazelisk test //services/orchestrator/sm:orchestrator_sm_test`.
- D1/D3/D5: adapter/shell-side tests in the binding crate (out of the reducer
  tree): a verification-failure event does not lock down; an actuation error
  does; an `InvalidResetId` fails bring-up; a chain/table mismatch fails
  bring-up.
- D4 (Phase 2): its own test set for the trial/commit/rollback transitions under
  both `CommitPolicy` values.

## Rollout

- **Phase 1** (correctness at the seam): D1, D2, D3, D5. D2 is the only reducer
  code change (`Event::Timeout` + `AwaitingReady` arm + tests); the rest are
  adapter contract + startup validation living in the shell/binding crate.
- **Phase 2** (feature parity): D4 trial-boot/commit.

Terse commits, code only (no markdown staged per convention), e.g.
`orchestrator-sm: add Timeout event and AwaitingReady recovery arm`.

## Open questions

- Does `Timeout` also need to fire in `PreSupervision` (a `Passive`/symbiont
  device that never returns a verify verdict), or is the verify-verdict watchdog
  the shell's responsibility there? Leaning: the shell injects
  `VerificationFailed` on verify-side timeout, and `Event::Timeout` is reserved
  for the `AwaitingReady`/readiness gate only. Confirm.
- For D4, should `RollbackSlot` re-enter the normal walk (re-verify the restored
  previous slot) or trust the previously-good slot implicitly? Leaning: re-verify
  for consistency with recover-first.
- For D5, is a build-time generated table feasible given the two separate
  workspaces, or must the bijection be a runtime startup assertion?
