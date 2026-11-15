// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Each test names the sentence of the design it pins.

use crate::*;

const CFG: FdConfig = FdConfig::DEFAULT;

fn fd() -> FdModel {
    FdModel::new(CFG)
}

fn msg(at_millis: u64, kind: UaKind, code: Code) -> UaMessage {
    UaMessage {
        at_millis,
        kind,
        code,
    }
}

/// Offer, accept, and let the transfer finish: the FD parks at `VerifyPending`.
fn to_verify_pending(f: &mut FdModel) {
    assert_eq!(f.ua_offer(1, 4096, 0), Ok(()));
    f.query_status(0);
    assert_eq!(f.apply(Op::AcceptOffer { base: 0x1000 }, None, 0), Ok(()));
    f.poll(CFG.transfer_ms);
    assert_eq!(f.status(), Status::VerifyPending);
}

/// Carry on through verify and apply: the FD parks at `ActivationPending`.
fn to_activation_pending(f: &mut FdModel) {
    to_verify_pending(f);
    let t = CFG.transfer_ms;
    assert_eq!(f.apply(Op::PerformVerify, None, t), Ok(()));
    f.poll(t + CFG.verify_ms);
    assert_eq!(f.status(), Status::ApplyPending);
    let t = t + CFG.verify_ms;
    assert_eq!(f.apply(Op::PerformApply, None, t), Ok(()));
    f.poll(t + CFG.apply_ms);
    assert_eq!(f.status(), Status::ActivationPending);
}

const T_ACTIVATION: u64 = CFG.transfer_ms + CFG.verify_ms + CFG.apply_ms;

fn drain(f: &mut FdModel) -> usize {
    let mut n = 0;
    while f.take_ua_message().is_some() {
        n += 1;
    }
    n
}

// "nothing at all on a fresh start, which is how a restarted FD is told apart
// from one that finished"
#[test]
fn a_fresh_fd_is_idle_with_no_reason() {
    let mut f = fd();
    assert_eq!(
        f.status(),
        Status::Idle {
            reason: IdleReason::Fresh
        }
    );
    assert!(!f.nudge());
    assert_eq!(f.take_ua_message(), None);
}

// "Offer ready (UA sent UpdateComponent, FD has target + total)" raises a nudge.
#[test]
fn an_offer_raises_the_nudge_and_reports_offer_pending() {
    let mut f = fd();
    assert_eq!(f.ua_offer(7, 4096, 10), Ok(()));
    assert!(f.nudge());
    assert_eq!(
        f.status(),
        Status::OfferPending {
            target: 7,
            total: 4096
        }
    );
}

// "The FD clears the signal when it answers QueryStatus, and raises it again on
// the next state change." The nudge is a level.
#[test]
fn a_query_clears_the_nudge_and_reads_the_state_after() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();
    assert!(f.nudge());

    let reply = f.query_status(0);

    assert!(!f.nudge());
    assert_eq!(
        reply.status,
        Status::OfferPending {
            target: 1,
            total: 100
        }
    );
}

// "pldm-lib ... the FD answers ALREADY_IN_UPDATE_MODE and the UA retries."
#[test]
fn an_offer_during_an_update_is_already_in_update_mode() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();
    assert_eq!(f.ua_offer(2, 100, 1), Err(Code::AlreadyInUpdateMode));
}

// Operations belong to one phase each; the FD turns the rest away.
#[test]
fn an_operation_in_the_wrong_phase_is_refused_and_changes_nothing() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();
    let before = f.status();

    assert_eq!(
        f.apply(Op::PerformVerify, None, 0),
        Err(Refusal::WrongPhase)
    );
    assert_eq!(
        f.apply(Op::PerformActivate, None, 0),
        Err(Refusal::WrongPhase)
    );
    assert_eq!(f.apply(Op::AckCancel, None, 0), Err(Refusal::WrongPhase));

    assert_eq!(f.status(), before);
}

// The transfer is "zero-IPC": the FD pulls the image, then nudges for verify.
#[test]
fn accepting_the_offer_runs_the_transfer_then_parks_for_the_verify_order() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();
    f.query_status(0);
    f.apply(Op::AcceptOffer { base: 0x2000 }, None, 0).unwrap();
    assert_eq!(f.status(), Status::InFlight(PhaseId::Transfer));
    assert_eq!(f.staging_base(), Some(0x2000));
    assert!(!f.nudge(), "the orchestrator caused this change");

    f.poll(CFG.transfer_ms);

    assert_eq!(f.status(), Status::VerifyPending);
    assert!(f.nudge());
    assert_eq!(
        f.take_ua_message(),
        Some(msg(
            CFG.transfer_ms,
            UaKind::TransferComplete,
            Code::Success
        ))
    );
}

// "RejectOffer ... FD tells UA"; "A rejected offer ends with TransferComplete.
// The FD sends TransferComplete with FdAbortedTransfer".
#[test]
fn a_rejected_offer_ends_with_a_transfer_complete_abort() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();

    f.apply(Op::RejectOffer(Reason::Busy), None, 5).unwrap();

    assert_eq!(
        f.take_ua_message(),
        Some(msg(5, UaKind::TransferComplete, Code::FdAbortedTransfer))
    );
    assert_eq!(
        f.status(),
        Status::Idle {
            reason: IdleReason::OfferRejected
        }
    );
}

#[test]
fn verify_then_apply_each_park_for_their_order_and_tell_the_ua() {
    let mut f = fd();
    to_activation_pending(&mut f);

    let mut kinds = [UaKind::TransferComplete; 3];
    for slot in kinds.iter_mut() {
        *slot = f.take_ua_message().unwrap().kind;
    }
    assert_eq!(
        kinds,
        [
            UaKind::TransferComplete,
            UaKind::VerifyComplete,
            UaKind::ApplyComplete
        ]
    );
    assert!(f.nudge(), "activation decision needed");
}

// "RejectVerify ... FD returns failure to UA". "PhaseFailed is a state the FD
// stays in".
#[test]
fn a_refused_verify_fails_the_phase_to_the_ua_and_the_fd_stays_in_it() {
    let mut f = fd();
    to_verify_pending(&mut f);
    f.take_ua_message();
    f.query_status(CFG.transfer_ms);

    f.apply(Op::RejectVerify(Reason::Isolated), None, 120)
        .unwrap();

    assert_eq!(
        f.take_ua_message(),
        Some(msg(
            120,
            UaKind::VerifyComplete,
            Code::Failed(Cause::Rejected(Reason::Isolated))
        ))
    );
    assert_eq!(
        f.status(),
        Status::PhaseFailed {
            phase: PhaseId::Verify,
            code: Code::Failed(Cause::Rejected(Reason::Isolated))
        }
    );
    assert!(!f.nudge(), "the orchestrator refused it, so it knows");
    // Stays there while the UA is not idle for long.
    f.poll(120 + CFG.t1_ms - 1);
    assert!(matches!(f.status(), Status::PhaseFailed { .. }));
}

#[test]
fn a_failed_verify_of_its_own_nudges_the_orchestrator() {
    let mut f = FdModel::new(FdConfig {
        verify_passes: false,
        ..CFG
    });
    f.ua_offer(1, 100, 0).unwrap();
    f.apply(Op::AcceptOffer { base: 0 }, None, 0).unwrap();
    f.poll(CFG.transfer_ms);
    f.query_status(CFG.transfer_ms);
    f.apply(Op::PerformVerify, None, CFG.transfer_ms).unwrap();

    f.poll(CFG.transfer_ms + CFG.verify_ms);

    assert_eq!(
        f.status(),
        Status::PhaseFailed {
            phase: PhaseId::Verify,
            code: Code::Failed(Cause::VerifyFailed)
        }
    );
    assert!(f.nudge(), "phase failed");
}

#[test]
fn a_failed_apply_fails_the_phase() {
    let mut f = FdModel::new(FdConfig {
        apply_passes: false,
        ..CFG
    });
    to_verify_pending(&mut f);
    let t = CFG.transfer_ms;
    f.apply(Op::PerformVerify, None, t).unwrap();
    f.poll(t + CFG.verify_ms);
    f.apply(Op::PerformApply, None, t + CFG.verify_ms).unwrap();

    f.poll(t + CFG.verify_ms + CFG.apply_ms);

    assert_eq!(
        f.status(),
        Status::PhaseFailed {
            phase: PhaseId::Apply,
            code: Code::Failed(Cause::ApplyFailed)
        }
    );
}

// "a CancelUpdate puts it in Idle"; "Cancelled lasts only from one command to
// another ... until the orchestrator's AckCancel".
#[test]
fn cancel_update_nudges_and_ack_cancel_returns_the_fd_to_idle() {
    let mut f = fd();
    to_verify_pending(&mut f);
    f.query_status(CFG.transfer_ms);

    assert_eq!(f.ua_cancel_update(150), Code::Success);

    assert_eq!(f.status(), Status::Cancelled);
    assert!(f.nudge(), "Cancelled");
    assert_eq!(f.apply(Op::AckCancel, None, 160), Ok(()));
    assert_eq!(
        f.status(),
        Status::Idle {
            reason: IdleReason::Cancelled
        }
    );
}

// "a CancelUpdateComponent puts the FD in ReadyXfer".
#[test]
fn cancel_update_component_returns_to_ready_xfer_and_allows_a_new_offer() {
    let mut f = fd();
    to_verify_pending(&mut f);
    assert_eq!(f.ua_cancel_update_component(150), Code::Success);
    f.apply(Op::AckCancel, None, 160).unwrap();
    assert_eq!(f.status(), Status::ReadyXfer);

    assert_eq!(f.ua_offer(2, 100, 200), Ok(()));
    assert!(matches!(f.status(), Status::OfferPending { target: 2, .. }));
}

// "CancelUpdate answers immediately ... from the FD's own state".
#[test]
fn a_cancel_while_idle_is_not_in_update_mode() {
    let mut f = fd();
    assert_eq!(f.ua_cancel_update(0), Code::NotInUpdateMode);
    assert_eq!(f.take_ua_message().unwrap().kind, UaKind::CancelResponse);
}

// "Once apply has succeeded the CancelUpdate handler refuses the cancel
// outright".
#[test]
fn a_cancel_after_apply_succeeded_is_refused() {
    let mut f = fd();
    to_activation_pending(&mut f);

    assert_eq!(f.ua_cancel_update(T_ACTIVATION + 1), Code::CancelRefused);
    assert_eq!(f.status(), Status::ActivationPending);
}

// "the FD nudges as soon as apply finishes ... and sends PerformActivate ... The
// FD stores that verdict and answers ActivateFirmware from it".
#[test]
fn a_stored_activation_order_answers_the_request_with_success() {
    let mut f = fd();
    to_activation_pending(&mut f);

    f.apply(Op::PerformActivate, None, T_ACTIVATION).unwrap();
    assert_eq!(f.status(), Status::ReadyXfer);

    drain(&mut f);
    assert_eq!(f.ua_activate(T_ACTIVATION + 500), Code::Success);
    assert_eq!(f.status(), Status::InFlight(PhaseId::Activate));
}

// "the FD nudges again when activation is done".
#[test]
fn activation_ends_idle_with_a_nudge() {
    let mut f = fd();
    to_activation_pending(&mut f);
    f.apply(Op::PerformActivate, None, T_ACTIVATION).unwrap();
    f.ua_activate(T_ACTIVATION + 10);
    f.query_status(T_ACTIVATION + 10);

    f.poll(T_ACTIVATION + 10 + CFG.activate_ms);

    assert_eq!(
        f.status(),
        Status::Idle {
            reason: IdleReason::ActivateFw
        }
    );
    assert!(f.nudge(), "Activated");
}

// "a denial returns INCOMPLETE_UPDATE, which per DSP0267 leaves the FD in READY
// XFER without activating".
#[test]
fn a_denied_activation_answers_incomplete_update_and_stays_ready_xfer() {
    let mut f = fd();
    to_activation_pending(&mut f);
    f.apply(
        Op::RejectActivate(Reason::PolicyViolation),
        None,
        T_ACTIVATION,
    )
    .unwrap();

    assert_eq!(f.ua_activate(T_ACTIVATION + 10), Code::IncompleteUpdate);
    assert_eq!(f.status(), Status::ReadyXfer);
}

// "the order is revocable: a RejectActivate arriving later overrides a stored
// order".
#[test]
fn a_stored_order_is_revocable_until_the_ua_asks() {
    let mut f = fd();
    to_activation_pending(&mut f);
    f.apply(Op::PerformActivate, None, T_ACTIVATION).unwrap();

    assert_eq!(
        f.apply(
            Op::RejectActivate(Reason::Isolated),
            None,
            T_ACTIVATION + 100
        ),
        Ok(())
    );

    assert_eq!(f.ua_activate(T_ACTIVATION + 200), Code::IncompleteUpdate);
}

// "Once the request arrives, the FD has answered and the boot-preference call is
// running, so a refusal racing that request loses."
#[test]
fn a_refusal_racing_the_activation_request_loses() {
    let mut f = fd();
    to_activation_pending(&mut f);
    f.apply(Op::PerformActivate, None, T_ACTIVATION).unwrap();
    f.ua_activate(T_ACTIVATION + 10);

    assert_eq!(
        f.apply(
            Op::RejectActivate(Reason::Isolated),
            None,
            T_ACTIVATION + 11
        ),
        Err(Refusal::TooLate)
    );
    f.poll(T_ACTIVATION + 10 + CFG.activate_ms);
    assert_eq!(
        f.status(),
        Status::Idle {
            reason: IdleReason::ActivateFw
        }
    );
}

// "A session that ends any way but activation, a cancel or an FD_T1 timeout
// alike, invalidates a stored order."
#[test]
fn the_idle_timer_ends_the_session_and_drops_a_stored_order() {
    let mut f = fd();
    to_activation_pending(&mut f);
    f.apply(Op::PerformActivate, None, T_ACTIVATION).unwrap();
    f.query_status(T_ACTIVATION);

    f.poll(T_ACTIVATION + CFG.t1_ms);

    assert_eq!(
        f.status(),
        Status::Idle {
            reason: IdleReason::T1Timeout
        }
    );
    assert!(f.nudge(), "Idle on timeout");
    assert_eq!(f.ua_activate(T_ACTIVATION + CFG.t1_ms + 1), Code::NotReady);
}

// A message to the UA resets the idle timer: "FD_T1 resets on every message".
#[test]
fn a_ua_message_resets_the_idle_timer() {
    let mut f = fd();
    to_activation_pending(&mut f);
    f.apply(Op::PerformActivate, None, T_ACTIVATION).unwrap();
    // The UA asks too early (denied is not the case here), but a failed
    // activate request is still a message that resets the timer.
    f.apply(Op::RejectActivate(Reason::Busy), None, T_ACTIVATION)
        .unwrap();
    f.ua_activate(T_ACTIVATION + CFG.t1_ms - 1);

    f.poll(T_ACTIVATION + CFG.t1_ms + 100);

    assert_eq!(f.status(), Status::ReadyXfer);
}

// "A deadline the FD keeps itself, so a wedged or dead orchestrator does not hold
// a phase open ... treats the expiry as a rejection, failing the phase to the UA".
#[test]
fn a_silent_orchestrator_fails_the_verify_phase_to_the_ua() {
    let mut f = fd();
    to_verify_pending(&mut f);
    f.query_status(CFG.transfer_ms);
    drain(&mut f);

    f.poll(CFG.transfer_ms + CFG.order_window_ms);

    assert_eq!(
        f.take_ua_message(),
        Some(msg(
            CFG.transfer_ms + CFG.order_window_ms,
            UaKind::VerifyComplete,
            Code::Failed(Cause::OrderTimeout)
        ))
    );
    assert!(matches!(f.status(), Status::PhaseFailed { .. }));
    assert!(f.nudge());
}

#[test]
fn a_silent_orchestrator_aborts_the_offer() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();

    f.poll(CFG.order_window_ms);

    assert_eq!(
        f.take_ua_message(),
        Some(msg(
            CFG.order_window_ms,
            UaKind::TransferComplete,
            Code::FdAbortedTransfer
        ))
    );
    assert_eq!(
        f.status(),
        Status::Idle {
            reason: IdleReason::OrderTimeout
        }
    );
}

// An unanswered activation decision is a denial.
#[test]
fn a_silent_orchestrator_denies_activation() {
    let mut f = fd();
    to_activation_pending(&mut f);

    f.poll(T_ACTIVATION + CFG.order_window_ms);

    assert_eq!(f.status(), Status::ReadyXfer);
    assert_eq!(
        f.ua_activate(T_ACTIVATION + CFG.order_window_ms + 1),
        Code::IncompleteUpdate
    );
}

// "SVN commit requested (UA sent UpdateSecurityRevision)"; PerformSvnCommit and
// RejectSvnCommit; "the FD accepts 0x22 only in the IDLE state".
#[test]
fn a_security_revision_request_is_gated_by_the_orchestrator() {
    let mut f = fd();
    assert_eq!(f.ua_security_revision(9, 0), Ok(()));
    assert!(f.nudge());
    assert_eq!(f.status(), Status::SvnCommitPending { component: 9 });

    f.apply(Op::PerformSvnCommit, None, 5).unwrap();

    assert_eq!(
        f.take_ua_message(),
        Some(msg(5, UaKind::SecurityRevisionResponse, Code::Success))
    );
    assert!(matches!(f.status(), Status::Idle { .. }));
}

#[test]
fn a_refused_security_revision_commit_tells_the_ua_not_permitted() {
    let mut f = fd();
    f.ua_security_revision(9, 0).unwrap();

    f.apply(Op::RejectSvnCommit(Reason::PolicyViolation), None, 5)
        .unwrap();

    assert_eq!(
        f.take_ua_message(),
        Some(msg(5, UaKind::SecurityRevisionResponse, Code::NotPermitted))
    );
}

#[test]
fn a_security_revision_request_outside_idle_is_not_permitted() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();

    assert_eq!(f.ua_security_revision(9, 1), Err(Code::NotPermitted));
}

// The generation names the phase an operation was meant for. This is the design's
// open question: "whether an order carries a token".
#[test]
fn an_operation_naming_an_old_generation_is_refused() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();
    let old = f.query_status(0).generation;
    f.apply(Op::AcceptOffer { base: 0 }, Some(old), 0).unwrap();
    f.poll(CFG.transfer_ms);

    assert_eq!(
        f.apply(Op::PerformVerify, Some(old), CFG.transfer_ms),
        Err(Refusal::StaleGeneration)
    );
    let now = f.query_status(CFG.transfer_ms).generation;
    assert_eq!(
        f.apply(Op::PerformVerify, Some(now), CFG.transfer_ms),
        Ok(())
    );
}

#[test]
fn the_generation_advances_each_time_the_fd_parks_for_the_orchestrator() {
    let mut f = fd();
    let g0 = f.generation();
    f.ua_offer(1, 100, 0).unwrap();
    let g1 = f.generation();
    f.apply(Op::AcceptOffer { base: 0 }, None, 0).unwrap();
    f.poll(CFG.transfer_ms);
    let g2 = f.generation();

    assert!(g0 != g1 && g1 != g2 && g0 != g2);
}

// "One bit, no framing, no lost messages": a nudge raised while another is still
// up is merged, and the orchestrator never sees the state in between.
#[test]
fn a_nudge_raised_while_one_is_up_is_merged_and_a_state_goes_unseen() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();
    f.query_status(0);
    f.apply(Op::AcceptOffer { base: 0 }, None, 0).unwrap();

    // The transfer finishes (VerifyPending, nudge raised), and the orchestrator
    // does not query before its window runs out (PhaseFailed, nudge raised again).
    f.poll(CFG.transfer_ms + CFG.order_window_ms);

    assert_eq!(f.nudges_merged(), 1);
    // The one query that follows sees only the later state, PhaseFailed. It never
    // saw VerifyPending.
    let now = CFG.transfer_ms + CFG.order_window_ms;
    assert!(matches!(
        f.query_status(now).status,
        Status::PhaseFailed { .. }
    ));
}

#[test]
fn next_event_at_follows_the_phase() {
    let mut f = fd();
    assert_eq!(f.next_event_at(), None, "idle");

    f.ua_offer(1, 100, 0).unwrap();
    assert_eq!(
        f.next_event_at(),
        Some(CFG.order_window_ms),
        "the orchestrator's window"
    );

    f.apply(Op::AcceptOffer { base: 0 }, None, 0).unwrap();
    assert_eq!(f.next_event_at(), Some(CFG.transfer_ms), "transfer running");
}

#[test]
fn a_late_poll_applies_every_due_change_in_order_stamped_with_its_due_time() {
    let mut f = fd();
    f.ua_offer(1, 100, 0).unwrap();
    f.apply(Op::AcceptOffer { base: 0 }, None, 0).unwrap();

    f.poll(100_000);

    assert_eq!(
        f.take_ua_message(),
        Some(msg(
            CFG.transfer_ms,
            UaKind::TransferComplete,
            Code::Success
        ))
    );
    assert_eq!(
        f.take_ua_message(),
        Some(msg(
            CFG.transfer_ms + CFG.order_window_ms,
            UaKind::VerifyComplete,
            Code::Failed(Cause::OrderTimeout)
        ))
    );
}

#[test]
fn ua_messages_beyond_the_queue_are_counted_not_kept() {
    let mut f = fd();
    for i in 0..12 {
        f.ua_cancel_update(i);
    }
    assert!(f.dropped() > 0);
    assert!(drain(&mut f) <= 8);
}

#[test]
fn a_clock_that_goes_backwards_does_not_panic() {
    let mut f = fd();
    f.ua_offer(1, 100, 500).unwrap();
    f.poll(10);
    assert_eq!(f.apply(Op::AcceptOffer { base: 0 }, None, 5), Ok(()));
}

#[test]
fn a_huge_time_does_not_overflow() {
    let mut f = FdModel::new(FdConfig {
        order_window_ms: u64::MAX,
        ..CFG
    });
    f.ua_offer(1, 100, u64::MAX - 1).unwrap();
    assert_eq!(f.next_event_at(), Some(u64::MAX));
}
