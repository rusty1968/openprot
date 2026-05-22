// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

use console_backend::console_backend_write_all;
use target_common::{TargetInterface, declare_target};
use {console_backend as _, entry as _};

pub struct Target;

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 NOP Test";

    fn main() -> ! {
        codegen::start();
        loop {}
    }

    fn shutdown(code: u32) -> ! {
        let sentinel: &[u8] = if code == 0 { b"TEST_RESULT:PASS\n" } else { b"TEST_RESULT:FAIL\n" };
        let _ = console_backend_write_all(sentinel);
        loop {}
    }
}

declare_target!(Target);
