// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Reusable MCTP echo loop.
//!
//! This crate keeps the echo application policy separate from target wiring:
//! callers create an `openprot_mctp_api::Stack`, then hand it to the helpers
//! here to configure the local EID, open a listener, and echo received payloads
//! back to the sender.

#![no_std]

use openprot_mctp_api::{MctpClient, MctpError, MctpRespChannel, Stack, StackListener};

/// Default MCTP message type used by the echo app.
pub const ECHO_MSG_TYPE: u8 = 1;

/// Default local EID used by the echo app.
pub const ECHO_EID: u8 = 8;

/// Prepare a stack for echoing by setting the local EID and opening a listener.
pub fn prepare_listener<C: MctpClient>(stack: &Stack<C>) -> Result<StackListener<'_, C>, MctpError> {
    stack.set_eid(ECHO_EID)?;
    stack.listener(ECHO_MSG_TYPE, 0)
}

/// Run the echo loop forever.
pub fn run<L>(listener: &mut L) -> !
where
    L: openprot_mctp_api::MctpListener,
{
    let mut buf = [0u8; 255];
    loop {
        match listener.recv(&mut buf) {
            Ok((_meta, msg, mut resp)) => {
                let _ = resp.send(msg);
            }
            Err(e) => {
                pw_log::error!("echo recv failed: code={}", e.code as u32);
            }
        }
    }
}
