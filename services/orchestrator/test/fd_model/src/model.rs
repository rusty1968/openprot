// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! The state machine.

use crate::types::{
    Cause, Code, FdConfig, IdleReason, Op, OpResult, PhaseId, Refusal, Status, StatusReply, UaKind,
    UaMessage,
};

/// Messages to the Update Agent held for [`FdModel::take_ua_message`]. A caller
/// drains them every wake; more are counted, not kept.
const QUEUE: usize = 8;
// Queue indices are taken modulo `QUEUE`, so it must not be zero.
const _: () = assert!(QUEUE > 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    ReadyXfer,
    OfferPending,
    Transferring,
    VerifyPending,
    Verifying,
    ApplyPending,
    Applying,
    ActivationPending,
    Activating,
    SvnCommitPending,
    Failed,
    Cancelled,
}

/// The activation verdict the FD stores for the Update Agent's request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    None,
    Perform,
    Deny,
}

/// Which cancel the Update Agent sent. It decides where `AckCancel` leaves the
/// FD, and no status field records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancelKind {
    Update,
    Component,
}

/// The firmware device, as the orchestrator sees it over IPC.
///
/// Inputs are the Update Agent's requests (`ua_*`), the orchestrator's
/// operations ([`apply`](Self::apply)) and time. Every call takes `now_millis`
/// and first applies whatever fell due, stamping each change with the moment it
/// was due. Nothing here waits.
pub struct FdModel {
    cfg: FdConfig,
    phase: Phase,
    idle_reason: IdleReason,
    target: u16,
    total: u32,
    component: u16,
    base: Option<u32>,
    failed: Option<(PhaseId, Code)>,
    /// Apply has succeeded, so the update can no longer be cancelled.
    applied: bool,
    order: Order,
    cancel_kind: CancelKind,
    /// When the next internal change is due: a running phase finishing, or the
    /// orchestrator's window running out.
    deadline: Option<u64>,
    /// The last message to or from the Update Agent, which resets the idle timer.
    ua_last: u64,
    nudge: bool,
    nudges_raised: u32,
    nudges_merged: u32,
    generation: u16,
    queue: [Option<UaMessage>; QUEUE],
    head: usize,
    len: usize,
    dropped: u32,
}

impl FdModel {
    /// A fresh FD: idle, with no reason, and no nudge raised.
    pub const fn new(cfg: FdConfig) -> Self {
        Self {
            cfg,
            phase: Phase::Idle,
            idle_reason: IdleReason::Fresh,
            target: 0,
            total: 0,
            component: 0,
            base: None,
            failed: None,
            applied: false,
            order: Order::None,
            cancel_kind: CancelKind::Update,
            deadline: None,
            ua_last: 0,
            nudge: false,
            nudges_raised: 0,
            nudges_merged: 0,
            generation: 0,
            queue: [None; QUEUE],
            head: 0,
            len: 0,
            dropped: 0,
        }
    }

    /// What a query would answer, without the side effects of asking.
    pub fn status(&self) -> Status {
        match self.phase {
            Phase::Idle => Status::Idle {
                reason: self.idle_reason,
            },
            Phase::ReadyXfer => Status::ReadyXfer,
            Phase::OfferPending => Status::OfferPending {
                target: self.target,
                total: self.total,
            },
            Phase::Transferring => Status::InFlight(PhaseId::Transfer),
            Phase::Verifying => Status::InFlight(PhaseId::Verify),
            Phase::Applying => Status::InFlight(PhaseId::Apply),
            Phase::Activating => Status::InFlight(PhaseId::Activate),
            Phase::VerifyPending => Status::VerifyPending,
            Phase::ApplyPending => Status::ApplyPending,
            Phase::ActivationPending => Status::ActivationPending,
            Phase::SvnCommitPending => Status::SvnCommitPending {
                component: self.component,
            },
            Phase::Failed => match self.failed {
                Some((phase, code)) => Status::PhaseFailed { phase, code },
                None => Status::PhaseFailed {
                    phase: PhaseId::Verify,
                    code: Code::NotReady,
                },
            },
            Phase::Cancelled => Status::Cancelled,
        }
    }

    /// Whether the nudge is raised. It is a level: it stays up until a query is
    /// answered, however many changes happen meanwhile.
    pub fn nudge(&self) -> bool {
        self.nudge
    }

    /// How many times the FD raised the nudge, counting raises that found it
    /// already up.
    pub fn nudges_raised(&self) -> u32 {
        self.nudges_raised
    }

    /// How many of those raises found the nudge already up. Each one is a state
    /// the orchestrator may never see, because one query answers them all.
    pub fn nudges_merged(&self) -> u32 {
        self.nudges_merged
    }

    /// The current phase generation.
    pub fn generation(&self) -> u16 {
        self.generation
    }

    /// Where the orchestrator told the FD to stage the image.
    pub fn staging_base(&self) -> Option<u32> {
        self.base
    }

    /// Messages to the Update Agent lost because the queue was full.
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// When the next change is due, so an event loop can sleep until then.
    pub fn next_event_at(&self) -> Option<u64> {
        match (self.deadline, self.t1_at()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The oldest message to the Update Agent not yet taken, if any.
    pub fn take_ua_message(&mut self) -> Option<UaMessage> {
        if self.len == 0 {
            return None;
        }
        let taken = self.queue.get_mut(self.head).and_then(Option::take);
        self.head = self.head.saturating_add(1) % QUEUE;
        self.len = self.len.saturating_sub(1);
        taken
    }

    /// Apply whatever fell due by `now_millis`, in order, each stamped with the
    /// moment it was due.
    pub fn poll(&mut self, now_millis: u64) {
        while let Some(at) = self.deadline {
            if now_millis < at {
                break;
            }
            self.fire(at);
        }
        if let Some(at) = self.t1_at()
            && now_millis >= at
        {
            self.go_idle(IdleReason::T1Timeout);
            self.raise_nudge();
        }
    }

    /// Answer a status query. The nudge is cleared first and the state read
    /// after, as the design says.
    pub fn query_status(&mut self, now_millis: u64) -> StatusReply {
        self.poll(now_millis);
        self.nudge = false;
        StatusReply {
            status: self.status(),
            generation: self.generation,
        }
    }

    /// Apply an order or refusal from the orchestrator. `generation`, when given,
    /// must be the current one.
    ///
    /// A change the orchestrator itself causes does not raise the nudge: it
    /// already knows.
    pub fn apply(&mut self, op: Op, generation: Option<u16>, now_millis: u64) -> OpResult {
        self.poll(now_millis);
        if let Some(g) = generation
            && g != self.generation
        {
            return Err(Refusal::StaleGeneration);
        }
        let now = now_millis;
        match (op, self.phase) {
            (Op::AcceptOffer { base }, Phase::OfferPending) => {
                self.base = Some(base);
                self.phase = Phase::Transferring;
                self.deadline = Some(now.saturating_add(self.cfg.transfer_ms));
                Ok(())
            }
            (Op::RejectOffer(_), Phase::OfferPending) => {
                self.send(now, UaKind::TransferComplete, Code::FdAbortedTransfer);
                self.go_idle(IdleReason::OfferRejected);
                Ok(())
            }
            (Op::PerformVerify, Phase::VerifyPending) => {
                self.phase = Phase::Verifying;
                self.deadline = Some(now.saturating_add(self.cfg.verify_ms));
                Ok(())
            }
            (Op::RejectVerify(r), Phase::VerifyPending) => {
                self.fail_phase(now, PhaseId::Verify, Cause::Rejected(r), false);
                Ok(())
            }
            (Op::PerformApply, Phase::ApplyPending) => {
                self.phase = Phase::Applying;
                self.deadline = Some(now.saturating_add(self.cfg.apply_ms));
                Ok(())
            }
            (Op::RejectApply(r), Phase::ApplyPending) => {
                self.fail_phase(now, PhaseId::Apply, Cause::Rejected(r), false);
                Ok(())
            }
            (Op::PerformActivate, Phase::ActivationPending) => {
                self.order = Order::Perform;
                self.phase = Phase::ReadyXfer;
                self.deadline = None;
                Ok(())
            }
            (Op::RejectActivate(_), Phase::ActivationPending) => {
                self.order = Order::Deny;
                self.phase = Phase::ReadyXfer;
                self.deadline = None;
                Ok(())
            }
            // A stored order is revocable until the Update Agent asks for it.
            (Op::RejectActivate(_), Phase::ReadyXfer)
                if self.applied && self.order == Order::Perform =>
            {
                self.order = Order::Deny;
                Ok(())
            }
            // The request arrived: the FD has answered and activation is under
            // way, so a refusal racing it loses.
            (Op::RejectActivate(_), Phase::Activating) => Err(Refusal::TooLate),
            (Op::AckCancel, Phase::Cancelled) => {
                match self.cancel_kind {
                    CancelKind::Update => self.go_idle(IdleReason::Cancelled),
                    CancelKind::Component => {
                        self.phase = Phase::ReadyXfer;
                        self.failed = None;
                        self.applied = false;
                        self.order = Order::None;
                        self.base = None;
                    }
                }
                Ok(())
            }
            (Op::PerformSvnCommit, Phase::SvnCommitPending) => {
                self.send(now, UaKind::SecurityRevisionResponse, Code::Success);
                self.phase = Phase::Idle;
                self.deadline = None;
                Ok(())
            }
            (Op::RejectSvnCommit(_), Phase::SvnCommitPending) => {
                self.send(now, UaKind::SecurityRevisionResponse, Code::NotPermitted);
                self.phase = Phase::Idle;
                self.deadline = None;
                Ok(())
            }
            _ => Err(Refusal::WrongPhase),
        }
    }

    /// The Update Agent offers a component (`RequestUpdate`, `PassComponentTable`
    /// and `UpdateComponent` folded into one). `Err` is what the Update Agent is
    /// told when the FD cannot take it.
    pub fn ua_offer(&mut self, target: u16, total: u32, now_millis: u64) -> Result<(), Code> {
        self.poll(now_millis);
        let open = self.phase == Phase::Idle || (self.phase == Phase::ReadyXfer && !self.applied);
        if !open {
            return Err(Code::AlreadyInUpdateMode);
        }
        self.ua_last = now_millis;
        self.target = target;
        self.total = total;
        self.applied = false;
        self.order = Order::None;
        self.failed = None;
        self.base = None;
        self.enter_pending(Phase::OfferPending, now_millis);
        Ok(())
    }

    /// The Update Agent asks for activation. The answer comes from the stored
    /// order: an order succeeds and starts activation, a denial or no order is
    /// `INCOMPLETE_UPDATE`, which leaves the FD in `READY XFER`.
    pub fn ua_activate(&mut self, now_millis: u64) -> Code {
        self.poll(now_millis);
        let code = match self.phase {
            Phase::ReadyXfer if self.applied => match self.order {
                Order::Perform => {
                    self.phase = Phase::Activating;
                    self.deadline = Some(now_millis.saturating_add(self.cfg.activate_ms));
                    Code::Success
                }
                Order::Deny | Order::None => Code::IncompleteUpdate,
            },
            // The Update Agent asked before the orchestrator decided.
            Phase::ActivationPending => Code::IncompleteUpdate,
            _ => Code::NotReady,
        };
        self.send(now_millis, UaKind::ActivateResponse, code);
        code
    }

    /// The Update Agent cancels the whole update (`CancelUpdate`). Answered at
    /// once, from the FD's own state.
    pub fn ua_cancel_update(&mut self, now_millis: u64) -> Code {
        self.ua_cancel(CancelKind::Update, now_millis)
    }

    /// The Update Agent cancels one component (`CancelUpdateComponent`).
    pub fn ua_cancel_update_component(&mut self, now_millis: u64) -> Code {
        self.ua_cancel(CancelKind::Component, now_millis)
    }

    /// The Update Agent asks to commit the security revision
    /// (`UpdateSecurityRevision`), which the FD accepts only when idle.
    pub fn ua_security_revision(&mut self, component: u16, now_millis: u64) -> Result<(), Code> {
        self.poll(now_millis);
        if self.phase != Phase::Idle {
            self.send(
                now_millis,
                UaKind::SecurityRevisionResponse,
                Code::NotPermitted,
            );
            return Err(Code::NotPermitted);
        }
        self.ua_last = now_millis;
        self.component = component;
        self.enter_pending(Phase::SvnCommitPending, now_millis);
        Ok(())
    }

    fn ua_cancel(&mut self, kind: CancelKind, now: u64) -> Code {
        self.poll(now);
        let code = match self.phase {
            Phase::Idle | Phase::SvnCommitPending => Code::NotInUpdateMode,
            // Once apply has succeeded the FD refuses the cancel outright.
            Phase::ActivationPending | Phase::Activating => Code::CancelRefused,
            Phase::ReadyXfer if self.applied => Code::CancelRefused,
            Phase::Cancelled => Code::Success,
            _ => {
                self.order = Order::None;
                self.failed = None;
                self.deadline = None;
                self.phase = Phase::Cancelled;
                self.cancel_kind = kind;
                self.raise_nudge();
                Code::Success
            }
        };
        self.send(now, UaKind::CancelResponse, code);
        code
    }

    /// When the idle timer runs out, while the FD waits on the Update Agent.
    fn t1_at(&self) -> Option<u64> {
        match self.phase {
            Phase::ReadyXfer | Phase::Failed => Some(self.ua_last.saturating_add(self.cfg.t1_ms)),
            _ => None,
        }
    }

    /// Something internal fell due at `at`.
    fn fire(&mut self, at: u64) {
        self.deadline = None;
        match self.phase {
            Phase::Transferring => {
                self.send(at, UaKind::TransferComplete, Code::Success);
                self.enter_pending(Phase::VerifyPending, at);
            }
            Phase::Verifying => {
                if self.cfg.verify_passes {
                    self.send(at, UaKind::VerifyComplete, Code::Success);
                    self.enter_pending(Phase::ApplyPending, at);
                } else {
                    self.fail_phase(at, PhaseId::Verify, Cause::VerifyFailed, true);
                }
            }
            Phase::Applying => {
                if self.cfg.apply_passes {
                    self.applied = true;
                    self.send(at, UaKind::ApplyComplete, Code::Success);
                    self.enter_pending(Phase::ActivationPending, at);
                } else {
                    self.fail_phase(at, PhaseId::Apply, Cause::ApplyFailed, true);
                }
            }
            Phase::Activating => {
                self.go_idle(IdleReason::ActivateFw);
                self.raise_nudge();
            }
            // The orchestrator's window ran out: silence is a refusal.
            Phase::OfferPending => {
                self.send(at, UaKind::TransferComplete, Code::FdAbortedTransfer);
                self.go_idle(IdleReason::OrderTimeout);
                self.raise_nudge();
            }
            Phase::VerifyPending => self.fail_phase(at, PhaseId::Verify, Cause::OrderTimeout, true),
            Phase::ApplyPending => self.fail_phase(at, PhaseId::Apply, Cause::OrderTimeout, true),
            Phase::ActivationPending => {
                self.order = Order::Deny;
                self.phase = Phase::ReadyXfer;
                self.raise_nudge();
            }
            Phase::SvnCommitPending => {
                self.send(at, UaKind::SecurityRevisionResponse, Code::NotPermitted);
                self.phase = Phase::Idle;
                self.raise_nudge();
            }
            Phase::Idle | Phase::ReadyXfer | Phase::Failed | Phase::Cancelled => {}
        }
    }

    /// Enter a phase that waits on the orchestrator: advance the generation, start
    /// the FD's own window, and raise the nudge.
    fn enter_pending(&mut self, phase: Phase, at: u64) {
        self.phase = phase;
        self.generation = self.generation.wrapping_add(1);
        self.deadline = Some(at.saturating_add(self.cfg.order_window_ms));
        self.raise_nudge();
    }

    /// Fail a verify or apply phase: the Update Agent is told, and the FD stays
    /// in the failed phase until the Update Agent cancels.
    fn fail_phase(&mut self, at: u64, phase: PhaseId, cause: Cause, nudge: bool) {
        let kind = match phase {
            PhaseId::Apply => UaKind::ApplyComplete,
            _ => UaKind::VerifyComplete,
        };
        let code = Code::Failed(cause);
        self.send(at, kind, code);
        self.failed = Some((phase, code));
        self.phase = Phase::Failed;
        self.deadline = None;
        if nudge {
            self.raise_nudge();
        }
    }

    fn go_idle(&mut self, reason: IdleReason) {
        self.phase = Phase::Idle;
        self.idle_reason = reason;
        self.failed = None;
        self.applied = false;
        self.order = Order::None;
        self.base = None;
        self.deadline = None;
    }

    fn raise_nudge(&mut self) {
        if self.nudge {
            self.nudges_merged = self.nudges_merged.saturating_add(1);
        }
        self.nudge = true;
        self.nudges_raised = self.nudges_raised.saturating_add(1);
    }

    /// Record a message to the Update Agent. Any message resets the idle timer.
    fn send(&mut self, at: u64, kind: UaKind, code: Code) {
        self.ua_last = at;
        if self.len == QUEUE {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        let slot = self.head.saturating_add(self.len) % QUEUE;
        if let Some(s) = self.queue.get_mut(slot) {
            *s = Some(UaMessage {
                at_millis: at,
                kind,
                code,
            });
            self.len = self.len.saturating_add(1);
        }
    }
}
