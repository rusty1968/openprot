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
// deployment. The board owns `N` (chain length), `E` (effect-buffer size) and
// max_retry.

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
/// component's restore attempts are **exhausted** (its per-component retry
/// count reaches `max_retry`). Every verification or corruption failure enters
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
    /// Hold this component in reset (added to `Rot.gated`) and continue
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

/// Superstate entered once the eRoT exits [`State::PreSupervision`] — i.e. on
/// release of the first `Active` component, or once the whole chain has
/// finished if it is all-`Passive`. Provides two platform-wide guarantees
/// that must hold across all four sub-states ([`State::AwaitingReady`],
/// [`State::Ready`], [`State::Updating`], [`State::Recovering`]):
///
/// - Attestation challenges are always answered.
/// - Corruption of a required component always triggers recovery.
///
/// Note: [`State::PreSupervision`] itself is *not* linked to this superstate
/// (its `superstate()` returns `None`), so the attestation guarantee does not
/// hold while a component is still being walked there — CSA defines no
/// requirement to answer challenges before the chain walk completes, so
/// `AttestationChallenge` is left unhandled in `PreSupervision` (discarded via
/// `Outcome::Super`).
///
/// The corruption guarantee, however, *does* hold in `PreSupervision`:
/// [`State::PreSupervision`] handles [`Event::CorruptionDetected`] directly
/// (via [`Rot::handle_corruption`]) rather than through this superstate,
/// since linking the whole superstate in would also pull in the attestation
/// behavior above. CSA defines no mechanism guaranteeing a corruption report
/// arrives for an already-released component's *live, executing* state (its
/// only at-rest mechanism — background NVM integrity polling — is explicitly
/// scoped to "at rest"/"between boots", not an in-progress boot's chain
/// walk), but that only means such a report isn't guaranteed to exist — it's
/// not a reason to discard one if it does arrive. See
/// `corruption_during_presupervision_selfloop_triggers_recovery` below.
#[derive(Debug)]
pub enum Superstate<'sub> {
    SupervisingPlatform(PhantomData<&'sub ()>),
}

/// The effect buffer handed to every handler (statig's `Context`), sized to `E`.
///
/// The only thing a handler can do to the outside world is call `emit`. The
/// orchestrator gives each event a fresh `Sink` and drains it afterward.
///
/// `E` is bounded from below by the chain length: the worst single event is a
/// full cascade (up to `N` `AssertReset`s) plus the destination
/// `PreSupervision` entry's `ReadFirmware`/`VerifyFirmware` (2), all landing in
/// one `Sink`. `Rot::new` refuses to compile unless `E >= N + 2`, so a machine
/// that builds can never overflow this buffer.
pub struct Sink<const E: usize> {
    effects: heapless::Vec<Effect, E>,
}

impl<const E: usize> Sink<E> {
    fn new() -> Self {
        Self {
            effects: heapless::Vec::new(),
        }
    }

    /// Append one effect. `E` is sized so overflow is impossible for a machine
    /// that compiles (the `E >= N + 2` floor in `Rot::new`); the panic is a
    /// loud, fail-closed backstop for a future handler that emits beyond the
    /// proven worst case, never a silent drop of a security-critical effect.
    pub fn emit(&mut self, effect: Effect) {
        if self.effects.push(effect).is_err() {
            panic!("effect buffer overflow: E must be >= chain length N + 2");
        }
    }

    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }
}

/// Outcome of [`Rot::gate_by_policy`]: whether a component was gated out of
/// service. Collapses the three [`FailurePolicy`] values into the two
/// control-flow outcomes the caller actually branches on — so the
/// runtime-corruption path and the recovery-exhaustion path decide on the same
/// result.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gating {
    /// The component was gated (`Isolable` → itself; `Cascading` → itself and
    /// its transitive dependents). Its reset is asserted and the walk skips it.
    Gated,
    /// Not a gating policy (`Required`, or an unknown/missing id). Nothing was
    /// gated; the caller handles this in its own context — recover (at runtime)
    /// or lock down (once recovery is exhausted).
    NotGated,
}

/// Shared storage: data that persists across events. `N` is the chain capacity
/// and `E` the effect-buffer size — both board choices; the core sets no
/// default. `E` must be at least `N + 2` (enforced in [`Rot::new`]).
pub struct Rot<const N: usize, const E: usize> {
    chain: heapless::Vec<(ComponentId, ComponentAttrs), N>,
    cursor: u8,
    /// Components physically held in reset by a policy decision: runtime
    /// corruption of a non-required component, or recovery exhausted under
    /// `FailurePolicy::Isolable` or `Cascading`. Each id here has a live
    /// `AssertReset` and is skipped on every chain walk. This is a **durable**
    /// trust-boundary gate: it persists across a return to `Ready` and is only
    /// cleared by a fresh `Rot` on `PowerOnReset`.
    gated: heapless::Vec<ComponentId, N>,
    failed: Option<ComponentId>,
    /// Per-component consecutive failed-restore counts. An entry is present only
    /// for a component with at least one recorded attempt; absent means zero.
    /// Keyed by `ComponentId` so interleaved recoveries of different components
    /// never share a retry budget — CSA frames recovery-attempt exhaustion per
    /// device, not as a single global counter. A component's entry is cleared
    /// when it passes verification (recovered) or is gated, and the whole map is
    /// cleared on a clean return to `Ready`.
    retries: heapless::Vec<(ComponentId, u8), N>,
    max_retry: u8,
    /// The `Active` component whose iRoT readiness is outstanding. `Some` only
    /// while in `AwaitingReady` (INV9).
    awaiting: Option<ComponentId>,
    /// Ties the effect-buffer size `E` to this type (zero-sized).
    _effect_cap: PhantomData<[u8; E]>,
}

impl<const N: usize, const E: usize> Rot<N, E> {
    /// Compile-time floor: the effect buffer must hold a full cascade (`N`
    /// `AssertReset`s) plus the destination `PreSupervision` entry's two
    /// effects. Forced by `new` below, so an under-sized `E` fails to build.
    const EFFECT_CAP_OK: () = assert!(
        E >= N + 2,
        "effect buffer E must be >= chain length N + 2"
    );

    pub fn new(chain: heapless::Vec<(ComponentId, ComponentAttrs), N>, max_retry: u8) -> Self {
        let () = Self::EFFECT_CAP_OK;
        Self {
            chain,
            cursor: 0,
            gated: heapless::Vec::new(),
            failed: None,
            retries: heapless::Vec::new(),
            max_retry,
            awaiting: None,
            _effect_cap: PhantomData,
        }
    }

    /// Look up a component's attributes by id. `None` if the id is not in the
    /// chain (should never happen for ids the core itself produced).
    fn attrs_of(&self, id: ComponentId) -> Option<ComponentAttrs> {
        self.chain.iter().find(|(cid, _)| *cid == id).map(|(_, a)| *a)
    }

    fn is_gated(&self, id: ComponentId) -> bool {
        self.gated.iter().any(|h| *h == id)
    }

    /// Increment `id`'s consecutive failed-restore count and return the new
    /// value. Counts are kept per component so that interleaved recovery
    /// episodes for different components never share a budget.
    fn bump_retry(&mut self, id: ComponentId) -> u8 {
        for entry in self.retries.iter_mut() {
            if entry.0 == id {
                entry.1 = entry.1.saturating_add(1);
                return entry.1;
            }
        }
        let _ = self.retries.push((id, 1));
        1
    }

    /// Drop `id`'s retry count. Called when the component recovers (passes
    /// verification) or is gated — either way its consecutive-failure streak
    /// ends, so a future failure starts counting fresh (INV7: consecutive only).
    fn clear_retry(&mut self, id: ComponentId) {
        if let Some(pos) = self.retries.iter().position(|e| e.0 == id) {
            let _ = self.retries.swap_remove(pos);
        }
    }

    /// Advance `cursor` from `start_idx` to the first component not in
    /// `gated`, emitting its `ReadFirmware`/`VerifyFirmware`. Returns `true` if
    /// found. If the rest of the chain is exhausted or entirely gated, sets
    /// `cursor` to a past-the-end sentinel (`chain.len()`) and returns
    /// `false` — the caller should treat that as "chain done".
    fn advance_to_next_ungated(&mut self, ctx: &mut Sink<E>, start_idx: usize) -> bool {
        let mut idx = start_idx;
        while let Some(&(id, _)) = self.chain.get(idx) {
            if !self.is_gated(id) {
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

    /// Gate a component out of service according to its [`FailurePolicy`], and
    /// report the [`Gating`] outcome. This is the **single source of truth**
    /// shared by both paths that take a component out of service — the
    /// runtime-corruption path ([`handle_corruption`](Self::handle_corruption))
    /// and the recovery-exhaustion path — so the two can never disagree about
    /// what a policy means (in particular, `Cascading` cascades in both).
    ///
    /// - `Isolable` → gate this component alone → [`Gating::Gated`].
    /// - `Cascading` → gate this component and every transitive dependent →
    ///   [`Gating::Gated`].
    /// - `Required`, or an unknown/missing id → gate nothing →
    ///   [`Gating::NotGated`], leaving the caller to handle the non-gating
    ///   outcome in its own context (recover, at runtime; lock down, once
    ///   recovery is exhausted).
    ///
    /// Idempotent per component (guarded by `is_gated`).
    fn gate_by_policy(&mut self, ctx: &mut Sink<E>, id: ComponentId) -> Gating {
        match self.attrs_of(id).map(|attrs| attrs.failure_policy) {
            Some(FailurePolicy::Cascading) => {
                self.cascade_hold(ctx, id);
                Gating::Gated
            }
            Some(FailurePolicy::Isolable) => {
                if !self.is_gated(id) {
                    ctx.emit(Effect::AssertReset(id));
                    let _ = self.gated.push(id);
                }
                Gating::Gated
            }
            // `Required`, or an unknown/missing id: not a gating policy.
            _ => Gating::NotGated,
        }
    }

    /// Shared `CorruptionDetected` handling, called from both `PreSupervision`
    /// (directly) and `SupervisingPlatform` (via its superstate handler).
    /// Delegates the policy interpretation to [`gate_by_policy`](Self::gate_by_policy)
    /// so this path and the recovery-exhaustion path can never diverge:
    /// `Isolable`/`Cascading` → gate the component (single or cascade) and stay
    /// put, so a later re-walk skips it instead of silently re-releasing one we
    /// already found corrupt; `Required`/unknown → recover first (the
    /// halt-on-exhaustion decision happens later in `Recovering`).
    fn handle_corruption(&mut self, id: ComponentId, ctx: &mut Sink<E>) -> Outcome<State> {
        match self.gate_by_policy(ctx, id) {
            Gating::Gated => Outcome::Handled,
            Gating::NotGated => {
                self.failed = Some(id);
                Outcome::Transition(State::Recovering)
            }
        }
    }

    /// Hold `root` and cascade-hold every component whose `depends_on`
    /// (transitively) names it. Emits `AssertReset` for each newly gated
    /// component, including `root` itself.
    fn cascade_hold(&mut self, ctx: &mut Sink<E>, root: ComponentId) {
        if !self.is_gated(root) {
            ctx.emit(Effect::AssertReset(root));
            let _ = self.gated.push(root);
        }
        let mut i = 0;
        while let Some(&holder) = self.gated.get(i) {
            i += 1;
            let mut newly_gated: heapless::Vec<ComponentId, N> = heapless::Vec::new();
            for &(id, attrs) in self.chain.iter() {
                if attrs.depends_on == Some(holder) && !self.is_gated(id) {
                    let _ = newly_gated.push(id);
                }
            }
            for id in newly_gated {
                ctx.emit(Effect::AssertReset(id));
                let _ = self.gated.push(id);
            }
        }
    }
}

impl<const N: usize, const E: usize> IntoStateMachine for Rot<N, E> {
    type Event<'evt> = Event;
    type Context<'ctx> = Sink<E>;
    type State = State;
    type Superstate<'sub> = Superstate<'sub>;

    fn initial() -> State {
        State::PowerOnReset
    }
}

impl<const N: usize, const E: usize> StatigState<Rot<N, E>> for State {
    fn call_handler(&mut self, rot: &mut Rot<N, E>, event: &Event, ctx: &mut Sink<E>) -> Outcome<State> {
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
                    // The component passed its check — it has recovered, so its
                    // consecutive-failure streak ends (INV7: consecutive only).
                    rot.clear_retry(*id);
                    ctx.emit(Effect::ReleaseReset(*id));
                    let current_kind = rot.chain.get(rot.cursor as usize).map(|(_, a)| a.kind);
                    let next_idx = (rot.cursor as usize).saturating_add(1);
                    if rot.advance_to_next_ungated(ctx, next_idx) {
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
                // Minimal fix: react to a corruption report if one arrives,
                // even though `PreSupervision` isn't linked to
                // `SupervisingPlatform`. Not acting on data we already have
                // would be strictly worse than acting on it, regardless of
                // whether CSA defines a mechanism that guarantees this event
                // exists in the first place. `AttestationChallenge` is left
                // unhandled here (falls through to `Outcome::Super` and is
                // discarded) — that's a separate question.
                Event::CorruptionDetected(id) => rot.handle_corruption(*id, ctx),
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
                    // The component passed its check — it has recovered, so its
                    // consecutive-failure streak ends (INV7: consecutive only).
                    rot.clear_retry(*id);
                    ctx.emit(Effect::ReleaseReset(*id));
                    let next_idx = (rot.cursor as usize).saturating_add(1);
                    if rot.advance_to_next_ungated(ctx, next_idx) {
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
                    // Count this attempt against the specific component in
                    // recovery, not a global budget (CSA: exhaustion is
                    // per-device). `failed` is always `Some` while in
                    // `Recovering`; treat a missing id as exhausted defensively.
                    let attempts = rot.failed.map(|id| rot.bump_retry(id)).unwrap_or(rot.max_retry);
                    if attempts < rot.max_retry {
                        Outcome::Transition(State::PreSupervision)
                    } else {
                        // Retries exhausted: gate via the same `gate_by_policy`
                        // the runtime-corruption path uses, so the two can never
                        // disagree. Gated → continue the walk; NotGated
                        // (Required/unknown) → lock down.
                        match rot.failed.map(|id| (id, rot.gate_by_policy(ctx, id))) {
                            Some((id, Gating::Gated)) => {
                                rot.clear_retry(id);
                                rot.failed = None;
                                Outcome::Transition(State::PreSupervision)
                            }
                            // `Required`, or an unknown/missing id: lock down.
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

    fn call_entry_action(&mut self, rot: &mut Rot<N, E>, ctx: &mut Sink<E>) {
        match self {
            State::PreSupervision => {
                rot.awaiting = None;
                let _ = rot.advance_to_next_ungated(ctx, 0);
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
                // NB: `gated` is intentionally NOT cleared here. It is a durable
                // trust-boundary gate (each id has a live `AssertReset`);
                // clearing it would let a later chain walk re-release a
                // component that policy deliberately isolated. Only a fresh
                // `Rot` on `PowerOnReset` clears it.
                //
                // Retry counts, by contrast, ARE cleared: a clean boot means no
                // recovery episode is in flight, so every per-component streak
                // resets to zero.
                rot.retries.clear();
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

impl<const N: usize, const E: usize> StatigSuperstate<Rot<N, E>> for Superstate<'_> {
    fn call_handler(&mut self, rot: &mut Rot<N, E>, event: &Event, ctx: &mut Sink<E>) -> Outcome<State> {
        match self {
            Superstate::SupervisingPlatform(_) => match event {
                Event::AttestationChallenge => {
                    ctx.emit(Effect::SignAttestation);
                    Outcome::Handled
                }
                Event::CorruptionDetected(id) => rot.handle_corruption(*id, ctx),
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
pub struct Orchestrator<const N: usize, const E: usize> {
    machine: StateMachine<Rot<N, E>>,
}

impl<const N: usize, const E: usize> Orchestrator<N, E> {
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

            let mut buf = Sink::<E>::new();
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
        let mut orch = Orchestrator::<CAPACITY, ECAP>::new(chain, MAX_RETRY);
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
        assert_eq!(state, State::Recovering);
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
        let mut orch = Orchestrator::<CAPACITY, ECAP>::new(c, 2);
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

    /// Retry budgets are **per component**, not a single global counter. Two
    /// required components each fail exactly once (well under `max_retry = 2`)
    /// in an interleaved recovery sequence before the chain finally settles.
    /// Under a shared global counter the second failure would push the count to
    /// the cap and latch the platform to `Locked`; per-component counting lets
    /// each device use its own budget, so the walk reaches `Ready`.
    #[test]
    fn retry_budget_is_per_component() {
        let mut c = heapless::Vec::<(ComponentId, ComponentAttrs), CAPACITY>::new();
        c.push((C0, ComponentAttrs::passive_required())).expect("fits");
        c.push((C1, ComponentAttrs::passive_required())).expect("fits");
        let mut orch = Orchestrator::<CAPACITY, ECAP>::new(c, 2);
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
            orch.dispatch_with(ev, |e| effects.push(e));
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
        let mut orch = Orchestrator::<CAPACITY, ECAP>::new(c, 1);
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
        let mut orch = Orchestrator::<3, 5>::new(c, MAX_RETRY);
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
        assert_eq!(state, State::Recovering);
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
        let mut orch = Orchestrator::<CAPACITY, ECAP>::new(c, 1);
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
        let mut orch = Orchestrator::<CAPACITY, ECAP>::new(
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
