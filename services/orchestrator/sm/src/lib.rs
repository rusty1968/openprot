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

use statig::Outcome;
use statig::blocking::{
    IntoStateMachine, IntoStateMachineExt as _, State as StatigState, StateMachine,
    Superstate as StatigSuperstate,
};

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

/// A validated **chain of trust**: the ordered list of components the eRoT
/// walks, verifies, and supervises, in walk order.
///
/// Build one with [`TryFrom`]/[`TryInto`] from a `heapless::Vec` of
/// `(ComponentId, ComponentAttrs)` pairs. The conversion is the single place
/// the reducer's structural invariants are enforced, so a malformed chain
/// fails closed at the boundary instead of misbehaving later:
///
/// - the chain is non-empty,
/// - every [`ComponentId`] is unique,
/// - every `depends_on` names a component that exists and appears *strictly
///   earlier* in the chain (no dangling, forward, or self dependencies — a
///   dependency is always walked before its dependents),
/// - the length fits `u8`, the `cursor` index type.
///
/// ```ignore
/// let mut v = heapless::Vec::<_, 4>::new();
/// v.push((ComponentId::new(0), ComponentAttrs::passive_required())).unwrap();
/// let chain: Chain<4> = v.try_into()?;
/// ```
#[derive(Clone, Debug)]
pub struct Chain<const N: usize> {
    entries: heapless::Vec<(ComponentId, ComponentAttrs), N>,
}

/// Why a `heapless::Vec` of components is not a valid [`Chain`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChainError {
    /// The chain has no components.
    Empty,
    /// The chain is longer than `u8::MAX`, so `cursor` could not index it.
    TooLong,
    /// The same [`ComponentId`] appears more than once.
    DuplicateId(ComponentId),
    /// A `depends_on` names an id that is not in the chain.
    UnknownDependency {
        component: ComponentId,
        depends_on: ComponentId,
    },
    /// A `depends_on` names a component that does not appear strictly earlier
    /// in the chain (a forward reference or a self-reference). A dependency
    /// must be walked before its dependents.
    ForwardDependency {
        component: ComponentId,
        depends_on: ComponentId,
    },
}

impl<const N: usize> Chain<N> {
    /// Consume the validated chain, yielding its components in walk order.
    fn into_entries(self) -> heapless::Vec<(ComponentId, ComponentAttrs), N> {
        self.entries
    }
}

impl<const N: usize> TryFrom<heapless::Vec<(ComponentId, ComponentAttrs), N>> for Chain<N> {
    type Error = ChainError;

    fn try_from(
        entries: heapless::Vec<(ComponentId, ComponentAttrs), N>,
    ) -> Result<Self, ChainError> {
        if entries.is_empty() {
            return Err(ChainError::Empty);
        }
        if entries.len() > u8::MAX as usize {
            return Err(ChainError::TooLong);
        }
        for (i, (id, _)) in entries.iter().enumerate() {
            if entries[..i].iter().any(|(prev, _)| prev == id) {
                return Err(ChainError::DuplicateId(*id));
            }
        }
        for (i, (id, attrs)) in entries.iter().enumerate() {
            if let Some(dep) = attrs.depends_on {
                match entries.iter().position(|(cid, _)| *cid == dep) {
                    None => {
                        return Err(ChainError::UnknownDependency {
                            component: *id,
                            depends_on: dep,
                        });
                    }
                    Some(j) if j >= i => {
                        return Err(ChainError::ForwardDependency {
                            component: *id,
                            depends_on: dep,
                        });
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(Self { entries })
    }
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
    const EFFECT_CAP_OK: () = assert!(E >= N + 2, "effect buffer E must be >= chain length N + 2");

    pub fn new(chain: Chain<N>, max_retry: u8) -> Self {
        let () = Self::EFFECT_CAP_OK;
        Self {
            chain: chain.into_entries(),
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
        self.chain
            .iter()
            .find(|(cid, _)| *cid == id)
            .map(|(_, a)| *a)
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
    fn call_handler(
        &mut self,
        rot: &mut Rot<N, E>,
        event: &Event,
        ctx: &mut Sink<E>,
    ) -> Outcome<State> {
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
                    let attempts = rot
                        .failed
                        .map(|id| rot.bump_retry(id))
                        .unwrap_or(rot.max_retry);
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
    fn call_handler(
        &mut self,
        rot: &mut Rot<N, E>,
        event: &Event,
        ctx: &mut Sink<E>,
    ) -> Outcome<State> {
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
    pub fn new(chain: Chain<N>, max_retry: u8) -> Self {
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
mod tests;
