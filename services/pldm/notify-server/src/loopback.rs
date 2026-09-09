// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! In-process transport: client marshalling -> [`dispatch`] -> a shared
//! [`NotifyState`], no kernel. Host-testable path that exercises the real
//! client encoders/decoders against the real server dispatch.

use notify_api::{Transport, TransportError};

use crate::{dispatch, NotifyState};

/// A [`Transport`] that runs [`dispatch`] against a borrowed [`NotifyState`].
pub struct LoopbackTransport<'a> {
    state: &'a mut NotifyState,
}

impl<'a> LoopbackTransport<'a> {
    pub fn new(state: &'a mut NotifyState) -> Self {
        Self { state }
    }
}

impl Transport for LoopbackTransport<'_> {
    fn transact(&mut self, req: &[u8], resp: &mut [u8]) -> Result<usize, TransportError> {
        // `dispatch` always produces a valid response frame (it encodes
        // notify errors *into* the payload), so the transport layer never fails.
        Ok(dispatch(self.state, req, resp))
    }
}
