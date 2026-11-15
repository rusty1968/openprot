// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! The vocabulary of the design: the orchestrator's operations, what a status
//! query answers, and what the Update Agent is told. Names follow the design
//! doc; the numeric wire values are not modelled.

/// Why the orchestrator refuses a phase. It rides on a `Reject*` operation, and
/// the FD maps it onto a completion code for the Update Agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The target component is isolated.
    Isolated,
    /// Update policy forbids it.
    PolicyViolation,
    /// The offer names a target the platform does not have.
    UnknownTarget,
    /// The platform is busy with something else.
    Busy,
}

/// An order or a refusal from the orchestrator to the FD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Accept the offer and stage the image at `base`.
    AcceptOffer { base: u32 },
    /// Refuse the offer.
    RejectOffer(Reason),
    /// Order the verify phase.
    PerformVerify,
    /// Refuse the verify phase.
    RejectVerify(Reason),
    /// Order the apply phase.
    PerformApply,
    /// Refuse the apply phase.
    RejectApply(Reason),
    /// Order activation ahead of the Update Agent's request. The FD stores it.
    PerformActivate,
    /// Refuse activation, or revoke a stored order.
    RejectActivate(Reason),
    /// Acknowledge a cancel. Records it orchestrator-side and orders nothing.
    AckCancel,
    /// Tell the FD the security revision floor is raised.
    PerformSvnCommit,
    /// Refuse the security revision commit.
    RejectSvnCommit(Reason),
}

/// Why the FD turned an operation away, as opposed to the orchestrator's own
/// policy refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The FD is not in the phase this operation belongs to.
    WrongPhase,
    /// The Update Agent already asked for activation, so a refusal now loses.
    TooLate,
    /// The operation carried a generation other than the current one.
    StaleGeneration,
}

/// The result of an operation.
pub type OpResult = Result<(), Refusal>;

/// Why the FD went idle. Names only: the DSP0267 reason-code values are not
/// modelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleReason {
    /// Nothing has happened since the FD started. This is how a restarted FD is
    /// told apart from one that finished.
    Fresh,
    /// An activation completed.
    ActivateFw,
    /// The Update Agent cancelled the whole update.
    Cancelled,
    /// The orchestrator refused the offer.
    OfferRejected,
    /// The orchestrator never answered within the FD's window.
    OrderTimeout,
    /// The Update Agent went quiet for the FD's idle timer.
    T1Timeout,
}

/// A phase that runs or can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseId {
    Transfer,
    Verify,
    Apply,
    Activate,
}

/// What failed a phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// The orchestrator refused it.
    Rejected(Reason),
    /// The orchestrator did not answer in time.
    OrderTimeout,
    /// The check itself failed.
    VerifyFailed,
    /// The apply itself failed.
    ApplyFailed,
}

/// The completion code the Update Agent is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Success,
    /// A transfer the FD gave up on (the design's `FdAbortedTransfer`).
    FdAbortedTransfer,
    /// A phase failed.
    Failed(Cause),
    /// Activation was denied (the design's `INCOMPLETE_UPDATE`).
    IncompleteUpdate,
    /// The security revision commit was refused.
    NotPermitted,
    /// A cancel arrived while the FD was not in update mode.
    NotInUpdateMode,
    /// A request arrived while an update was already in progress.
    AlreadyInUpdateMode,
    /// A cancel arrived after apply succeeded, which the FD refuses outright.
    CancelRefused,
    /// A request arrived in a state that cannot serve it.
    NotReady,
}

/// What a status query answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No update in progress, and why.
    Idle { reason: IdleReason },
    /// The spec's `READY XFER`: an update is open and the FD waits for the
    /// Update Agent.
    ReadyXfer,
    /// The orchestrator owes an answer to an offer.
    OfferPending { target: u16, total: u32 },
    /// A phase is running. **Not in the design's list of conditions**: a query
    /// between nudges needs an answer, and this is the assumption made here.
    InFlight(PhaseId),
    /// The orchestrator owes the verify order.
    VerifyPending,
    /// The orchestrator owes the apply order.
    ApplyPending,
    /// The orchestrator owes the activation decision.
    ActivationPending,
    /// The orchestrator owes the answer to a security revision request.
    SvnCommitPending { component: u16 },
    /// A verify or apply failed and the FD waits for the Update Agent to cancel.
    PhaseFailed { phase: PhaseId, code: Code },
    /// The Update Agent cancelled; the FD waits for `AckCancel`.
    Cancelled,
}

/// A status answer, with the phase generation it describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusReply {
    pub status: Status,
    /// Advances each time the FD enters a phase that waits on the orchestrator.
    /// An operation may name it so a stale one can be told from a current one.
    pub generation: u16,
}

/// Which message to the Update Agent a [`UaMessage`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UaKind {
    TransferComplete,
    VerifyComplete,
    ApplyComplete,
    ActivateResponse,
    CancelResponse,
    SecurityRevisionResponse,
}

/// A message the FD sent the Update Agent, stamped with when it was due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UaMessage {
    pub at_millis: u64,
    pub kind: UaKind,
    pub code: Code,
}

/// How long each part of an update takes, and what each phase's own outcome
/// is. All times are in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdConfig {
    /// How long the FD takes to pull the image from the Update Agent.
    pub transfer_ms: u64,
    /// How long the verify phase runs once ordered.
    pub verify_ms: u64,
    /// How long the apply phase runs once ordered.
    pub apply_ms: u64,
    /// How long activation runs once the Update Agent asks for it.
    pub activate_ms: u64,
    /// How long the FD waits for the orchestrator before it treats silence as a
    /// refusal. This is the design's "deadline the FD keeps itself".
    pub order_window_ms: u64,
    /// How long the FD waits on a quiet Update Agent before it cancels itself.
    pub t1_ms: u64,
    /// Whether the verify phase succeeds when it runs.
    pub verify_passes: bool,
    /// Whether the apply phase succeeds when it runs.
    pub apply_passes: bool,
}

impl FdConfig {
    /// A healthy FD: every phase succeeds, with generous windows.
    pub const DEFAULT: Self = Self {
        transfer_ms: 100,
        verify_ms: 50,
        apply_ms: 50,
        activate_ms: 20,
        order_window_ms: 1_000,
        t1_ms: 5_000,
        verify_passes: true,
        apply_passes: true,
    };
}
