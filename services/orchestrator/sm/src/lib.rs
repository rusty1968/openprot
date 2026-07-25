// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! `openprot_orchestrator_sm` — the eRoT boot-sequence state machine.
//!
//! This is the pure-reducer core ported from `rot_reducer`. It describes side
//! effects as [`Effect`] values rather than performing them; the surrounding
//! OpenPRoT shell carries them out via a [`Platform`] impl. No concrete hardware
//! appears here — the machine is generic over an opaque [`ComponentId`].
//!
//! See `docs/verification-model.md` and `docs/state-machine.md` in the
//! `rot_reducer` workspace for the full domain context and design rationale.
//!
//! Three invariants define the boundary:
//!   1. **Effects flow through [`Sink`]** — fresh per event, drained afterward.
//!   2. **Feedback as data ([`Effect::Emit`])** — follow-up events are effects,
//!      visible in the trace; used for the retry cap (INV7).
//!   3. **Reads as events** — outside information arrives in [`Event`] payloads;
//!      the core never reads anything directly.

#![no_std]
#![forbid(unsafe_code)]

use core::marker::PhantomData;

use statig::blocking::{
    IntoStateMachine, IntoStateMachineExt as _, State as StatigState, StateMachine,
    Superstate as StatigSuperstate,
};
use statig::Outcome;

// Internal capacities — these follow from how the machine works, not from the
// deployment. The board owns CAPACITY (chain length) and max_retry.

/// Max effects one event can emit. The busiest handler emits 3; 8 is plenty.
const EFFECT_CAP: usize = 8;

/// Max pending events while settling one outside event (original + Emit follow-ups).
const PENDING_CAP: usize = 8;

/// An opaque identifier for one platform component. The core never inspects it;
/// the board layer decides which real hardware each id refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ComponentId(u8);

impl ComponentId {
    pub const fn new(id: u8) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

/// How a component in the trust chain is classified. The board supplies one
/// [`ComponentKind`] per [`ComponentId`] when building the chain.
///
/// Corresponds directly to the two-tier model in the CSA architecture document:
/// `Active` = eRoT gate + iRoT gate; `Passive` = eRoT gate only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ComponentKind {
    /// Has an integrated iRoT (e.g. Caliptra). Both eRoT-side (signature + SVN)
    /// and iRoT-side (local self-verification) checks apply. The machine waits in
    /// [`State::AwaitingReady`] for [`Event::ComponentReady`] before advancing.
    Active,
    /// No integrated iRoT. The eRoT's signature + SVN check is the only gate.
    /// The chain walk advances immediately after `ReleaseReset`.
    Passive,
}

/// Recovery-failure classification: what the machine does once a required
/// component's restore attempts are **exhausted** (`retry_count` reaches
/// `max_retry`). Every verification or corruption failure enters
/// [`State::Recovering`] and is retried first, regardless of this
/// classification — CSA's "recover first" principle. This value is consulted
/// only after retries are exhausted.
///
/// (The narrative design docs sometimes call the `Required` outcome "platform
/// halt" — same behavior, this is the type-level name.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FailurePolicy {
    /// Stop the boot sequence entirely: self-emits [`Event::RecoveryFailed`],
    /// which drives the machine to [`State::Locked`].
    Required,
    /// Hold this component in reset (added to `Rot.held`) and continue
    /// booting the rest of the platform.
    Isolable,
    /// Hold this component **and** any component whose `depends_on` names it
    /// (transitively), then continue booting the rest of the platform.
    Cascading,
}

/// Opaque recovery-region key supplied by the board at chain-build time.
/// Components sharing a `RegionId` are restored together: when any region
/// member enters [`State::Recovering`], the shell resolves and restores the
/// whole region. The core treats this as an equality key only and never
/// inspects membership itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RegionId(u8);

impl RegionId {
    pub const fn new(id: u8) -> Self {
        Self(id)
    }
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Per-component attributes supplied by the board at chain-build time.
///
/// Three orthogonal axes:
/// - [`kind`](ComponentAttrs::kind): controls the iRoT gate (Active vs Passive).
/// - [`failure_policy`](ComponentAttrs::failure_policy): controls what happens
///   once this component's recovery is exhausted.
/// - [`recovery_region`](ComponentAttrs::recovery_region) /
///   [`depends_on`](ComponentAttrs::depends_on): control restore grouping and
///   cascade-skip on exhaustion.
///
/// A component that fails verification is never released from reset —
/// running untrusted firmware would break the trust invariant regardless of
/// `failure_policy`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ComponentAttrs {
    pub kind: ComponentKind,
    pub failure_policy: FailurePolicy,
    pub recovery_region: RegionId,
    pub depends_on: Option<ComponentId>,
}

impl ComponentAttrs {
    pub const fn active_required() -> Self {
        Self {
            kind: ComponentKind::Active,
            failure_policy: FailurePolicy::Required,
            recovery_region: RegionId::new(0),
            depends_on: None,
        }
    }
    pub const fn passive_required() -> Self {
        Self {
            kind: ComponentKind::Passive,
            failure_policy: FailurePolicy::Required,
            recovery_region: RegionId::new(0),
            depends_on: None,
        }
    }
    pub const fn active_isolable() -> Self {
        Self {
            kind: ComponentKind::Active,
            failure_policy: FailurePolicy::Isolable,
            recovery_region: RegionId::new(0),
            depends_on: None,
        }
    }
    pub const fn passive_isolable() -> Self {
        Self {
            kind: ComponentKind::Passive,
            failure_policy: FailurePolicy::Isolable,
            recovery_region: RegionId::new(0),
            depends_on: None,
        }
    }
    pub const fn active_cascading() -> Self {
        Self {
            kind: ComponentKind::Active,
            failure_policy: FailurePolicy::Cascading,
            recovery_region: RegionId::new(0),
            depends_on: None,
        }
    }
    pub const fn passive_cascading() -> Self {
        Self {
            kind: ComponentKind::Passive,
            failure_policy: FailurePolicy::Cascading,
            recovery_region: RegionId::new(0),
            depends_on: None,
        }
    }

    /// Builder: assign a non-default recovery region (default is region `0`).
    pub const fn with_region(mut self, region: RegionId) -> Self {
        self.recovery_region = region;
        self
    }

    /// Builder: mark this component as cascade-held whenever `dependency` is
    /// held. Only meaningful in combination with `FailurePolicy::Cascading`
    /// on the *dependency*, not on this component.
    pub const fn with_depends_on(mut self, dependency: ComponentId) -> Self {
        self.depends_on = Some(dependency);
        self
    }
}

/// The result of the board's power-on checks, delivered inside [`Event::PowerGood`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerOnResult {
    /// Self-verified and provisioned.
    Provisioned,
    /// Self-verified but not provisioned — cannot act as a RoT.
    Unprovisioned,
    /// Self-verification failed — latches immediately to [`State::Locked`].
    SelfVerificationFailed,
}

/// Everything the outside world can tell the state machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    /// Power-on, carrying the shell's self-verification and provisioning result.
    PowerGood(PowerOnResult),
    VerificationPassed(ComponentId),
    VerificationFailed(ComponentId),
    /// An `Active` component's iRoT has finished local verification and is ready
    /// (e.g. MCTP channel established).
    ComponentReady(ComponentId),
    AttestationChallenge,
    UpdateRequest,
    UpdateVerified,
    UpdateRejected,
    CorruptionDetected(ComponentId),
    Restored(ComponentId),
    RecoveryFailed,
}

/// Everything the state machine can ask the outside world to do.
///
/// [`Effect::Emit`] is the sole internal effect: the orchestrator catches it and
/// queues the carried event for immediate handling, making follow-up events
/// visible in the effect trace instead of hidden state changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    ReadFirmware(ComponentId),
    VerifyFirmware(ComponentId),
    ReleaseReset(ComponentId),
    /// Assert reset on a component that is already running — the inverse of
    /// [`ReleaseReset`]. Emitted when an `Isolable`/`Cascading` component is
    /// found corrupt at runtime, or is held after recovery is exhausted: the
    /// component is gated without triggering (or continuing) a recovery cycle.
    AssertReset(ComponentId),
    SignAttestation,
    AuthenticateUpdate,
    StageUpdate,
    ActivateUpdate,
    DiscardStaged,
    RestoreGoldenImage(ComponentId),
    LatchLockdown,
    /// Internal only — tells the orchestrator to handle this event next.
    /// Never forwarded to a [`Platform`].
    Emit(Event),
}

/// The states the machine can be in. None carry data; all mutable state lives
/// in [`Rot`] shared storage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    PowerOnReset,
    PreSupervision,
    /// eRoT has released an `Active` component; waiting for its iRoT to finish
    /// local verification and signal [`Event::ComponentReady`].
    AwaitingReady,
    Ready,
    Updating,
    Recovering,
    Locked,
}

/// Superstate entered on the eRoT's first component release and held until
/// [`State::Locked`]. Provides two platform-wide guarantees that must hold
/// across all four sub-states ([`State::AwaitingReady`], [`State::Ready`],
/// [`State::Updating`], [`State::Recovering`]):
///
/// - Attestation challenges are always answered.
/// - Corruption of a required component always triggers recovery.
#[derive(Debug)]
pub enum Superstate<'sub> {
    SupervisingPlatform(PhantomData<&'sub ()>),
}

/// The effect buffer handed to every handler (statig's `Context`).
///
/// The only thing a handler can do to the outside world is call `emit`. The
/// orchestrator gives each event a fresh `Sink` and drains it afterward.
pub struct Sink {
    effects: heapless::Vec<Effect, EFFECT_CAP>,
}

impl Sink {
    fn new() -> Self {
        Self {
            effects: heapless::Vec::new(),
        }
    }

    /// Append one effect. Overflow is silently dropped rather than panicking
    /// (`no_std` safety); overflow means a logic bug.
    pub fn emit(&mut self, effect: Effect) {
        let _ = self.effects.push(effect);
    }

    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }
}

/// Shared storage: data that persists across events. `N` is the chain capacity
/// — a board choice; the core sets no default.
pub struct Rot<const N: usize> {
    chain: heapless::Vec<(ComponentId, ComponentAttrs), N>,
    cursor: u8,
    /// Components skipped because their recovery was exhausted under
    /// `FailurePolicy::Isolable` or `Cascading`. Held in reset; not
    /// re-verified on subsequent re-walks. Cleared on `Ready` entry.
    held: heapless::Vec<ComponentId, N>,
    failed: Option<ComponentId>,
    retry_count: u8,
    max_retry: u8,
    /// The `Active` component whose iRoT readiness is outstanding. `Some` only
    /// while in `AwaitingReady` (INV9).
    awaiting: Option<ComponentId>,
}

impl<const N: usize> Rot<N> {
    pub fn new(chain: heapless::Vec<(ComponentId, ComponentAttrs), N>, max_retry: u8) -> Self {
        Self {
            chain,
            cursor: 0,
            held: heapless::Vec::new(),
            failed: None,
            retry_count: 0,
            max_retry,
            awaiting: None,
        }
    }

    /// Look up a component's attributes by id. `None` if the id is not in the
    /// chain (should never happen for ids the core itself produced).
    fn attrs_of(&self, id: ComponentId) -> Option<ComponentAttrs> {
        self.chain.iter().find(|(cid, _)| *cid == id).map(|(_, a)| *a)
    }

    fn is_held(&self, id: ComponentId) -> bool {
        self.held.iter().any(|h| *h == id)
    }

    /// Advance `cursor` from `start_idx` to the first component not in
    /// `held`, emitting its `ReadFirmware`/`VerifyFirmware`. Returns `true` if
    /// found. If the rest of the chain is exhausted or entirely held, sets
    /// `cursor` to a past-the-end sentinel (`chain.len()`) and returns
    /// `false` — the caller should treat that as "chain done".
    fn advance_to_next_unheld(&mut self, ctx: &mut Sink, start_idx: usize) -> bool {
        let mut idx = start_idx;
        while let Some(&(id, _)) = self.chain.get(idx) {
            if !self.is_held(id) {
                // `idx < chain.len() <= N`; chain capacity is board-chosen and
                // assumed to fit `u8`, matching the existing `cursor: u8` contract.
                self.cursor = idx as u8;
                ctx.emit(Effect::ReadFirmware(id));
                ctx.emit(Effect::VerifyFirmware(id));
                return true;
            }
            idx += 1;
        }
        self.cursor = self.chain.len() as u8;
        false
    }

    /// Hold `root` and cascade-hold every component whose `depends_on`
    /// (transitively) names it. Emits `AssertReset` for each newly held
    /// component, including `root` itself.
    fn cascade_hold(&mut self, ctx: &mut Sink, root: ComponentId) {
        if !self.is_held(root) {
            ctx.emit(Effect::AssertReset(root));
            let _ = self.held.push(root);
        }
        let mut i = 0;
        while let Some(&holder) = self.held.get(i) {
            i += 1;
            let mut newly_held: heapless::Vec<ComponentId, N> = heapless::Vec::new();
            for &(id, attrs) in self.chain.iter() {
                if attrs.depends_on == Some(holder) && !self.is_held(id) {
                    let _ = newly_held.push(id);
                }
            }
            for id in newly_held {
                ctx.emit(Effect::AssertReset(id));
                let _ = self.held.push(id);
            }
        }
    }
}

impl<const N: usize> IntoStateMachine for Rot<N> {
    type Event<'evt> = Event;
    type Context<'ctx> = Sink;
    type State = State;
    type Superstate<'sub> = Superstate<'sub>;

    fn initial() -> State {
        State::PowerOnReset
    }
}

impl<const N: usize> StatigState<Rot<N>> for State {
    fn call_handler(&mut self, rot: &mut Rot<N>, event: &Event, ctx: &mut Sink) -> Outcome<State> {
        match self {
            State::PowerOnReset => match event {
                Event::PowerGood(PowerOnResult::Provisioned) => {
                    Outcome::Transition(State::PreSupervision)
                }
                Event::PowerGood(PowerOnResult::Unprovisioned) => {
                    Outcome::Transition(State::Locked)
                }
                Event::PowerGood(PowerOnResult::SelfVerificationFailed) => {
                    Outcome::Transition(State::Locked)
                }
                _ => Outcome::Super,
            },

            // Cursor walk via Outcome::Handled — a self-transition would reset cursor.
            State::PreSupervision => match event {
                Event::VerificationPassed(id) => {
                    ctx.emit(Effect::ReleaseReset(*id));
                    let current_kind = rot.chain.get(rot.cursor as usize).map(|(_, a)| a.kind);
                    let next_idx = (rot.cursor as usize).saturating_add(1);
                    if rot.advance_to_next_unheld(ctx, next_idx) {
                        match current_kind {
                            Some(ComponentKind::Active) => {
                                rot.awaiting = Some(*id);
                                Outcome::Transition(State::AwaitingReady)
                            }
                            _ => Outcome::Handled,
                        }
                    } else {
                        Outcome::Transition(State::Ready)
                    }
                }
                Event::VerificationFailed(id) => {
                    // Recovery is attempted first for every failure, regardless
                    // of the component's recovery-failure policy (CSA: recover
                    // first, classify only once retries are exhausted).
                    rot.failed = Some(*id);
                    Outcome::Transition(State::Recovering)
                }
                _ => Outcome::Super,
            },

            State::AwaitingReady => match event {
                Event::ComponentReady(id) => {
                    if rot.awaiting != Some(*id) {
                        return Outcome::Handled; // spurious / stale (INV9)
                    }
                    rot.awaiting = None;
                    // If cursor is past the end, the eRoT side of the walk has
                    // already finished (chain done, or the remainder is held) —
                    // nothing left to verify, we're done.
                    if (rot.cursor as usize) >= rot.chain.len() {
                        Outcome::Transition(State::Ready)
                    } else {
                        Outcome::Handled
                    }
                }
                Event::VerificationPassed(id) => {
                    ctx.emit(Effect::ReleaseReset(*id));
                    let next_idx = (rot.cursor as usize).saturating_add(1);
                    if rot.advance_to_next_unheld(ctx, next_idx) {
                        Outcome::Handled
                    } else {
                        Outcome::Transition(State::Ready)
                    }
                }
                Event::VerificationFailed(id) => {
                    // Recovery is attempted first for every failure, regardless
                    // of the component's recovery-failure policy.
                    rot.failed = Some(*id);
                    rot.awaiting = None;
                    Outcome::Transition(State::Recovering)
                }
                _ => Outcome::Super,
            },

            State::Ready => match event {
                Event::UpdateRequest => Outcome::Transition(State::Updating),
                _ => Outcome::Super,
            },

            State::Updating => match event {
                Event::UpdateVerified => {
                    ctx.emit(Effect::ActivateUpdate);
                    Outcome::Transition(State::Ready)
                }
                Event::UpdateRejected => {
                    ctx.emit(Effect::DiscardStaged);
                    Outcome::Transition(State::Ready)
                }
                _ => Outcome::Super,
            },

            State::Recovering => match event {
                Event::Restored(_) => {
                    rot.retry_count = rot.retry_count.saturating_add(1);
                    if rot.retry_count < rot.max_retry {
                        Outcome::Transition(State::PreSupervision)
                    } else {
                        // Retries exhausted: consult the recovery-failure policy.
                        let classification = rot.failed.and_then(|id| {
                            rot.attrs_of(id).map(|attrs| (id, attrs.failure_policy))
                        });
                        match classification {
                            Some((id, FailurePolicy::Isolable)) => {
                                ctx.emit(Effect::AssertReset(id));
                                let _ = rot.held.push(id);
                                rot.failed = None;
                                rot.retry_count = 0;
                                Outcome::Transition(State::PreSupervision)
                            }
                            Some((id, FailurePolicy::Cascading)) => {
                                rot.cascade_hold(ctx, id);
                                rot.failed = None;
                                rot.retry_count = 0;
                                Outcome::Transition(State::PreSupervision)
                            }
                            // `Required` (or an unknown/missing id — safe default): halt.
                            _ => {
                                ctx.emit(Effect::Emit(Event::RecoveryFailed));
                                Outcome::Handled
                            }
                        }
                    }
                }
                Event::RecoveryFailed => Outcome::Transition(State::Locked),
                _ => Outcome::Super,
            },

            State::Locked => Outcome::Super,
        }
    }

    fn call_entry_action(&mut self, rot: &mut Rot<N>, ctx: &mut Sink) {
        match self {
            State::PreSupervision => {
                rot.awaiting = None;
                let _ = rot.advance_to_next_unheld(ctx, 0);
            }
            State::Updating => {
                ctx.emit(Effect::AuthenticateUpdate);
                ctx.emit(Effect::StageUpdate);
            }
            State::Recovering => {
                if let Some(failed) = rot.failed {
                    ctx.emit(Effect::RestoreGoldenImage(failed));
                }
            }
            State::Locked => {
                ctx.emit(Effect::LatchLockdown);
            }
            State::Ready => {
                rot.retry_count = 0;
                rot.held.clear();
                rot.failed = None;
            }
            _ => {}
        }
    }

    fn superstate(&mut self) -> Option<Superstate<'_>> {
        match self {
            State::Ready | State::Updating | State::Recovering | State::AwaitingReady => {
                Some(Superstate::SupervisingPlatform(PhantomData))
            }
            _ => None,
        }
    }
}

impl<const N: usize> StatigSuperstate<Rot<N>> for Superstate<'_> {
    fn call_handler(&mut self, rot: &mut Rot<N>, event: &Event, ctx: &mut Sink) -> Outcome<State> {
        match self {
            Superstate::SupervisingPlatform(_) => match event {
                Event::AttestationChallenge => {
                    ctx.emit(Effect::SignAttestation);
                    Outcome::Handled
                }
                Event::CorruptionDetected(id) => {
                    // Respect the per-component policy encoded at chain-build time.
                    // `FailurePolicy::Required` → recover (halt chain, restore, re-walk)
                    // `Isolable` / `Cascading` → gate the component; it stays running
                    //   but is not considered trusted by the core, and no recovery
                    //   episode is started (it is already known to be skippable).
                    let required = rot
                        .attrs_of(*id)
                        .map(|attrs| matches!(attrs.failure_policy, FailurePolicy::Required))
                        .unwrap_or(true); // unknown id: treat as required (safe default)
                    if required {
                        rot.failed = Some(*id);
                        Outcome::Transition(State::Recovering)
                    } else {
                        // Isolable/Cascading: gate the component (put it back in
                        // reset) but do not halt the chain or trigger recovery.
                        ctx.emit(Effect::AssertReset(*id));
                        Outcome::Handled
                    }
                }
                _ => Outcome::Super,
            },
        }
    }
}

/// Outward connection to the platform. Carry out one effect. Never called with
/// [`Effect::Emit`] — the orchestrator consumes those internally.
pub trait Platform {
    fn execute(&mut self, effect: Effect);
}

/// A handle for a caller's own event loop. Wraps the statig machine so callers
/// only depend on this crate, never on statig types directly.
pub struct Orchestrator<const N: usize> {
    machine: StateMachine<Rot<N>>,
}

impl<const N: usize> Orchestrator<N> {
    pub fn new(chain: heapless::Vec<(ComponentId, ComponentAttrs), N>, max_retry: u8) -> Self {
        Self {
            machine: Rot::new(chain, max_retry).state_machine(),
        }
    }

    pub fn state(&self) -> State {
        *self.machine.state()
    }

    /// Handle one event all the way through — including any [`Effect::Emit`]
    /// follow-ups — calling `on_effect` for each external effect in order.
    pub fn dispatch_with(&mut self, event: Event, mut on_effect: impl FnMut(Effect)) {
        let mut pending: heapless::Vec<Event, PENDING_CAP> = heapless::Vec::new();
        let _ = pending.push(event);

        let mut i = 0;
        while i < pending.len() {
            let ev = pending[i];
            i += 1;

            let mut buf = Sink::new();
            self.machine.handle_with_context(&ev, &mut buf);

            for &effect in buf.effects() {
                match effect {
                    Effect::Emit(internal) => {
                        let _ = pending.push(internal);
                    }
                    external => on_effect(external),
                }
            }
        }
    }

    /// Same as [`dispatch_with`] but routes effects to a [`Platform`].
    pub fn dispatch(&mut self, platform: &mut impl Platform, event: Event) {
        self.dispatch_with(event, |effect| platform.execute(effect));
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    const C0: ComponentId = ComponentId::new(0);
    const C1: ComponentId = ComponentId::new(1);
    const C2: ComponentId = ComponentId::new(2);

    const BOOT: Event = Event::PowerGood(PowerOnResult::Provisioned);

    const CAPACITY: usize = 8;
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

    fn passive_required(
        ids: &[ComponentId],
    ) -> heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY> {
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
        fn execute(&mut self, effect: Effect) {
            self.recorded.push(effect);
        }
    }

    fn drive(
        chain: heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY>,
        script: &[Event],
    ) -> (Vec<Effect>, State) {
        let mut orch = Orchestrator::new(chain, MAX_RETRY);
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
        let mut orch = Orchestrator::new(c, 2);
        let mut effects = Vec::new();

        for ev in [
            BOOT,
            Event::VerificationPassed(C0),
            Event::CorruptionDetected(C0),
            Event::Restored(C0),
            Event::VerificationPassed(C0),
        ] {
            orch.dispatch_with(ev, |e| effects.push(e));
        }
        assert_eq!(orch.state(), State::Ready);

        let start = effects.len();
        for ev in [
            Event::CorruptionDetected(C0),
            Event::Restored(C0),
            Event::VerificationPassed(C0),
        ] {
            orch.dispatch_with(ev, |e| effects.push(e));
        }
        assert_eq!(orch.state(), State::Ready);
        assert!(!effects[start..].contains(&Effect::LatchLockdown));
    }

    /// Board-supplied retry cap: max_retry = 1 latches on the first failed
    /// restore.
    #[test]
    fn custom_retry_cap_latches_sooner() {
        let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
        c.push((C0, ComponentAttrs::passive_required()))
            .expect("fits");
        let mut orch = Orchestrator::new(c, 1);
        let mut effects = Vec::new();
        for ev in [
            BOOT,
            Event::VerificationPassed(C0),
            Event::CorruptionDetected(C0),
            Event::Restored(C0),
        ] {
            orch.dispatch_with(ev, |e| effects.push(e));
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
        let mut orch = Orchestrator::new(c, MAX_RETRY);
        let mut effects = Vec::new();
        for ev in [
            BOOT,
            Event::VerificationPassed(C0),
            Event::VerificationPassed(C1),
            Event::VerificationPassed(C2),
        ] {
            orch.dispatch_with(ev, |e| effects.push(e));
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
        assert_eq!(state, State::AwaitingReady);
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
        assert_eq!(state, State::AwaitingReady);
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
        assert_eq!(state, State::AwaitingReady);
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
        assert_eq!(state, State::Recovering);
        assert!(effects.contains(&Effect::RestoreGoldenImage(C0)));
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
        assert_eq!(state, State::Recovering);
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
        assert_eq!(state, State::Recovering);
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
        assert_eq!(state, State::Recovering);
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
        assert_eq!(state, State::Recovering);
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
        let mut c: heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY> =
            heapless::Vec::new();
        c.push((C0, ComponentAttrs::passive_required())).unwrap();
        // max_retry = 1 so the first failed restore latches immediately.
        let mut orch = Orchestrator::new(c, 1);
        let mut effects: Vec<Effect> = Vec::new();

        for ev in [BOOT, Event::VerificationFailed(C0), Event::Restored(C0)] {
            orch.dispatch_with(ev, |e| effects.push(e));
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
            orch.dispatch_with(ev, |e| effects.push(e));
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
        let mut orch = Orchestrator::<CAPACITY>::new(
            chain(&[
                (C0, ComponentAttrs::active_required()),
                (C1, ComponentAttrs::passive_required()),
            ]),
            MAX_RETRY,
        );
        let mut effects: Vec<Effect> = Vec::new();

        orch.dispatch_with(BOOT, |e| effects.push(e));
        assert_eq!(
            effects,
            std::vec![Effect::ReadFirmware(C0), Effect::VerifyFirmware(C0)],
        );

        effects.clear();
        orch.dispatch_with(Event::VerificationPassed(C0), |e| effects.push(e));
        // All three effects emitted in the same handler, before ComponentReady.
        assert_eq!(
            effects,
            std::vec![
                Effect::ReleaseReset(C0),
                Effect::ReadFirmware(C1),
                Effect::VerifyFirmware(C1),
            ],
        );
        assert_eq!(orch.state(), State::AwaitingReady);
    }

    /// A chain with a single Active component goes directly to Ready on
    /// VerificationPassed — no AwaitingReady, no ComponentReady required.
    /// This exercises the `chain done` branch of PreSupervision for an
    /// Active component (distinct from the multi-component Active path which
    /// transitions to AwaitingReady).
    #[test]
    fn single_active_chain_goes_directly_to_ready() {
        let mut c: heapless::Vec<(ComponentId, ComponentAttrs), CAPACITY> =
            heapless::Vec::new();
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
}
