// Licensed under the Apache-2.0 license

//! SPDM Responder Service
//!
//! This service implements the SPDM responder (server) role, which responds
//! to attestation and measurement requests from SPDM requesters.
//!
//! ## Overview
//!
//! The SPDM responder receives requests and provides:
//! - Version and capability information
//! - Certificate chains for authentication
//! - Device measurements
//! - Challenge-response attestation
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────┐
//! │  MCTP Transport         │
//! │  (incoming messages)    │
//! └───────────┬─────────────┘
//!             │ SPDM requests
//!             ▼
//! ┌─────────────────────────┐
//! │  SPDM Responder         │◄── This crate
//! │  (request handler)      │
//! └───────────┬─────────────┘
//!             │
//!             ▼
//! ┌─────────────────────────┐
//! │  Crypto Service         │
//! │  Storage Service        │
//! └─────────────────────────┘
//! ```

#![no_std]
#![warn(missing_docs)]

/// SPDM responder state and configuration.
#[derive(Debug)]
pub struct SpdmResponder {
    /// Local endpoint ID for MCTP transport.
    pub local_eid: u8,
}

impl SpdmResponder {
    /// Create a new SPDM responder with the given local endpoint ID.
    pub fn new(local_eid: u8) -> Self {
        Self { local_eid }
    }

    /// Get the local endpoint ID.
    pub fn local_eid(&self) -> u8 {
        self.local_eid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_responder_creation() {
        let responder = SpdmResponder::new(8);
        assert_eq!(responder.local_eid(), 8);
    }
}
