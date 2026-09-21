# Runtime corruption realignment plan

The goal is to make the way the state machine reacts to runtime corruption match
what CSA asks for: **try to recover the component first, and only isolate it if
recovery keeps failing.** Right now it doesn't always do that. This plan explains
the gap and how to close it.

## What's wrong today

CSA says corruption can be found in two places: when a component boots, and when
the eRoT polls stored firmware in the background. In both cases, the eRoT is
supposed to run its recovery routine. It only gives up on a component — isolates
it, cascades to its dependents, or halts the platform — *after a recovery attempt
has already failed*, never the first time it sees a problem.

The code (`services/orchestrator/sm/src/lib.rs`) doesn't follow that rule
consistently. When corruption is reported at runtime, `handle_corruption` looks at
the component's policy and does the following:

- **Required (or an unknown id):** record the component as failed and go into
  `Recovering`. This is correct — it tries to recover first.
- **Isolable:** immediately reset the component and gate it. No recovery attempt.
- **Cascading:** immediately reset the component and its dependents. No recovery
  attempt.

So Isolable and Cascading components get isolated the moment corruption is
reported, without ever trying to recover them. That's not what CSA asks for, and
it's also inconsistent with how the machine handles a failure *at boot*, where it
always tries to recover first no matter what the policy is.

## What we want instead

Treat runtime corruption the same way for every policy: go into `Recovering`
first, and let the existing "recovery exhausted" logic decide whether to isolate,
cascade, or halt. The `gate_by_policy` function stays the one place that makes that
decision — it just gets called only after recovery has been exhausted, not on the
first report.

## One decision to make before we start

A runtime corruption report can arrive for a component that has already been
released and is running. CSA doesn't cover this case directly (its two detection
points are at boot and at rest). There are two ways to handle it:

- **Option A — just recover.** Mark the component failed and go into `Recovering`.
  Don't reset it right away. The downside: the corrupt component keeps running
  until the recovery step resets it.
- **Option B — stop it, then recover (recommended).** Reset the component right
  away to stop the corrupt code from running, then mark it failed and go into
  `Recovering`. If recovery keeps failing, the existing logic makes the reset
  permanent (Isolable/Cascading) or halts the platform (Required). This keeps the
  component contained *and* still tries to recover it. CSA's recovery routine
  resets the device anyway, so this fits.

**Recommendation: Option B.** The rest of the plan assumes it.

### Two smaller questions that come with Option B

1. **Don't make the reset permanent too early.** The reset we do to stop the
   corrupt component should *not* add it to the `gated` list yet. That list is what
   makes a gate permanent, and we only want that once recovery has actually been
   exhausted. Keeping it off the list lets a retry re-check and re-release the
   component if recovery succeeds — same as the boot path.
2. **Confirm this behavior change is OK.** Today, a corrupt Isolable/Cascading
   component is isolated instantly. With Option B it gets recovery retries first.
   We should confirm nobody was relying on the instant-isolation behavior for
   security reasons.

## Code changes (`services/orchestrator/sm/src/lib.rs`)

1. **`handle_corruption`** — simplify it so it no longer branches on policy:
   - Known component, not already gated: reset it (to stop it running), mark it
     failed, and go into `Recovering`.
   - Known component that's already gated: do nothing — it's already contained.
   - Unknown id: do nothing. Today an unknown id wrongly kicks off a recovery for a
     component that isn't even in the chain; this fixes that.
2. **`gate_by_policy`** — no change. It just gets called from one place now (the
   recovery-exhausted step) instead of two.
3. **Recovery-exhausted step** — no real change, but double-check that resetting
   the component earlier doesn't cause a duplicate: the early reset only emits an
   effect, it doesn't add the component to `gated`, so the existing "already
   gated?" guard still works.
4. Add a short comment in `handle_corruption` noting why we reset at runtime but
   not at boot (at boot the component was never released, so it's already in
   reset).

## Test changes (`services/orchestrator/sm/src/lib.rs`, `mod tests`)

A few tests currently expect the instant-isolation behavior and need to be updated
to drive the component through recovery and exhaustion instead:

- `isolable_runtime_corruption_is_ignored` — corruption now resets the component
  and starts recovery (state becomes `Recovering`).
- `isolable_runtime_corruption_holds_across_rewalk` — fail recovery `MAX_RETRY`
  times before the component becomes permanently held.
- `gate_survives_return_to_ready` — reach the permanent gate through
  exhaustion, then check it survives a return to `Ready`.
- `cascading_runtime_corruption_cascades` — the cascade now happens
  once recovery is exhausted, not on the first report.
- **New test** `unknown_id_corruption_is_ignored` — an unknown id causes no
  recovery and no state change.

## Doc changes

- `docs/src/design/orchestrator/orchestrator-sm-transitions.md`: update the two
  "component not required" corruption entries to describe the new flow — reset,
  then recover, then apply the policy only if recovery is exhausted.
- `docs/src/design/orchestrator/orchestrator-machine.md` and
  `orchestrator-sm-walkthru.md`: update the corruption transitions and the note
  about runtime monitoring.

## How to check it

- Run `get_errors` on `lib.rs` after each edit.
- Run
  `bazelisk test //services/orchestrator/sm:orchestrator_sm_test --nocache_test_results`
  from `/home/antrocha/work/apps/typeconstructor/openprot`.

## Things to keep in mind

- This is a **behavior change, not just a cleanup.** A corrupt Isolable/Cascading
  component now gets recovery retries instead of being isolated right away.
- We reset at runtime but not at boot. That's on purpose (boot components are
  already in reset), but it's worth a comment in the code so it doesn't look like a
  mistake.

## Related issue — the retry counter is shared, but CSA treats it per-device

CSA never says exactly how many recovery attempts to allow, but everywhere it talks
about retries and giving up, it talks about *one device at a time*:

- "Recovery scope is a per-device, platform-configurable policy rather than a
  single global behavior" (Boot Sequence, Recovery Policy).
- "If a managed device exhausts recovery attempts without success..." (Resiliency,
  Degraded Mode) — it's talking about a single device.
- The isolate/cascade/halt decision applies "to the affected device(s)" (Boot
  Sequence, Recovery Policy).

In the code, `retry_count` is one shared counter on `Rot`. It goes up on every
`Restored` event, no matter which component is being recovered. This works today
only because the machine recovers one component at a time and the counter resets
when it returns to `Ready` or after it gives up on a component. But the counter
isn't tied to a specific component. To match CSA properly, each component should
have its own attempt count (for example, a small `Vec<(ComponentId, u8)>` next to
the existing `gated`/`failed` bookkeeping).

**Scope:** this is separate from the recover-first work above and can be done on
its own later. It only becomes a real problem if the machine ever recovers more
than one component at a time. With today's one-at-a-time model it's a latent issue,
not an active bug.
