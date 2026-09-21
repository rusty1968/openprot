# Issue: orchestrator-sm commit window is global; align with CSA per-component commit and remove redundant CommitPolicy

## Summary

The orchestrator state machine (`orchestrator-sm`) tracks the
activated-but-not-committed window with a single platform-wide flag, while the
CSA defines commit (anti-rollback counter advance) as **per component**.
Separately, `fwmanager/api`'s `CommitPolicy` re-encodes an attestation
requirement that the CSA derives from the device tier (`ComponentKind`),
creating a redundant, contradictable setting. These two are related and should
be resolved together.

## Background (where things are)

- `CommitPolicy` — `services/fwmanager/api/src/config.rs` (on `origin/main`),
  enum `{ Liveness, LivenessAndAttestation }`, field `DeviceConfig.commit_policy`.
  No consumers anywhere in the tree.
- Commit window — `orchestrator-sm` (`services/orchestrator/sm/src/lib.rs`):
  - `Rot.pending_commit: bool` — single global flag.
  - `Event::CommitTimeout` — carries **no** `ComponentId`.
  - `Event::BootConfirmed(id)` → `Effect::CommitSvnFloor(id)` — already
    per-component at the floor level.
  - Update track (`State::Updating`, `Event::UpdateRequest` / `UpdateVerified`)
    is single-track (no id).
- CSA references:
  - `RoT_architecture/.../resiliency/README.md`: "per-component anti-rollback
    counter in OTP/fuse storage"; counter advances after each image "successfully
    boots and is deemed stable."
  - `RoT_architecture/.../boot_sequence/boot_sequence.md`: recovery-region
    devices "must be updated and/or recovered together"; different regions
    "independently of one another."
  - Attestation follows from the iRoT/symbiont tier: active-RoT devices attest
    over SPDM; symbiont devices cannot.

## Problem

1. **Global commit window under-models the CSA.** `pending_commit: bool` plus an
   id-less `CommitTimeout` allow only one update/commit in flight platform-wide.
   The per-component SVN floor (`CommitSvnFloor(id)`) is already correct, but the
   *window bookkeeping* is not per-component, so concurrent/independent per-device
   (or per-region) commits — which the CSA permits — cannot be represented.

2. **`CommitPolicy` is redundant with `ComponentKind`.** Whether a device *can*
   attest is fixed by its kind (`Active` = has iRoT, attests over SPDM;
   `Passive`/symbiont = cannot). `CommitPolicy` states the same attestation
   requirement a second time, in a separate crate, and admits combinations the
   kind already rules out:
   - `Passive + LivenessAndAttestation` — demands proof a symbiont device cannot
     produce (unsatisfiable).
   - `Active + Liveness` — silently discards a guarantee the device was built to
     give.
   Neither is rejected today; the two settings live in two crates that never
   check each other. This is **not** inherited from the CSA — the CSA has only the
   tier distinction and lets attestation follow from it.

## Decision gate

**Does the platform need concurrent (overlapping) per-device commit windows, or
is one update-in-flight-at-a-time acceptable?**

- **Serial acceptable** → no sm change required; the per-component floor already
  satisfies the CSA counter requirement. Document that updates serialize.
- **Concurrent required** → make the commit window per-component (below).

## Proposed change (if going per-component)

1. Remove `Rot.pending_commit: bool`; move the window into the existing
   per-component `statuses: Vec<ComponentStatus, N>` (e.g. a `commit` field:
   `None` / `Pending`).
2. `Event::CommitTimeout` → `Event::CommitTimeout(ComponentId)`; handler latches
   `Locked` only if that component's window is `Pending`, else drops it as stale
   (mirrors `Event::Timeout(id)`).
3. Thread `ComponentId` through the update track (`UpdateRequest`,
   `UpdateVerified`, `UpdateRejected`, `Effect::ActivateUpdate`) so opening a
   window targets the right component. `State::Updating` currently assumes one
   global update — decide between per-component sub-state or a commit-pending set
   held while in `Ready`.
4. Update `Event::component_id()` to list the newly id-carrying variants so the
   dispatch-boundary membership check covers them.
5. Keep fail-closed semantics: commit-or-lock still latches `Locked`, now naming
   the component that tripped it.

## `CommitPolicy` cleanup (independent of the concurrency decision)

- Derive the commit-evidence requirement from `ComponentKind`
  (`Active` ⇒ attestation, `Passive` ⇒ liveness) and remove `CommitPolicy`, **or**
- If a genuine exception exists (an `Active` device that commits on liveness
  alone), keep an explicit override layered on `ComponentKind` and validate it so
  impossible combinations (`Passive + attestation`) are rejected at build time.
- Note: `CommitPolicy` is already on `origin/main`, so this is a change to merged
  code (and it currently has no consumers).

## Acceptance criteria

- [ ] Decision recorded: serial vs concurrent commit windows.
- [ ] If concurrent: commit window is per-component; `CommitTimeout` carries a
      `ComponentId`; stale-fire drop and commit-or-lock covered by tests.
- [ ] Commit-evidence requirement derives from `ComponentKind`; no way to
      configure `Passive + attestation`.
- [ ] `CommitPolicy` removed or reduced to a validated override; no orphaned,
      unconsumed config remains.
