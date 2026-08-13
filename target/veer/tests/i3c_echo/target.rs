// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! I3C echo test.
//!
//! Echoes every received private write back verbatim via the TTI TX queue
//! (PEC byte included), so the host can read it back with a private read.
//! A write whose payload starts with ASCII "DONE" ends the test with
//! exit(0). Reception is pure-polling: this system image has an empty
//! interrupt table.

#![no_std]
#![no_main]

use caliptra_i3c_target::CaliptraI3cTarget;
use entry::exit;
use target_common::{declare_target, TargetInterface};
use {console_backend as _, kernel as _};

pub struct Target {}

// Emits PW_KERNEL_INTERRUPT_TABLE from the interrupt_table in system.json5.
codegen::declare_kernel_interrupt_handlers!();

impl TargetInterface for Target {
    const NAME: &'static str = "Caliptra I3C Echo Test";

    fn main() -> ! {
        // SAFETY: single call at boot; Caliptra ROM has already initialized
        // the I3C core and we are the only owner of the peripheral.
        let mut i3c = unsafe { CaliptraI3cTarget::new() };
        pw_log::info!("I3C echo test: waiting for private write");

        let mut buf = [0u8; 64];
        const MAX_POLLS: u32 = 10_000_000;
        let mut polls = 0u32;
        loop {
            if let Some(len) = i3c.rx_read(&mut buf) {
                if len >= 4 && buf[..4] == *b"DONE" {
                    pw_log::info!("I3C echo test: received DONE");
                    exit(0);
                }
                let len = len.min(buf.len());
                i3c.tx_write(&buf[..len]);
                pw_log::info!("I3C echo trace: echoed {} bytes", len as u32);
            }
            polls += 1;
            if polls >= MAX_POLLS {
                pw_log::info!("I3C echo test: timed out waiting for write");
                exit(2);
            }
        }
    }

    fn shutdown(code: u32) -> ! {
        match code {
            0 => pw_log::info!("PASS"),
            _ => pw_log::info!("FAIL: {}", code),
        }
        exit(code);
    }
}

declare_target!(Target);
