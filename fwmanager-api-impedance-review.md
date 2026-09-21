# fwmanager/api impedance-mismatch review

Review of `services/fwmanager/api/src` (`boot_control.rs`, `boot_monitor.rs`,
`config.rs`, `lib.rs`) against the `orchestrator-sm` domain model
(`ComponentId`, `ComponentKind`, `Event`, `Effect`) and the
`orchestrator-timer` crate (`TimerManager`, `Expired`), plus the runtime seam in
`services/orchestrator/server/src/runtime.rs` that bridges them.

## Surfaces compared

- **fwmanager/api** — `BootControl` (`hold_in_reset`/`release`), `BootMonitor`
  (`boot_status -> Result<BootStatus, Error>`, `BootStatus {Booting, Booted,
  Failed}`), and the board schema in `config.rs`: `CommitPolicy {Liveness,
  LivenessAndAttestation}`, `BootSignal<G> {GpioBootComplete, Heartbeat,
  MctpReady, VersionQuery}`, `BootCheckpoint<G> {name, signal, window:
  core::time::Duration}`, `DeviceConfig<R, G> {name, reset_signal, checkpoints,
  commit_policy}`.
- **orchestrator-sm** — `ComponentId(u8)`, `ComponentKind {Active, Passive}`,
  `Event` (incl. `ComponentReady(id)`, `Booted(id)`, `Timeout(id)`,
  `BootConfirmed(id)`, `CommitTimeout`, `EffectFailed`), `Effect` (incl.
  `ReleaseReset(id)`, `AssertReset(id)`, `CommitSvnFloor(id)`).
- **orchestrator-timer** — `TimerManager<T, Id, N>` (one boot watchdog per `Id`,
  single commit slot), `Expired<Id> {Boot(Id), Commit}`.

## Mismatches

### 1. `BootProgress::Booted` — dangling reference
sm's `Event::Booted` doc says it "Mirrors fwmanager's `BootProgress::Booted`",
but no `BootProgress` type exists anywhere in fwmanager. The real type is
`BootStatus` (`boot_monitor.rs`), variants `Booting`/`Booted`/`Failed`. Either a
rename drifted or the mirror was never built. Cheapest fix: update the sm doc to
name `BootStatus::Booted`.

### 2. `BootStatus::Failed` has no sm event (lost fast-fail)
`BootMonitor` can return `BootStatus::Failed` (device-reported boot failure), but
sm's `Event` vocabulary has no boot-failure input for a released component — only
`Booted`/`ComponentReady` (success) and `Timeout` (watchdog). A device that
*actively* reports `Failed` cannot be delivered as a distinct event; the runtime
must drop it and wait for `Event::Timeout`, or synthesize an early timeout. The
`boot_monitor` doc justifies the timeout path for *hung* devices, but an
actively-failed device gets the same slow path — there is no fast-fail seam.

### 3. Two incompatible `Duration` types
`BootCheckpoint.window` in `config.rs` is `core::time::Duration`; the runtime's
`BootWatchdogs::arm_boot`/`arm_commit` take `userspace::time::Duration`. Feeding a
config window into a watchdog needs an explicit conversion that nothing in these
types provides.

### 4. N checkpoints per device → 1 watchdog per component (lost checkpoint identity)
`config.rs` models a device as an ordered `&[BootCheckpoint]`, each with its own
`name` and `window` ("booted when the last one is reached"). But sm/timer model
one boot-progress watchdog per `ComponentId`: `TimerManager::arm_boot(id, …)`
*replaces* in place, and `Event::Timeout(ComponentId)` carries only the id. The
runtime must collapse the checkpoint sequence onto a single re-armed watchdog,
and `BootCheckpoint.name` — documented as "Names the checkpoint in timeout
reports" — has nowhere to go: the timeout event has no field for it.

### 5. `BootSignal` transport vs `ComponentKind` tier — orthogonal, unlinked
`BootSignal<G>` classifies the signal *transport* (`GpioBootComplete`,
`Heartbeat`, `MctpReady`, `VersionQuery`). sm's `ComponentKind` {Active, Passive}
decides whether progress maps to `ComponentReady` (Active) or `Booted`
(Passive). These are different axes: `DeviceConfig` carries no `ComponentKind`,
and sm carries no transport. Nothing enforces the join, so a device's configured
signal and its sm tier can silently disagree.

### 6. `CommitPolicy` (per-device) vs single, id-less commit machinery
`config.rs` attaches `CommitPolicy` {Liveness, LivenessAndAttestation} per
`DeviceConfig`. But the timer has exactly one commit slot (`Expired::Commit`, a
single `Option<T>`), and sm gates commit through `Event::BootConfirmed(id)` /
`Effect::CommitSvnFloor(id)` / `Event::CommitTimeout` — where `CommitTimeout`
carries no id. Two devices in a commit window simultaneously cannot be
distinguished. And the policy semantics (does `Liveness` alone raise
`BootConfirmed`? does `LivenessAndAttestation` require an extra attestation pass
first?) have no representation in the event vocabulary.

### 7. `CommitPolicy` duplicates `ComponentKind`'s attestation axis
`ComponentKind::Active` already means "has iRoT, gets self-verification";
`CommitPolicy::LivenessAndAttestation` separately encodes "commit needs
attestation." Two crates, two attestation-flavored axes, no shared type — risk of
an Active device configured `Liveness`, or a Passive one configured
`LivenessAndAttestation`, with nothing rejecting it.

### 8. Split component identity / two parallel config tables
`DeviceConfig` carries `name`, `reset_signal`, `checkpoints`, `commit_policy` —
but not `ComponentId`, `ComponentKind`, `FailurePolicy`, or `depends_on`, all of
which sm needs per component to build its `Chain`. A board supplies two
per-component tables keyed by different identities (`DeviceConfig.name`/index vs
`ComponentId`) with no linking key between them.

### 9. Rich `core::error::Error` collapses to payload-less `EffectFailed`
`BootControl`/`BootMonitor` require `Error: core::error::Error` (Display +
`source()` chain, downcastable). But when `hold_in_reset`/`release` fails, sm has
only `Event::EffectFailed` — no id, no error payload — which just latches
`Locked`. The cause chain the api preserves is discarded at the sm seam.

### 10. `ReleaseReset`/`AssertReset` vs `release`/`hold_in_reset`
sm emits `Effect::ReleaseReset(id)`/`AssertReset(id)`; the api trait is
`release()`/`hold_in_reset()`. Semantically aligned but no shared vocabulary, and
the `ComponentId -> BootControl` lookup the runtime must perform is unspecified
(compounds #8).

## Notable non-mismatch

The generic-id split in the timer (`Expired<Id>` + generic `Id`) is not a
mismatch — the runtime pins `Id = ComponentId` and maps `Expired::Boot(id) ->
Event::Timeout(id)`, `Expired::Commit -> Event::CommitTimeout`. Worth flagging:
`runtime.rs` is still on the old timer API (`TimerManager<Instant, N>`,
`poll_expired` returns `Event` directly) and will not compile against the leaf
timer until updated — and that is the layer where mismatches #2–#6 would actually
be bridged, yet today it does none of that bridging.

## Priority

- **Trivial:** #1 (doc fix).
- **Structural, high-leverage:** #4 and #6 (checkpoint-name and per-device-commit
  identity are genuinely unrepresentable in the current event types), #8 (missing
  `ComponentId` key on `DeviceConfig`).
- **Design alignment:** #2, #5, #7, #9.
- **Minor:** #3 (conversion), #10 (naming).
