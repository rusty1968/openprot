// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! The firmware-device side of the PLDM-FD-as-IPC-server design (#464), as an
//! executable model.
//!
//! The design has the PLDM Firmware Device (FD) serve the orchestrator over IPC.
//! The FD runs the update protocol with the Update Agent (UA); at each phase it
//! parks, raises a nudge, and waits for the orchestrator to order or refuse the
//! phase. This crate is that FD as a pure state machine: no kernel, no clock, no
//! dependencies, no heap. Time is an argument, in milliseconds, and every
//! change is stamped with the moment it was due.
//!
//! It exists so the design's claims can be run instead of read. A QEMU service
//! wraps it for the kernel-level claims (signals, one transaction per channel,
//! deadlines), and the host tests here pin the rules of the protocol itself.
//!
//! # What it models
//!
//! - The orchestrator's operations: [`Op`], answered at once, with a
//!   [`Refusal`] when the FD is in the wrong phase.
//! - The FD's conditions: [`Status`], including `PhaseFailed` and `Cancelled`.
//! - The nudge: a level, raised when the FD hands a state to the orchestrator,
//!   cleared when a query is answered. Raises that find it already up are
//!   counted, because each is a state the orchestrator may never see.
//! - The Update Agent's requests, as `ua_*` methods, and what the FD tells it
//!   back, as [`UaMessage`]s.
//! - Two timers the design names: a window the FD keeps for the orchestrator's
//!   answer (silence is a refusal), and the idle timer the Update Agent resets
//!   with every message.
//! - A phase generation, so an operation can name the phase it was meant for.
//!
//! # Assumptions where the design is silent
//!
//! Each is a question for the design, found by writing the model:
//!
//! 1. A query between nudges, while a phase runs, answers
//!    [`Status::InFlight`]. The design's list of conditions has no such entry.
//! 2. After the orchestrator refuses an offer, the FD goes idle
//!    ([`IdleReason::OfferRejected`]). The design says only that the transfer
//!    ends with `FdAbortedTransfer`.
//! 3. An `ActivateFirmware` that arrives before the orchestrator has decided is
//!    answered `INCOMPLETE_UPDATE`, as if the order were denied.
//! 4. A `PerformActivate` after a `RejectActivate` is refused as the wrong
//!    phase: a denial is final.
//! 5. The idle timer also runs while the FD holds a stored activation order.
//! 6. A cancel is accepted while verify or apply is running, not only while
//!    parked.
//! 7. A change the orchestrator itself causes does not raise the nudge.
//!
//! # Out of scope
//!
//! Out-of-transport transfer, the wire encoding, the write-access containment
//! layers, and `pldm-lib`'s own state machine.

#![no_std]
#![forbid(unsafe_code)]

mod model;
mod types;

pub use model::FdModel;
pub use types::{
    Cause, Code, FdConfig, IdleReason, Op, OpResult, PhaseId, Reason, Refusal, Status, StatusReply,
    UaKind, UaMessage,
};

#[cfg(test)]
mod tests;
