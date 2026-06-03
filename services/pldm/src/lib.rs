// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! # openprot-pldm-service
//!
//! PLDM-over-MCTP responder service built on top of
//! [`openprot-mctp-api`] and [`pldm-interface`].
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────┐
//! │   Application / Firmware │  creates PldmResponder / PldmRequester, calls run_once()
//! └───────────┬──────────────┘
//!             │
//!             ▼
//! ┌──────────────────────────┐
//! │   openprot-pldm-service  │◄── this crate
//! │   PldmResponder          │  dispatches to CmdInterface (responder side)
//! │   PldmRequester          │  sends PLDM requests (initiator side)
//! └───────────┬──────────────┘
//!             │ MctpListener / MctpReqChannel / MctpRespChannel
//!             ▼
//! ┌──────────────────────────┐
//! │   openprot-mctp-api      │  Stack<C: MctpClient>
//! │   (Stack facade)         │
//! └───────────┬──────────────┘
//!             │ IPC / transport
//!             ▼
//! ┌──────────────────────────┐
//! │   MCTP Server            │
//! └──────────────────────────┘
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use openprot_pldm_service::{PldmResponder, PldmRequester};
//! use pldm_interface::control_context::ProtocolCapability;
//! use pldm_common::protocol::base::{PldmControlCmd, PldmSupportedType};
//!
//! const CTRL_CMDS: [u8; 5] = [
//!     PldmControlCmd::SetTid as u8,
//!     PldmControlCmd::GetTid as u8,
//!     PldmControlCmd::GetPldmCommands as u8,
//!     PldmControlCmd::GetPldmVersion as u8,
//!     PldmControlCmd::GetPldmTypes as u8,
//! ];
//!
//! let caps = [
//!     ProtocolCapability::new(PldmSupportedType::Base, "1.1.0", &CTRL_CMDS).unwrap(),
//! ];
//!
//! // Responder: handle inbound PLDM requests from the Update Agent.
//! let mut responder = PldmResponder::new(&caps);
//! let mut buf = [0u8; 1024];
//! loop {
//!     if let Err(e) = responder.run_once(&stack, &mut buf, 0) {
//!         // handle or log error
//!     }
//! }
//!
//! // Requester: queue and send PLDM requests to a remote endpoint.
//! let mut requester = PldmRequester::new(&caps);
//! requester.queue_get_tid();
//! loop {
//!     if let Err(e) = requester.run_once(&stack, REMOTE_EID, &mut buf, 0) {
//!         // handle or log error
//!     }
//! }
//! ```

#![no_std]
#![warn(missing_docs)]

pub mod error;
pub mod requester;
pub mod responder;
pub mod transport;

pub use error::PldmServiceError;
pub use requester::{PldmRequester, PldmRequesterCommand};
pub use transport::MctpPldmTransport;
pub use responder::{PldmResponder, PLDM_MSG_TYPE};
