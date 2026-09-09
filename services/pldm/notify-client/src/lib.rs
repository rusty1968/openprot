// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Client for the PLDM notify channel.
//!
//! `PldmLink<T>` is the Orchestrator-side marshalling client, generic over
//! [`Transport`]: the same encode/decode code runs in production
//! (`IpcTransport`, Phase 2, cross-process) and in host tests
//! (`LoopbackTransport`, in-process against a shared `NotifyState`).
//!
//! This crate has **no kernel/IPC dependency** and builds on the host — that
//! is what makes the encoders/decoders testable without a kernel.

#![no_std]

use notify_api::{
    Decision, NotifyError, NotifyOp, NotifyRequestHeader, NotifyResponseHeader, Pending, Phase,
    Transport, TransportError, MAX_BUF_SIZE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    Transport(TransportError),
    ServerError(NotifyError),
    InvalidResponse,
    /// Request or response would exceed `MAX_BUF_SIZE`.
    BufferTooSmall,
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "notify transport error: {e}"),
            Self::ServerError(e) => write!(f, "notify server error: {e}"),
            Self::InvalidResponse => f.write_str("malformed notify response"),
            Self::BufferTooSmall => f.write_str("request or response exceeds one round-trip buffer"),
        }
    }
}

impl core::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Transport(e) => Some(e),
            Self::ServerError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<TransportError> for ClientError {
    fn from(e: TransportError) -> Self {
        Self::Transport(e)
    }
}

/// An Orchestrator-side link to PLDM's notify channel, speaking the
/// `notify_api` wire protocol over any [`Transport`]. Marshalling only, no
/// syscalls.
pub struct PldmLink<T: Transport> {
    transport: T,
}

impl<T: Transport> PldmLink<T> {
    /// Create a link bound to `transport`.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// One whole request/response round-trip. `req_payload` is copied after
    /// the request header; on success, the response payload is copied into
    /// `resp_payload` and its length returned.
    fn call(
        &mut self,
        op: NotifyOp,
        req_payload: &[u8],
        resp_payload: &mut [u8],
    ) -> Result<usize, ClientError> {
        let req_len = NotifyRequestHeader::SIZE + req_payload.len();
        if req_len > MAX_BUF_SIZE {
            return Err(ClientError::BufferTooSmall);
        }
        let hdr = NotifyRequestHeader::new(op, req_payload.len() as u16);
        let mut req = [0u8; MAX_BUF_SIZE];
        req[..NotifyRequestHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
        req[NotifyRequestHeader::SIZE..req_len].copy_from_slice(req_payload);

        let mut resp = [0u8; MAX_BUF_SIZE];
        let resp_len = self.transport.transact(&req[..req_len], &mut resp)?;

        if resp_len < NotifyResponseHeader::SIZE {
            return Err(ClientError::InvalidResponse);
        }
        let Some(rhdr) = zerocopy::Ref::<_, NotifyResponseHeader>::from_bytes(
            &resp[..NotifyResponseHeader::SIZE],
        )
        .ok() else {
            return Err(ClientError::InvalidResponse);
        };
        if !rhdr.is_success() {
            return Err(ClientError::ServerError(
                rhdr.error_code().unwrap_or(NotifyError::InternalError),
            ));
        }
        let n = rhdr.payload_length();
        if resp_len < NotifyResponseHeader::SIZE + n || n > resp_payload.len() {
            return Err(ClientError::InvalidResponse);
        }
        resp_payload[..n]
            .copy_from_slice(&resp[NotifyResponseHeader::SIZE..NotifyResponseHeader::SIZE + n]);
        Ok(n)
    }

    /// Arm PLDM to nudge this channel's peer signal on future events.
    ///
    /// # Errors
    /// - [`ClientError::Transport`] — the round-trip failed.
    /// - [`ClientError::ServerError`] — PLDM rejected the request.
    /// - [`ClientError::InvalidResponse`] — the response was malformed.
    pub fn subscribe(&mut self) -> Result<(), ClientError> {
        self.call(NotifyOp::Subscribe, &[], &mut []).map(|_| ())
    }

    /// Drain the latched event, if any.
    ///
    /// # Errors
    /// - [`ClientError::Transport`] — the round-trip failed (including a
    ///   [`TransportError::Timeout`] on a silent/unhealthy peer).
    /// - [`ClientError::ServerError`] — PLDM reported an error other than
    ///   "nothing pending" (which is folded into `Ok(None)`).
    /// - [`ClientError::InvalidResponse`] — the response was malformed.
    pub fn poll(&mut self) -> Result<Option<Pending>, ClientError> {
        let mut buf = [0u8; Pending::MAX_ENCODED_LEN];
        match self.call(NotifyOp::Poll, &[], &mut buf) {
            Ok(n) => Pending::decode(&buf[..n])
                .map(Some)
                .map_err(ClientError::ServerError),
            Err(ClientError::ServerError(NotifyError::NoPending)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Answer a pending `UpdateRequested` with `decision`.
    ///
    /// # Errors
    /// - [`ClientError::Transport`] — the round-trip failed.
    /// - [`ClientError::ServerError`] — PLDM rejected the decision.
    /// - [`ClientError::InvalidResponse`] — the response was malformed.
    pub fn decide(&mut self, decision: Decision) -> Result<(), ClientError> {
        self.call(NotifyOp::Decision, &[decision as u8], &mut [])
            .map(|_| ())
    }

    /// Report update-progress `phase` to PLDM.
    ///
    /// # Errors
    /// - [`ClientError::Transport`] — the round-trip failed.
    /// - [`ClientError::ServerError`] — PLDM rejected the status.
    /// - [`ClientError::InvalidResponse`] — the response was malformed.
    pub fn push_status(&mut self, phase: Phase) -> Result<(), ClientError> {
        self.call(NotifyOp::PushStatus, &[phase as u8], &mut [])
            .map(|_| ())
    }
}
