// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 USART Service Target
//!
//! This target runs the USART server as a userspace process.
//! Clients can communicate with it over an IPC channel.

#![no_std]
#![no_main]

use target_common::{TargetInterface, declare_target};
use {console_backend as _, entry as _};

pub struct Target {}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 USART Service";

    fn main() -> ! {
        codegen::start();
        #[expect(clippy::empty_loop)]
        loop {}
    }

    fn shutdown(_code: u32) -> ! {
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
