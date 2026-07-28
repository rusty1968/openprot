// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

extern crate std;

use super::*;
use std::vec::Vec;

const C0: ComponentId = ComponentId::new(0);
const C1: ComponentId = ComponentId::new(1);
const C2: ComponentId = ComponentId::new(2);

const BOOT: Event = Event::PowerGood(PowerOnResult::Provisioned);

const CAPACITY: usize = 8;
const ECAP: usize = CAPACITY + 2;
const MAX_RETRY: u8 = 3;

fn chain(
    ids: &[(ComponentId, ComponentAttrs)],
) -> heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY> {
    let mut c = heapless::Vec::new();
    for &entry in ids {
        c.push(entry).expect("chain within CAPACITY");
    }
    c
}

fn passive_required(ids: &[ComponentId]) -> heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY> {
    chain(
        &ids.iter()
            .map(|&id| (id, ComponentAttrs::passive_required()))
            .collect::<std::vec::Vec<_>>(),
    )
}

struct Recorder {
    recorded: Vec<Effect>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            recorded: Vec::new(),
        }
    }
}

impl Platform for Recorder {
    fn execute(&mut self, effect: Effect) -> Result<(), EffectError> {
        self.recorded.push(effect);
        Ok(())
    }
}

fn drive(
    chain: heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY>,
    script: &[Event],
) -> (Vec<Effect>, State) {
    let mut orch =
        Orchestrator::<CAPACITY, ECAP>::new(chain.try_into().expect("valid chain"), MAX_RETRY);
    let mut platform = Recorder::new();
    for &event in script {
        orch.dispatch(&mut platform, event);
    }
    (platform.recorded, orch.state())
}

/// INV1/INV2/INV3: provisioned power-on walks the chain in order; no
/// component is released before its eRoT-side verification passes.
#[test]
fn cold_boot_walks_chain_in_order() {
    let (effects, state) = drive(
        passive_required(&[C0, C1]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::VerificationPassed(C1),
        ],
    );
    assert_eq!(
        effects,
        std::vec![
            Effect::ReadFirmware(C0),
            Effect::VerifyFirmware(C0),
            Effect::ReleaseReset(C0),
            Effect::ReadFirmware(C1),
            Effect::VerifyFirmware(C1),
            Effect::ReleaseReset(C1),
        ],
    );
    assert_eq!(state, State::Ready);
}

/// Unprovisioned power-on latches immediately.
#[test]
fn unprovisioned_boot_locks_down() {
    let (effects, state) = drive(
        passive_required(&[C0]),
        &[Event::PowerGood(PowerOnResult::Unprovisioned)],
    );
    assert_eq!(effects, std::vec![Effect::LatchLockdown]);
    assert_eq!(state, State::Locked);
}

/// INV11: SelfVerificationFailed latches immediately without entering
/// PreSupervision.
#[test]
fn self_verification_failure_latches_immediately() {
    let (effects, state) = drive(
        passive_required(&[C0]),
        &[Event::PowerGood(PowerOnResult::SelfVerificationFailed)],
    );
    assert_eq!(effects, std::vec![Effect::LatchLockdown]);
    assert_eq!(state, State::Locked);
}

/// INV6: AttestationChallenge is answerable from every SupervisingPlatform state.
#[test]
fn attestation_shared_across_supervising_platform_states() {
    let (effects, state) = drive(
        passive_required(&[C0]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::AttestationChallenge,
        ],
    );
    assert_eq!(effects.last(), Some(&Effect::SignAttestation));
    assert_eq!(state, State::Ready);

    let (effects, state) = drive(
        passive_required(&[C0]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::UpdateRequest,
            Event::AttestationChallenge,
        ],
    );
    assert_eq!(effects.last(), Some(&Effect::SignAttestation));
    assert_eq!(state, State::Updating);
}

/// INV4: a rejected update rolls back via DiscardStaged and never enters
/// Recovering.
#[test]
fn update_rollback_is_not_recovery() {
    let (effects, state) = drive(
        passive_required(&[C0]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::UpdateRequest,
            Event::UpdateRejected,
        ],
    );
    let tail = &effects[effects.len() - 3..];
    assert_eq!(
        tail,
        &[
            Effect::AuthenticateUpdate,
            Effect::StageUpdate,
            Effect::DiscardStaged
        ],
    );
    assert_eq!(state, State::Ready);
    assert!(!effects.contains(&Effect::LatchLockdown));
}

/// INV5: runtime corruption targets the named component and re-walks from
/// the top after restore.
#[test]
fn runtime_corruption_targets_component_and_rewalks() {
    let (effects, state) = drive(
        passive_required(&[C0, C1]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::VerificationPassed(C1),
            Event::CorruptionDetected(C1),
            Event::Restored(C1),
        ],
    );
    let tail = &effects[effects.len() - 2..];
    assert_eq!(
        tail,
        &[Effect::ReadFirmware(C0), Effect::VerifyFirmware(C0)]
    );
    assert_eq!(state, State::PreSupervision);
}

/// `PreSupervision` reacts to `CorruptionDetected` directly (via
/// [`Rot::handle_corruption`]), even though it isn't linked to
/// `SupervisingPlatform` (so `AttestationChallenge` is still discarded
/// there — a separate question). CSA defines no mechanism guaranteeing a
/// corruption report exists for an already-released component's *live,
/// executing* state (at-rest/NVM-polling is scoped to "at
/// rest"/"between boots", not an in-progress boot's chain walk) — but
/// that only means such a report isn't guaranteed to arrive, not that one
/// should be ignored if it does.
#[test]
fn corruption_during_presupervision_selfloop_triggers_recovery() {
    let (effects, state) = drive(
        passive_required(&[C0, C1, C2]),
        &[
            BOOT,
            Event::VerificationPassed(C0), // released; walk continues (still PreSupervision)
            Event::CorruptionDetected(C0), // C0 already released, but caught anyway
        ],
    );
    assert_eq!(state, State::Recovering(C0));
    assert!(effects.contains(&Effect::RestoreGoldenImage(C0)));
}

/// INV7 (feedback-as-data): after MAX_RETRY restores the core self-emits
/// RecoveryFailed and latches to Locked without any external RecoveryFailed
/// in the script.
#[test]
fn retry_cap_self_latches_via_emit() {
    let mut script = std::vec![BOOT, Event::VerificationPassed(C0)];
    script.push(Event::CorruptionDetected(C0));
    for _ in 0..(MAX_RETRY - 1) {
        script.push(Event::Restored(C0));
        script.push(Event::VerificationFailed(C0));
    }
    script.push(Event::Restored(C0));

    let (effects, state) = drive(passive_required(&[C0]), &script);

    assert!(!script.contains(&Event::RecoveryFailed));
    assert_eq!(state, State::Locked);
    assert_eq!(effects.last(), Some(&Effect::LatchLockdown));
}

/// INV7: retry count resets after a successful recovery so a later episode
/// starts from zero.
#[test]
fn retry_count_resets_after_successful_recovery() {
    let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
    c.push((C0, ComponentAttrs::passive_required()))
        .expect("fits");
    let mut orch = Orchestrator::<CAPACITY, ECAP>::new(c.try_into().expect("valid chain"), 2);
    let mut effects = Vec::new();

    for ev in [
        BOOT,
        Event::VerificationPassed(C0),
        Event::CorruptionDetected(C0),
        Event::Restored(C0),
        Event::VerificationPassed(C0),
    ] {
        orch.dispatch_with(ev, |e| {
            effects.push(e);
            Ok(())
        });
    }
    assert_eq!(orch.state(), State::Ready);

    let start = effects.len();
    for ev in [
        Event::CorruptionDetected(C0),
        Event::Restored(C0),
        Event::VerificationPassed(C0),
    ] {
        orch.dispatch_with(ev, |e| {
            effects.push(e);
            Ok(())
        });
    }
    assert_eq!(orch.state(), State::Ready);
    assert!(!effects[start..].contains(&Effect::LatchLockdown));
}

/// Retry budgets are **per component**, not a single global counter. Two
/// required components each fail exactly once (well under `max_retry = 2`)
/// in an interleaved recovery sequence before the chain finally settles.
/// Under a shared global counter the second failure would push the count to
/// the cap and latch the platform to `Locked`; per-component counting lets
/// each device use its own budget, so the walk reaches `Ready`.
#[test]
fn retry_budget_is_per_component() {
    let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
    c.push((C0, ComponentAttrs::passive_required()))
        .expect("fits");
    c.push((C1, ComponentAttrs::passive_required()))
        .expect("fits");
    let mut orch = Orchestrator::<CAPACITY, ECAP>::new(c.try_into().expect("valid chain"), 2);
    let mut effects = Vec::new();

    for ev in [
        BOOT,
        Event::VerificationFailed(C0), // C0 fails once → Recovering
        Event::Restored(C0),           // C0 count = 1 (< 2) → re-walk
        Event::VerificationPassed(C0), // C0 recovered → its streak clears
        Event::VerificationFailed(C1), // C1 fails once → Recovering
        Event::Restored(C1),           // C1 count = 1 (< 2); global would be 2 → latch
        Event::VerificationPassed(C0), // re-walk restarts at the top
        Event::VerificationPassed(C1), // chain done → Ready
    ] {
        orch.dispatch_with(ev, |e| {
            effects.push(e);
            Ok(())
        });
    }

    assert_eq!(orch.state(), State::Ready);
    assert!(!effects.contains(&Effect::LatchLockdown));
}

/// Board-supplied retry cap: max_retry = 1 latches on the first failed
/// restore.
#[test]
fn custom_retry_cap_latches_sooner() {
    let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
    c.push((C0, ComponentAttrs::passive_required()))
        .expect("fits");
    let mut orch = Orchestrator::<CAPACITY, ECAP>::new(c.try_into().expect("valid chain"), 1);
    let mut effects = Vec::new();
    for ev in [
        BOOT,
        Event::VerificationPassed(C0),
        Event::CorruptionDetected(C0),
        Event::Restored(C0),
    ] {
        orch.dispatch_with(ev, |e| {
            effects.push(e);
            Ok(())
        });
    }
    assert_eq!(orch.state(), State::Locked);
    assert_eq!(effects.last(), Some(&Effect::LatchLockdown));
}

/// Three-component chain uses N=3; walks all three to Ready.
#[test]
fn custom_capacity_walks_full_chain() {
    let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), 3>::new();
    for &id in &[C0, C1, C2] {
        c.push((id, ComponentAttrs::passive_required()))
            .expect("3 fits");
    }
    let mut orch = Orchestrator::<3, 5>::new(c.try_into().expect("valid chain"), MAX_RETRY);
    let mut effects = Vec::new();
    for ev in [
        BOOT,
        Event::VerificationPassed(C0),
        Event::VerificationPassed(C1),
        Event::VerificationPassed(C2),
    ] {
        orch.dispatch_with(ev, |e| {
            effects.push(e);
            Ok(())
        });
    }
    assert_eq!(orch.state(), State::Ready);
    assert_eq!(effects.last(), Some(&Effect::ReleaseReset(C2)));
}

/// INV10: Active component gates the chain walk — cursor does not advance
/// until ComponentReady arrives.
#[test]
fn active_component_gates_on_component_ready() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::active_required()),
            (C1, ComponentAttrs::passive_required()),
        ]),
        &[BOOT, Event::VerificationPassed(C0)],
    );
    assert_eq!(state, State::AwaitingReady(Some(C0)));
    assert!(effects.contains(&Effect::ReleaseReset(C0)));
    assert!(effects.contains(&Effect::ReadFirmware(C1)));

    let (effects2, state2) = drive(
        chain(&[
            (C0, ComponentAttrs::active_required()),
            (C1, ComponentAttrs::passive_required()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::ComponentReady(C0),
            Event::VerificationPassed(C1),
        ],
    );
    assert_eq!(state2, State::Ready);
    assert!(effects2.contains(&Effect::ReleaseReset(C1)));
}

/// INV9: a ComponentReady for the wrong id is silently ignored.
#[test]
fn spurious_component_ready_is_ignored() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::active_required()),
            (C1, ComponentAttrs::passive_required()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::ComponentReady(C1), // wrong id
        ],
    );
    assert_eq!(state, State::AwaitingReady(Some(C0)));
    assert!(!effects.contains(&Effect::ReleaseReset(C1)));
}

/// INV12: AttestationChallenge is handled in AwaitingReady.
#[test]
fn attestation_in_awaiting_ready() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::active_required()),
            (C1, ComponentAttrs::passive_required()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::AttestationChallenge,
        ],
    );
    assert_eq!(state, State::AwaitingReady(Some(C0)));
    assert_eq!(effects.last(), Some(&Effect::SignAttestation));
}

/// Isolable component: every `VerificationFailed` is retried through a full
/// recovery episode first; only once retries are exhausted does the
/// component get held in reset and the walk continue to `Ready`.
#[test]
fn isolable_component_exhausts_recovery_then_skips() {
    let mut script = std::vec![BOOT, Event::VerificationPassed(C0)];
    for _ in 0..MAX_RETRY {
        script.push(Event::VerificationFailed(C1));
        script.push(Event::Restored(C1));
        script.push(Event::VerificationPassed(C0));
    }
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::passive_required()),
            (C1, ComponentAttrs::passive_isolable()),
        ]),
        &script,
    );
    assert_eq!(state, State::Ready);
    // C1 must never be released.
    assert!(!effects.contains(&Effect::ReleaseReset(C1)));
    // Recovery IS attempted before C1 is classified and held.
    assert!(effects.contains(&Effect::RestoreGoldenImage(C1)));
    assert!(effects.contains(&Effect::AssertReset(C1)));
    assert!(!effects.contains(&Effect::LatchLockdown));
}

/// Isolable Active component failure in AwaitingReady: retried through a
/// full recovery episode, then held once exhausted; the walk still
/// reaches Ready once the remaining chain (past the held component) drains.
#[test]
fn isolable_active_component_exhausted_in_awaiting_ready_skips() {
    // C0 Active required, C1 Active isolable.
    let mut script = std::vec![BOOT, Event::VerificationPassed(C0)]; // → AwaitingReady; spec ReadFirmware(C1)
    for _ in 0..MAX_RETRY {
        script.push(Event::VerificationFailed(C1));
        script.push(Event::Restored(C1));
        script.push(Event::VerificationPassed(C0)); // re-walk restarts at C0 each episode
    }
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::active_required()),
            (C1, ComponentAttrs::active_isolable()),
        ]),
        &script,
    );
    assert_eq!(state, State::Ready);
    assert!(!effects.contains(&Effect::ReleaseReset(C1)));
    assert!(effects.contains(&Effect::RestoreGoldenImage(C1)));
    assert!(effects.contains(&Effect::AssertReset(C1)));
}

/// Runtime corruption of an `Isolable` component gates the component
/// (AssertReset) but does not trigger recovery — the machine stays in Ready.
#[test]
fn isolable_runtime_corruption_is_ignored() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::passive_required()),
            (C1, ComponentAttrs::passive_isolable()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::VerificationPassed(C1),
            Event::CorruptionDetected(C1), // Isolable → gate, no recovery
        ],
    );
    assert_eq!(state, State::Ready);
    assert!(effects.contains(&Effect::AssertReset(C1)));
    assert!(!effects.contains(&Effect::RestoreGoldenImage(C1)));
    assert!(!effects.contains(&Effect::LatchLockdown));
}

/// Runtime corruption of an `Isolable` component holds it in reset: it is
/// added to `held`, so a later re-walk triggered by a *required*
/// component's recovery skips it instead of re-releasing a component we
/// already found corrupt.
#[test]
fn isolable_runtime_corruption_holds_across_rewalk() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::passive_required()),
            (C1, ComponentAttrs::passive_isolable()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::VerificationPassed(C1),
            Event::CorruptionDetected(C1), // isolable → gate + hold
            Event::CorruptionDetected(C0), // required → Recovering
            Event::Restored(C0),           // re-walk from top
            Event::VerificationPassed(C0), // C1 stays held → chain done
        ],
    );
    assert_eq!(state, State::Ready);
    // C1 is released exactly once (the initial walk); the post-corruption
    // re-walk must not release it again.
    assert_eq!(
        effects
            .iter()
            .filter(|e| **e == Effect::ReleaseReset(C1))
            .count(),
        1,
    );
    assert!(effects.contains(&Effect::AssertReset(C1)));
}

/// A durable gate must survive a return to `Ready`. The gate set is *not*
/// cleared on `Ready` entry, so a component isolated by policy stays gated
/// across an intervening return to `Ready` and a later chain walk never
/// re-reads or re-releases it. Extends the single-walk
/// `..._holds_across_rewalk` case with a *second* return to `Ready` — the
/// exact point where the old `held.clear()` dropped the gate.
#[test]
fn gate_survives_return_to_ready() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::passive_required()),
            (C1, ComponentAttrs::passive_isolable()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::VerificationPassed(C1), // walk 1 done → Ready
            Event::CorruptionDetected(C1), // isolable → gate + hold C1
            Event::CorruptionDetected(C0), // required → Recovering
            Event::Restored(C0),           // walk 2 from top
            Event::VerificationPassed(C0), // C1 gated skip → Ready (gate must persist)
            Event::CorruptionDetected(C0), // required → Recovering again
            Event::Restored(C0),           // walk 3 from top
            Event::VerificationPassed(C0), // C1 still gated → chain done → Ready
        ],
    );
    assert_eq!(state, State::Ready);
    // C1 was read/verified and released exactly once, on walk 1. If the
    // gate were dropped at `Ready`, walk 3 would re-read and re-release it.
    assert_eq!(
        effects
            .iter()
            .filter(|e| **e == Effect::ReadFirmware(C1))
            .count(),
        1,
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| **e == Effect::ReleaseReset(C1))
            .count(),
        1,
    );
}

/// Runtime corruption of a `Required` component still triggers
/// recovery as before.
#[test]
fn required_runtime_corruption_triggers_recovery() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::passive_required()),
            (C1, ComponentAttrs::passive_isolable()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::VerificationPassed(C1),
            Event::CorruptionDetected(C0), // required → Recovering
        ],
    );
    assert_eq!(state, State::Recovering(C0));
    assert!(effects.contains(&Effect::RestoreGoldenImage(C0)));
}

/// Runtime corruption of a `Cascading` component must gate the whole
/// cascade, not just the reported component. The runtime-corruption path
/// and the recovery-exhaustion path share one `gate_by_policy` source of
/// truth, so `Cascading` cascades in both. C2 `depends_on` C1; corrupting
/// C1 must assert reset on C1 **and** C2.
#[test]
fn cascading_runtime_corruption_cascades() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::passive_required()),
            (C1, ComponentAttrs::passive_cascading()),
            (C2, ComponentAttrs::passive_required().with_depends_on(C1)),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::VerificationPassed(C1),
            Event::VerificationPassed(C2), // walk done → Ready
            Event::CorruptionDetected(C1), // Cascading → gate C1 and its dependents
        ],
    );
    assert_eq!(state, State::Ready);
    // The whole cascade is gated: C1 (the root) and C2 (its dependent).
    assert!(effects.contains(&Effect::AssertReset(C1)));
    assert!(effects.contains(&Effect::AssertReset(C2)));
    // No recovery is started for a non-required corruption.
    assert!(!effects.contains(&Effect::RestoreGoldenImage(C1)));
    assert!(!effects.contains(&Effect::LatchLockdown));
}

/// Boot-time VerificationFailed on a required component → Recovering.
/// (Distinct from CorruptionDetected: this is a failed eRoT-side check
/// before the component is ever released from reset.)
#[test]
fn boot_failure_required_enters_recovering() {
    let (effects, state) = drive(
        passive_required(&[C0, C1]),
        &[BOOT, Event::VerificationFailed(C0)],
    );
    assert_eq!(state, State::Recovering(C0));
    assert!(effects.contains(&Effect::RestoreGoldenImage(C0)));
    // Component must never be released when its eRoT check failed.
    assert!(!effects.contains(&Effect::ReleaseReset(C0)));
}

/// Full boot-failure recovery cycle: VerificationFailed → Recovering →
/// Restored → re-walk from top → VerificationPassed → Ready.
#[test]
fn boot_failure_recovery_cycle_completes() {
    let (effects, state) = drive(
        passive_required(&[C0]),
        &[
            BOOT,
            Event::VerificationFailed(C0),
            Event::Restored(C0),
            Event::VerificationPassed(C0),
        ],
    );
    assert_eq!(state, State::Ready);
    assert!(effects.contains(&Effect::RestoreGoldenImage(C0)));
    // ReleaseReset only after the recovery re-walk passes.
    assert!(effects.contains(&Effect::ReleaseReset(C0)));
}

/// VerificationFailed (required) on a speculative check while the machine
/// is in AwaitingReady → enters Recovering without releasing the component.
#[test]
fn required_failure_in_awaiting_ready_enters_recovering() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::active_required()),
            (C1, ComponentAttrs::passive_required()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0), // → AwaitingReady; spec check of C1 starts
            Event::VerificationFailed(C1), // required → Recovering
        ],
    );
    assert_eq!(state, State::Recovering(C1));
    assert!(effects.contains(&Effect::RestoreGoldenImage(C1)));
    assert!(!effects.contains(&Effect::ReleaseReset(C1)));
}

/// CorruptionDetected while in AwaitingReady (required component) →
/// Recovering via the SupervisingPlatform superstate handler.
#[test]
fn corruption_in_awaiting_ready_triggers_recovery() {
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::active_required()),
            (C1, ComponentAttrs::passive_required()),
        ]),
        &[
            BOOT,
            Event::VerificationPassed(C0), // → AwaitingReady
            Event::CorruptionDetected(C0),
        ],
    );
    assert_eq!(state, State::Recovering(C0));
    assert!(effects.contains(&Effect::RestoreGoldenImage(C0)));
}

/// CorruptionDetected while in Updating (required component) → Recovering
/// via the SupervisingPlatform superstate handler.
#[test]
fn corruption_in_updating_triggers_recovery() {
    let (effects, state) = drive(
        passive_required(&[C0]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::UpdateRequest,
            Event::CorruptionDetected(C0),
        ],
    );
    assert_eq!(state, State::Recovering(C0));
    assert!(effects.contains(&Effect::RestoreGoldenImage(C0)));
}

/// UpdateVerified activates the staged image and returns to Ready.
/// (Complements update_rollback_is_not_recovery which tests UpdateRejected.)
#[test]
fn update_verified_activates_update() {
    let (effects, state) = drive(
        passive_required(&[C0]),
        &[
            BOOT,
            Event::VerificationPassed(C0),
            Event::UpdateRequest,
            Event::UpdateVerified,
        ],
    );
    assert_eq!(state, State::Ready);
    assert!(effects.contains(&Effect::ActivateUpdate));
    assert!(!effects.contains(&Effect::DiscardStaged));
    assert!(!effects.contains(&Effect::RestoreGoldenImage(C0)));
}

/// Locked is a terminal state: no effects are produced in response to any
/// event after the machine latches.
#[test]
fn locked_is_terminal() {
    let mut c: heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY> = heapless::Vec::new();
    c.push((C0, ComponentAttrs::passive_required())).unwrap();
    // max_retry = 1 so the first failed restore latches immediately.
    let mut orch = Orchestrator::<CAPACITY, ECAP>::new(c.try_into().expect("valid chain"), 1);
    let mut effects: Vec<Effect> = Vec::new();

    for ev in [BOOT, Event::VerificationFailed(C0), Event::Restored(C0)] {
        orch.dispatch_with(ev, |e| {
            effects.push(e);
            Ok(())
        });
    }
    assert_eq!(orch.state(), State::Locked);

    let count_before = effects.len();
    for ev in [
        BOOT,
        Event::VerificationPassed(C0),
        Event::AttestationChallenge,
        Event::UpdateRequest,
        Event::CorruptionDetected(C0),
    ] {
        orch.dispatch_with(ev, |e| {
            effects.push(e);
            Ok(())
        });
    }
    assert_eq!(
        effects.len(),
        count_before,
        "Locked state must produce no effects"
    );
}

/// An Isolable component at the head of the chain exhausts its recovery
/// retries, gets held, and the walk continues to the remaining required
/// components.
#[test]
fn isolable_first_component_exhausts_then_walk_continues() {
    let mut script = std::vec![BOOT];
    for _ in 0..MAX_RETRY {
        script.push(Event::VerificationFailed(C0));
        script.push(Event::Restored(C0));
    }
    script.push(Event::VerificationPassed(C1));
    let (effects, state) = drive(
        chain(&[
            (C0, ComponentAttrs::passive_isolable()),
            (C1, ComponentAttrs::passive_required()),
        ]),
        &script,
    );
    assert_eq!(state, State::Ready);
    assert!(!effects.contains(&Effect::ReleaseReset(C0)));
    assert!(effects.contains(&Effect::ReleaseReset(C1)));
    // Recovery IS attempted before C0 is classified and held.
    assert!(effects.contains(&Effect::RestoreGoldenImage(C0)));
    assert!(effects.contains(&Effect::AssertReset(C0)));
}

/// The speculative read emits ReleaseReset · ReadFirmware · VerifyFirmware
/// all in the same handler as VerificationPassed for an Active component,
/// before ComponentReady has arrived. Verifies both presence and order.
#[test]
fn speculative_read_effects_are_emitted_together() {
    let mut orch = Orchestrator::<CAPACITY, ECAP>::new(
        chain(&[
            (C0, ComponentAttrs::active_required()),
            (C1, ComponentAttrs::passive_required()),
        ])
        .try_into()
        .expect("valid chain"),
        MAX_RETRY,
    );
    let mut effects: Vec<Effect> = Vec::new();

    orch.dispatch_with(BOOT, |e| {
        effects.push(e);
        Ok(())
    });
    assert_eq!(
        effects,
        std::vec![Effect::ReadFirmware(C0), Effect::VerifyFirmware(C0)],
    );

    effects.clear();
    orch.dispatch_with(Event::VerificationPassed(C0), |e| {
        effects.push(e);
        Ok(())
    });
    // All three effects emitted in the same handler, before ComponentReady.
    assert_eq!(
        effects,
        std::vec![
            Effect::ReleaseReset(C0),
            Effect::ReadFirmware(C1),
            Effect::VerifyFirmware(C1),
        ],
    );
    assert_eq!(orch.state(), State::AwaitingReady(Some(C0)));
}

/// A chain with a single Active component goes directly to Ready on
/// VerificationPassed — no AwaitingReady, no ComponentReady required.
/// This exercises the `chain done` branch of PreSupervision for an
/// Active component (distinct from the multi-component Active path which
/// transitions to AwaitingReady).
#[test]
fn single_active_chain_goes_directly_to_ready() {
    let mut c: heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY> = heapless::Vec::new();
    c.push((C0, ComponentAttrs::active_required())).unwrap();
    let (effects, state) = drive(c, &[BOOT, Event::VerificationPassed(C0)]);
    assert_eq!(state, State::Ready);
    assert_eq!(
        effects,
        std::vec![
            Effect::ReadFirmware(C0),
            Effect::VerifyFirmware(C0),
            Effect::ReleaseReset(C0),
        ],
    );
}

/// An empty component list is not a valid chain of trust.
#[test]
fn chain_rejects_empty() {
    let empty = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
    assert_eq!(Chain::try_from(empty).unwrap_err(), ChainError::Empty);
}

/// A repeated `ComponentId` is rejected: the reducer's linear id lookups would
/// otherwise be ambiguous.
#[test]
fn chain_rejects_duplicate_id() {
    let v = chain(&[
        (C0, ComponentAttrs::passive_required()),
        (C0, ComponentAttrs::passive_required()),
    ]);
    assert_eq!(Chain::try_from(v).unwrap_err(), ChainError::DuplicateId(C0),);
}

/// A `depends_on` that names a component not in the chain is rejected.
#[test]
fn chain_rejects_unknown_dependency() {
    let v = chain(&[(C1, ComponentAttrs::passive_required().with_depends_on(C0))]);
    assert_eq!(
        Chain::try_from(v).unwrap_err(),
        ChainError::UnknownDependency {
            component: C1,
            depends_on: C0,
        },
    );
}

/// A dependency must appear strictly earlier in the walk than its dependent;
/// a forward reference is rejected.
#[test]
fn chain_rejects_forward_dependency() {
    let v = chain(&[
        (C0, ComponentAttrs::passive_required().with_depends_on(C1)),
        (C1, ComponentAttrs::passive_cascading()),
    ]);
    assert_eq!(
        Chain::try_from(v).unwrap_err(),
        ChainError::ForwardDependency {
            component: C0,
            depends_on: C1,
        },
    );
}

/// A component may not depend on itself.
#[test]
fn chain_rejects_self_dependency() {
    let v = chain(&[(C0, ComponentAttrs::passive_required().with_depends_on(C0))]);
    assert_eq!(
        Chain::try_from(v).unwrap_err(),
        ChainError::ForwardDependency {
            component: C0,
            depends_on: C0,
        },
    );
}

/// A well-formed chain with a backward dependency validates successfully.
#[test]
fn chain_accepts_valid_dependency() {
    let v = chain(&[
        (C0, ComponentAttrs::passive_cascading()),
        (C1, ComponentAttrs::passive_required().with_depends_on(C0)),
    ]);
    assert!(Chain::try_from(v).is_ok());
}

/// A [`Platform`] that records every effect and fails a chosen one, to exercise
/// the effect failure channel.
struct FailOn {
    trigger: Effect,
    recorded: Vec<Effect>,
    failed: bool,
}

impl FailOn {
    fn new(trigger: Effect) -> Self {
        Self {
            trigger,
            recorded: Vec::new(),
            failed: false,
        }
    }
}

impl Platform for FailOn {
    fn execute(&mut self, effect: Effect) -> Result<(), EffectError> {
        self.recorded.push(effect);
        if effect == self.trigger {
            self.failed = true;
            Err(EffectError)
        } else {
            Ok(())
        }
    }
}

/// A failed reset actuation is fail-closed: the driver injects `EffectFailed`
/// and the machine latches to `Locked`, emitting `LatchLockdown`.
#[test]
fn effect_failure_latches_lockdown() {
    let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
    c.push((C0, ComponentAttrs::passive_required())).unwrap();
    let mut orch =
        Orchestrator::<CAPACITY, ECAP>::new(c.try_into().expect("valid chain"), MAX_RETRY);
    let mut plat = FailOn::new(Effect::ReleaseReset(C0));

    orch.dispatch(&mut plat, BOOT); // ReadFirmware/VerifyFirmware C0 — both succeed
    orch.dispatch(&mut plat, Event::VerificationPassed(C0)); // ReleaseReset(C0) fails

    assert!(plat.failed, "the trigger effect should have been attempted");
    assert_eq!(orch.state(), State::Locked);
    assert!(plat.recorded.contains(&Effect::LatchLockdown));
}

/// A failed isolation actuation (`AssertReset`) is equally fail-closed: even a
/// non-required component's containment failing latches the platform.
#[test]
fn failed_isolation_actuation_latches_lockdown() {
    let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
    c.push((C0, ComponentAttrs::passive_required())).unwrap();
    c.push((C1, ComponentAttrs::passive_isolable())).unwrap();
    let mut orch =
        Orchestrator::<CAPACITY, ECAP>::new(c.try_into().expect("valid chain"), MAX_RETRY);
    let mut plat = FailOn::new(Effect::AssertReset(C1));

    orch.dispatch(&mut plat, BOOT);
    orch.dispatch(&mut plat, Event::VerificationPassed(C0));
    orch.dispatch(&mut plat, Event::VerificationPassed(C1)); // C1 released → Ready
    orch.dispatch(&mut plat, Event::CorruptionDetected(C1)); // isolable → AssertReset(C1) fails

    assert!(plat.failed);
    assert_eq!(orch.state(), State::Locked);
    assert!(plat.recorded.contains(&Effect::LatchLockdown));
}

/// A failed recovery actuation is fail-closed too: if the shell cannot even
/// restore a required component's golden image, the platform latches rather
/// than continuing with an unrecovered component.
#[test]
fn failed_restore_actuation_latches_lockdown() {
    let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
    c.push((C0, ComponentAttrs::passive_required())).unwrap();
    let mut orch =
        Orchestrator::<CAPACITY, ECAP>::new(c.try_into().expect("valid chain"), MAX_RETRY);
    let mut plat = FailOn::new(Effect::RestoreGoldenImage(C0));

    orch.dispatch(&mut plat, BOOT);
    orch.dispatch(&mut plat, Event::VerificationFailed(C0)); // → Recovering → RestoreGoldenImage(C0) fails

    assert!(plat.failed);
    assert_eq!(orch.state(), State::Locked);
    assert!(plat.recorded.contains(&Effect::LatchLockdown));
}

/// The lockdown latch is the last line of defense: even if *it* fails to
/// actuate, the machine must not spin. The re-injected `EffectFailed` is
/// ignored while `Locked`, so dispatch terminates and the latch is attempted
/// exactly once.
#[test]
fn failed_lockdown_actuation_does_not_loop() {
    let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
    c.push((C0, ComponentAttrs::passive_required())).unwrap();
    let mut orch =
        Orchestrator::<CAPACITY, ECAP>::new(c.try_into().expect("valid chain"), MAX_RETRY);
    let mut plat = FailOn::new(Effect::LatchLockdown);

    // An unprovisioned power-on latches immediately; the latch actuation fails.
    orch.dispatch(&mut plat, Event::PowerGood(PowerOnResult::Unprovisioned));

    assert!(plat.failed, "the lockdown latch should have been attempted");
    assert_eq!(orch.state(), State::Locked);
    assert_eq!(
        plat.recorded
            .iter()
            .filter(|&&e| e == Effect::LatchLockdown)
            .count(),
        1,
        "a failing latch must not re-latch forever",
    );
}

/// Actuation is fail-fast: once an effect in a batch fails, no effect ordered
/// *after* it is attempted. Here `VerificationPassed(C0)` emits the batch
/// `[ReleaseReset(C0), ReadFirmware(C1), VerifyFirmware(C1)]`; failing the first
/// effect must abandon the two speculative reads of `C1` and latch, rather than
/// actuate them for a transition that is immediately overridden by `Locked`.
#[test]
fn batch_actuation_is_fail_fast() {
    let mut orch = Orchestrator::<CAPACITY, ECAP>::new(
        passive_required(&[C0, C1]).try_into().expect("valid chain"),
        MAX_RETRY,
    );
    let mut plat = FailOn::new(Effect::ReleaseReset(C0));

    orch.dispatch(&mut plat, BOOT); // ReadFirmware/VerifyFirmware C0 — both succeed
    orch.dispatch(&mut plat, Event::VerificationPassed(C0)); // ReleaseReset(C0) fails first

    assert!(plat.failed, "the failing effect should have been attempted");
    assert!(
        plat.recorded.contains(&Effect::ReleaseReset(C0)),
        "the failing effect itself is attempted",
    );
    assert!(
        !plat.recorded.contains(&Effect::ReadFirmware(C1)),
        "an effect ordered after the failure must not be actuated",
    );
    assert!(
        !plat.recorded.contains(&Effect::VerifyFirmware(C1)),
        "an effect ordered after the failure must not be actuated",
    );
    assert_eq!(orch.state(), State::Locked);
    assert!(plat.recorded.contains(&Effect::LatchLockdown));
}
