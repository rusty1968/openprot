# CommitPolicy vs ComponentKind: the overlap, in plain language

Two different crates each carry a setting about "does attestation matter for
this device," and the two settings talk about the same underlying fact from
two angles. This note explains, without jargon, why that overlap is a problem.

## The two settings

**`ComponentKind`** lives in the orchestrator state machine (`orchestrator-sm`).
It labels every managed device as one of:

- **Active** — the device has its own little root of trust inside it (an iRoT,
  e.g. Caliptra). It can check *itself* and prove who it is. Because it has that
  ability, the orchestrator waits for the device to say "I verified myself and
  I'm ready" before moving on.
- **Passive** — the device has no root of trust inside it. It cannot check
  itself and cannot attest. The only thing it can tell us after we release it
  is "I booted." That's all we get.

So `ComponentKind` is really answering one question: **"Can this device attest
to itself, yes or no?"** Active = yes, Passive = no.

**`CommitPolicy`** lives in the firmware-manager device table
(`fwmanager/api`). After we push a new firmware image to a device, it labels how
sure we must be before we permanently accept that image:

- **Liveness** — we accept it as soon as the device comes up.
- **LivenessAndAttestation** — coming up is not enough; the device must also
  *attest* (cryptographically prove the running image is the right one) before
  we accept it.

So `CommitPolicy` is answering: **"Before we commit, do we require the device to
attest, yes or no?"** LivenessAndAttestation = yes, Liveness = no.

## Why that is an overlap

Look at the two questions side by side:

- `ComponentKind`: *Can this device attest?*
- `CommitPolicy`: *Do we require this device to attest?*

These are not independent. You cannot *require* attestation from a device that
is *incapable* of attesting. The second setting only makes sense in the light of
the first. They are two dials that are partly wired to the same thing —
"attestation" — but placed in two different boxes, owned by two different parts
of the system, with nothing keeping them in agreement.

## The concrete trouble: contradictory combinations

Because the two dials turn independently, a board author can set them to a
combination that makes no sense, and nothing stops them:

- **Passive device + LivenessAndAttestation.** You are demanding attestation
  from a device that has no way to attest. The requirement can never be
  satisfied, so committing its update would either hang forever or force
  someone to quietly ignore the policy. Either way the setting is a lie.
- **Active device + Liveness.** You have a device fully capable of proving
  itself, but you have told the system not to bother asking. Maybe that is
  intentional — but nothing records *why*, and it silently throws away the
  stronger guarantee the device was built to give.

Neither combination is caught at build time or run time today, because the two
settings live in two crates that do not check each other.

## Why it matters beyond "it's redundant"

It is not just tidiness. Having the same concept expressed twice means:

1. **They can drift apart.** Someone updates one dial and forgets the other, and
   now the device's declared capability and its commit requirement disagree.
2. **It is unclear who is authoritative.** If `ComponentKind` says Passive but
   `CommitPolicy` says attestation is required, which one wins? Nothing in the
   code decides, so the answer depends on whoever writes the glue later.
3. **It spreads one decision across two owners.** The state machine owns "can it
   attest," the device table owns "must it attest," but the real decision —
   *what evidence do we need before trusting this image* — is one decision and
   should have one home.

## The plain-language fix

Pick one source of truth for "does attestation apply to this device," and derive
the other from it instead of storing both:

- The cleanest option is to let **`ComponentKind` be the source of truth**. An
  Active device can attest, so its commit naturally requires attestation; a
  Passive device cannot, so its commit can only require liveness. Under that
  rule you do not need a separate `CommitPolicy` at all — it falls out of the
  kind.
- If a board genuinely needs an Active device to commit on liveness alone (a
  real exception, not an accident), then keep an explicit override, but make it
  a deliberate, validated exception layered *on top of* `ComponentKind` — so the
  system can reject the impossible combinations (like Passive + attestation)
  instead of silently trusting them.

Either way, the goal is the same: one dial for "does this device attest," owned
in one place, with the commit requirement following from it — rather than two
dials in two crates that can quietly contradict each other.
