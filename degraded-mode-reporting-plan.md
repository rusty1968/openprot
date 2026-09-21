# Degraded-Mode Reporting — Implementation Plan

## Problem

The CSA resiliency chapter ("Degraded Mode") requires two things when a managed
component exhausts recovery or is otherwise taken out of service:

> - **Report the failure** through the platform management interface so
>   operators and management software are aware.
> - **Isolate the failed device** — prevent it from participating in the
>   platform while allowing the remaining devices to continue operating.

The orchestrator reducer currently does the **isolate** half only (it emits
`AssertReset` and records the component in `gated`). It emits **no reporting
effect**, so a BMC / management stack has no in-band signal from the reducer
when a component is isolated or when the platform halts because a `Required`
component could not be recovered. This is the one substantive CSA
non-compliance found in the audit.

## Goal

Emit an explicit report effect at every point where a component is taken out of
service, and one when recovery exhaustion forces platform lockdown — without
changing any existing isolation or lockdown behavior.

## Scope

In scope (required for compliance):

- Report each component isolated under `Isolable` (recovery exhausted).
- Report each component isolated under `Cascading` (recovery exhausted) — the
  root **and** every transitive dependent.
- Report each component isolated by runtime `CorruptionDetected` under a
  non-`Required` policy.
- Report the `Required` component whose recovery was exhausted, before the
  machine latches to `Locked`.

Explicitly out of scope (not required by the degraded-mode clause; note as
possible follow-ups):

- Reporting successful recovery (`Restored` → re-verify passes). Telemetry
  nicety, not a degraded-mode requirement.
- Reporting every `LatchLockdown` cause beyond the recovery-exhaustion case
  (self-verification-failed / unprovisioned already route to `Locked` from
  `PowerOnReset`; a separate boot-abort report could be added later).

## Design

### New effect variants (`model.rs`)

Add two variants to `Effect`:

```rust
/// Report that a component has been isolated (held in reset and removed from
/// the trust chain) so management software is aware of the degraded platform.
/// Emitted once per component at the moment it is gated.
ReportIsolated(ComponentId),

/// Report that a `Required` component exhausted recovery, immediately before
/// the machine latches to `Locked`. Names the component that forced the halt.
ReportRecoveryFailed(ComponentId),
```

Rationale for two flat variants over a single `Report(ComponentId, Reason)`
enum: matches the existing flat-`Effect` style, each maps cleanly to one IPC
message, and the two cases have different downstream meaning (degraded-but-up
vs about-to-halt). Keep them payload-minimal (just the `ComponentId`); the shell
logs specifics on its side, mirroring the `EffectError` convention.

### Emission points (`lib.rs`)

All isolation flows through two functions, so reporting is centralized:

1. **`gate_by_policy`, `Isolable` branch** — right after
   `ctx.emit(Effect::AssertReset(id))`, add
   `ctx.emit(Effect::ReportIsolated(id))`. Both sit inside the existing
   `if !self.is_gated(id)` guard, so exactly one report per newly gated
   component.

2. **`cascade_hold`** — after each `ctx.emit(Effect::AssertReset(...))` (both the
   `root` emission and the per-dependent emission in the worklist loop), add a
   matching `ctx.emit(Effect::ReportIsolated(...))`, inside the same `is_gated`
   guards. Guarantees one report per isolated dependent, no duplicates.

3. **`Recovering` exhaustion, `NotGated` branch** (the `Required`/unknown case in
   `handle`) — before `ctx.emit(Effect::Emit(Event::RecoveryFailed))`, add
   `ctx.emit(Effect::ReportRecoveryFailed(failed))`.

No new emission is needed in `handle_corruption` itself — it delegates to
`gate_by_policy`, so it inherits reporting automatically. This preserves the
"single source of truth" property (runtime-corruption and
recovery-exhaustion paths report identically).

### Effect-buffer capacity (`lib.rs`)

Adding one `ReportIsolated` per `AssertReset` changes the worst-case single-
`Sink` effect count. Today's binding worst case is a full cascade during a
recovery re-walk: `N` `AssertReset`s (all components cascade-held) plus the
destination `PreSupervision` entry's `ReadFirmware` + `VerifyFirmware`, i.e.
`N + 2`. With one report per gated component that becomes `2N + 2`.

Required changes:

- `Rot::EFFECT_CAP_OK`: change the assertion from `E >= N + 2` to
  `E >= 2 * N + 2`, and update its message.
- `Sink::emit` doc comment and the `Sink` type doc: update the worst-case
  description from "`N` `AssertReset`s ... plus 2" to "`N` `AssertReset`s +
  `N` `ReportIsolated`s ... plus 2".
- `PENDING_CAP` is **unaffected** — no new `Effect::Emit` is introduced, so the
  event queue worst case (outside event + one `Emit` follow-up + one
  `EffectFailed`) is unchanged.

Note the cap increase requires every board's chosen `E` to satisfy the new
floor; a board sized exactly at `N + 2` will now fail to compile until it bumps
`E`. Callers in `tests.rs` use `ECAP = CAPACITY + 2`; this must become
`2 * CAPACITY + 2` (see below).

### Fail-fast / ordering interaction

`ReportIsolated` is emitted immediately after its `AssertReset`, so in the
driver's in-order batch the report follows the reset actuation. If the
`AssertReset` fails, fail-fast aborts the batch before the report — acceptable:
the machine is latching to `Locked` and a broader lockdown report supersedes a
per-component isolation report. `ReportRecoveryFailed` precedes
`Emit(RecoveryFailed)` (an internal effect), so it is actuated before the
machine transitions toward `Locked`. No ordering hazard: reports only add
information and never gate a release.

## Test plan (`tests.rs`)

Existing assertions are almost all `.contains()` presence checks, so adding
report effects will not break them. Two spots need attention:

- `ECAP` constant → `2 * CAPACITY + 2` (buffer floor changed).
- The two `assert_eq!(effects, vec![Effect::LatchLockdown])` exact-vector tests
  (the `Unprovisioned`/`SelfVerificationFailed` → `Locked` paths) are on the
  `PowerOnReset` lockdown path, which emits no isolation/recovery report — so
  they remain correct and need no change. Confirm they still pass.

New tests to add:

1. `isolable_exhaustion_reports_isolation` — drive an `Isolable` component
   through `max_retry` failed restores; assert `effects` contains
   `ReportIsolated(id)` and the machine continues (not `Locked`).
2. `cascading_exhaustion_reports_each_isolated` — `Cascading` root with a
   dependent; assert `ReportIsolated` present for **both** the root and the
   dependent.
3. `runtime_corruption_isolable_reports` — `CorruptionDetected` on an
   `Isolable` component in a supervised state; assert `ReportIsolated(id)` and
   no `RestoreGoldenImage`.
4. `required_exhaustion_reports_before_lockdown` — `Required` component through
   exhaustion; assert `ReportRecoveryFailed(id)` appears in the trace and
   precedes `LatchLockdown`.
5. `successful_recovery_emits_no_isolation_report` — component recovers within
   the retry cap; assert no `ReportIsolated` / `ReportRecoveryFailed`.
6. `report_isolated_emitted_once_per_component` — re-trigger corruption on an
   already-gated component; assert no duplicate `ReportIsolated` (guarded by
   `is_gated`).

## Validation

- `rustfmt --edition 2024 services/orchestrator/sm/src/lib.rs services/orchestrator/sm/src/model.rs services/orchestrator/sm/src/tests.rs`
- `bazelisk test //services/orchestrator/sm:orchestrator_sm_test --test_output=errors --nocache_test_results`
  (run from `openprot/`, via the execution subagent).

## Rollout / commits

Two terse commits (code only; no markdown staged per convention):

1. `orchestrator-sm: add degraded-mode isolation/recovery-failure reports`
   — `Effect::ReportIsolated` + `Effect::ReportRecoveryFailed`, emission points,
   and the `E >= 2N + 2` cap bump.
2. `orchestrator-sm: test degraded-mode reporting` — the six new tests plus the
   `ECAP` bump.

(Or a single squashed commit if preferred.)

## Open questions

- **Report granularity**: one effect per isolated component (this plan) vs a
  single batch report listing all cascade victims. Per-component keeps `Effect`
  atomic and IPC-message-shaped but grows the effect buffer to `2N + 2`; a batch
  variant would need a bounded payload (`heapless::Vec<ComponentId, N>` inside
  the effect), which complicates the `Copy`-able `Effect` enum. Per-component is
  recommended.
- **Reason payload**: whether `ReportIsolated` should distinguish "isolated due
  to runtime corruption" from "isolated due to recovery exhaustion." The CSA
  clause does not require the distinction; omitted here to keep the effect
  payload-minimal, but easy to add later as a second field if operators need it.
