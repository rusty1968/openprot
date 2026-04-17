// Licensed under the Apache-2.0 license

//! In-process [`MctpClient`] implementation — no IPC channel required.
//!
//! [`DirectMctpClient`] wraps a `&RefCell<Server<S, N>>` and implements the
//! [`MctpClient`] trait by calling [`Server`] methods directly. This allows
//! application code written against [`MctpClient`] (e.g. [`MctpSpdmTransport`])
//! to run inside the same process as the MCTP server without going through a
//! Pigweed IPC channel.
//!
//! [`MctpSpdmTransport`]: openprot_spdm_transport_mctp::MctpSpdmTransport
//!
//! # Usage
//!
//! ```rust,ignore
//! use core::cell::RefCell;
//! use openprot_mctp_server::{Server, direct_client::DirectMctpClient};
//!
//! let server = RefCell::new(Server::new(mctp::Eid(8), 0, sender));
//! let client = DirectMctpClient::new(&server);
//! // pass `client` to MctpSpdmTransport::new_responder(client)
//! ```

use core::cell::RefCell;

use mctp_lib::Sender;
use openprot_mctp_api::{Handle, MctpClient, MctpError, RecvMetadata, ResponseCode};

use crate::server::Server;

/// An [`MctpClient`] that calls [`Server`] methods directly (no IPC).
///
/// The shared reference uses [`RefCell`] for interior mutability, matching
/// the `&self` signature of the [`MctpClient`] trait while allowing `Server`
/// mutation. Safe on single-threaded embedded targets where there is no
/// concurrent access.
pub struct DirectMctpClient<'a, S: Sender, const N: usize> {
    server: &'a RefCell<Server<S, N>>,
}

impl<'a, S: Sender, const N: usize> DirectMctpClient<'a, S, N> {
    /// Create a new `DirectMctpClient` wrapping the given server cell.
    pub fn new(server: &'a RefCell<Server<S, N>>) -> Self {
        Self { server }
    }
}

impl<S: Sender, const N: usize> MctpClient for DirectMctpClient<'_, S, N> {
    fn req(&self, eid: u8) -> Result<Handle, MctpError> {
        self.server.borrow_mut().req(eid)
    }

    fn listener(&self, msg_type: u8) -> Result<Handle, MctpError> {
        self.server.borrow_mut().listener(msg_type)
    }

    fn get_eid(&self) -> u8 {
        self.server.borrow().get_eid()
    }

    fn set_eid(&self, eid: u8) -> Result<(), MctpError> {
        self.server.borrow_mut().set_eid(eid)
    }

    /// Poll for a received message on `handle`.
    ///
    /// Returns `Err(TimedOut)` immediately if no message is available.
    /// The caller (e.g. `MctpSpdmTransport`) should treat `TimedOut` as
    /// "nothing ready yet" and retry on the next loop iteration.
    fn recv(
        &self,
        handle: Handle,
        _timeout_millis: u32,
        buf: &mut [u8],
    ) -> Result<RecvMetadata, MctpError> {
        self.server
            .borrow_mut()
            .try_recv(handle, buf)
            .ok_or(MctpError::from_code(ResponseCode::TimedOut))
    }

    fn send(
        &self,
        handle: Option<Handle>,
        msg_type: u8,
        eid: Option<u8>,
        tag: Option<u8>,
        integrity_check: bool,
        buf: &[u8],
    ) -> Result<u8, MctpError> {
        self.server
            .borrow_mut()
            .send(handle, msg_type, eid, tag, integrity_check, buf)
    }

    fn drop_handle(&self, handle: Handle) {
        let _ = self.server.borrow_mut().unbind(handle);
    }
}
