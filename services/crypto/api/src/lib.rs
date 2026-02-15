// Licensed under the Apache-2.0 license

//! Crypto Service API
//!
//! Shared definitions for the crypto client-server IPC:
//! - `protocol` — wire format (headers, op codes, error codes)
//! - `backend` — backend abstraction (algorithm markers, `OneShot`, `Streaming`)

#![no_std]

pub mod backend;
pub mod protocol;

pub use protocol::*;
