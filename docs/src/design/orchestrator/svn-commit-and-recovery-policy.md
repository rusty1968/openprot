# Design: Deferred SVN-Floor Commit and Recovery-Image Selection

**Status:** Partially implemented. **Proposal A's reducer surface has landed**
(`Event::BootConfirmed`, `Effect::CommitSvnFloor`, commit decoupled from
activation, boot-failure rollback kept on the update path). **Still open:** the
attacker-bounded commit-or-lock watchdog (§3), the precise "proven healthy"
definition, and all of **Proposal B** (golden refresh / recent-tier fallback).
**Scope:** orchestrator state machine (`services/orchestrator/sm`) and the
platform/provisioning policy around it.
**Related tests today:** `svn_floor_commits_on_boot_confirmed_not_on_activation`,
`update_verified_activates_update`, `update_rollback_is_not_recovery`,
`corruption_in_updating_triggers_recovery`, `corruption_during_update_discards_staged`.

## 1. Problem

Two related concerns were raised in review:

1. **The anti-rollback (SVN) floor may be advanced too early.** If the floor is
   committed at *activation* of a new image, the previous image immediately
   falls below the floor. A new image that activates but then fails to boot
   leaves no legal fallback — the device can brick rather than roll back to the
   version that was working moments ago.

2. **The golden image ages.** Golden is a factory baseline. Years into
   deployment it is a large re-update away from current and may carry
   long-since-patched vulnerabilities. Always recovering to it makes every
   recovery expensive and returns the platform to a stale baseline.

The tempting fix — "on failure, jump back to the most recent known-good version
instead of golden" — is correct for **one** of the two failure paths and wrong
for the other. This doc separates them.

## 2. Two failure paths, deliberately distinct

The machine already distinguishes these, and the distinction is load-bearing:

| Path | Trigger | Meaning | Current response |
|---|---|---|---|
| **Update/boot failure** | `UpdateRejected`, boot `Timeout` on a fresh image | *The new image is bad; the old one was fine.* | Stay on / return to the current good image (`DiscardStaged`); this is **not** recovery. |
| **Integrity corruption** | `CorruptionDetected` | *We can no longer trust whatever is running.* | Enter `Recovering`, emit `RestoreGoldenImage`. |

The key asymmetry:

- On **update/boot failure**, the previously-running image is *known good* — it
  ran until we deliberately replaced it. Rolling back to it is safe.
- On **corruption**, no mutable slot is trustworthy — the same cause (glitch,
  bit-rot, malicious write) may have hit the previous slot too. Only the
  immutable golden anchor can be trusted without a check we'd have to trust.

**Consequence:** "recover to a recent version" belongs *only* on the
update/boot-failure path. It must not be wired into the corruption trust
decision.

## 3. Proposal A — commit the SVN floor on *proven boot*, not on activation

Advance the anti-rollback floor only after the freshly-activated image has
**proven itself healthy**, not merely activated or merely "booted." This is the
standard trial-boot / commit-on-success pattern (TF-A trial firmware,
A/B commit-on-successful-boot, UEFI capsule confirmation).

Benefits:

- A new image that activates but fails to boot/verify can roll back to the
  previous image **without violating anti-rollback**, because the floor was
  never moved past it.
- It dissolves the "previous slot fails the floor" objection *for the
  boot-failure case specifically* — during the pre-commit window the previous
  slot is still ≥ floor.

### Two hard requirements

1. **The commit trigger must be "proven healthy," not "booted."** Malware boots.
   Committing on mere boot commits the floor to whatever booted, including a
   malicious image. The trigger must be a supervised health / attestation pass.

2. **The rollback window must be attacker-bounded.** While the floor sits behind
   the running version, the device is downgrade-eligible. An attacker able to
   *induce* boot failures (or suppress the confirmation) can pin the device in
   that window to force a downgrade to a vulnerable prior version. The window
   needs a watchdog / max-attempts that eventually **commits or locks** — it may
   not stay open indefinitely.

## 4. Proposal B — keep golden fresh instead of trusting a recent mutable slot

To address the aging-golden concern **without** weakening the corruption trust
model:

- **Preferred: refresh golden under stronger authority.** Periodically
  re-provision the recovery image on a rare cadence under a higher-privilege
  signing key. Golden stays the trusted anchor; the re-update delta after a
  corruption recovery shrinks. Honest cost: golden is no longer purely
  ROM-immutable — it becomes "rarely updated under strict authority." That is a
  weaker but usually acceptable trust story, and far safer than anchoring
  recovery on an ordinary mutable slot.

- **If a recent-known-good fallback is still wanted on corruption**, treat it as
  **availability-only**: attempt it *ahead of* golden, gated on signature **and**
  SVN floor, with golden as the guaranteed backstop and a full re-verification
  afterward. It may only *shorten* the road to a provably-good state — never
  substitute for reaching one.

## 5. Where this lands in the reducer

Most of this is **provisioning/commit policy that lives around the reducer**, not
inside it. The reducer emits `ActivateUpdate`, `CommitSvnFloor`, and
`RestoreGoldenImage`; "when to burn the SVN floor" is kept a separate concern
from activation.

Reducer-visible changes — **implemented** (names are now final, not tentative):

- **`Event::BootConfirmed(ComponentId)`** — raised by the driver *only* after a
  supervised health / attestation pass, not on mere readiness. Handled in
  `State::Ready` in place (confirming a running image is not a state change).
- **`Effect::CommitSvnFloor(ComponentId)`** — emitted only on `BootConfirmed`,
  kept **decoupled** from `ActivateUpdate` so activation and commit are separate
  steps. Pinned by `svn_floor_commits_on_boot_confirmed_not_on_activation`.
- **Boot-failure rollback stays on the update path** (`DiscardStaged`,
  "not recovery"), leaving `RestoreGoldenImage` as the corruption backstop
  untouched.

> **Still open (not in the reducer):** the §3 watchdog that bounds the
> activated-but-not-committed window and forces *commit-or-lock*. Today
> `State::Ready` waits for `BootConfirmed` with no deadline, so the downgrade
> window described in §3.2 is currently unbounded — the bound must still be added
> (driver-side or as a reducer timeout arm).

Sketch of the decoupled flow:

```text
UpdateRequest → Updating
  ├─ UpdateVerified → ActivateUpdate → Ready            (activated, NOT committed)
  │     └─ BootConfirmed → CommitSvnFloor               (floor advances only now)
  │     └─ boot Timeout / health fail → roll back to previous  (floor never moved → legal)
  └─ UpdateRejected → DiscardStaged → Ready             (never activated)

CorruptionDetected → Recovering → RestoreGoldenImage    (unchanged; golden anchor)
```

The watchdog from §3 is the mechanism that closes the
`Ready`-activated-but-not-committed window: after a bounded number of
attempts/time without `BootConfirmed`, either commit (if health finally passes)
or lock/roll-back per policy.

## 6. Threat and trade summary

| Change | Buys | Costs / risks | Mitigation |
|---|---|---|---|
| Deferred SVN commit (A) | Safe rollback on boot failure; no brick | Bounded downgrade window | Health-gated commit + watchdog that commits-or-locks |
| Commit on "booted" (rejected) | — | Commits floor to malware | Require attestation/health, not boot |
| Refresh golden (B) | Golden stays recent; cheaper recovery | Golden no longer pure-immutable | Rare cadence, higher-privilege key |
| Recent-tier on corruption | Faster time-to-online | Reintroduces untrusted-slot problem | Availability-only, signature+floor gated, golden backstop, full re-verify |

## 7. Recommendation

- **Adopt Proposal A**: deferred, health-gated, watchdog-bounded SVN commit,
  decoupled from activation. It is the correct fix and makes boot-failure
  rollback legitimate without weakening anti-rollback. *(Landed in the reducer
  except for the watchdog bound — see §5.)*
- **Address stale golden via Proposal B's refresh option**, not by trusting a
  recent mutable slot on the corruption path. *(Not yet implemented.)*
- **Keep the two failure paths separate**: recent-version rollback on
  update/boot failure; golden on corruption. *(Enforced today: `DiscardStaged`
  on the update path, `RestoreGoldenImage` on corruption.)*

## 8. Open questions

- What precisely constitutes "proven healthy" for `BootConfirmed` — self-test,
  attestation quote, N clean boots, workload signal?
- What are the watchdog bounds (attempts / time) before commit-or-lock?
- Refresh-golden cadence and signing-authority model — who is allowed to move
  the anchor, and how is that authority itself rooted?
- Does the SVN floor advance per-component or platform-wide, and how does that
  interact with the per-device recovery budget already in the machine?
