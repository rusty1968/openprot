// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! I3C IBI test.
//!
//! Raises an IBI (MDB 0xA5 with a 4-byte payload) each time a private write
//! arrives, so the host can assert the IBI reaches the controller side. A
//! write whose payload starts with ASCII "DONE" ends the test with exit(0).
//! Reception is pure-polling: this system image has an empty interrupt
//! table.

#![no_std]
#![no_main]

use caliptra_i3c_target::CaliptraI3cTarget;
use entry::exit;
use target_common::{declare_target, TargetInterface};
use {console_backend as _, kernel as _};

pub struct Target {}

// Emits PW_KERNEL_INTERRUPT_TABLE from the interrupt_table in system.json5.
codegen::declare_kernel_interrupt_handlers!();

const IBI_MDB: u8 = 0xA5;
const IBI_PAYLOAD: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

impl TargetInterface for Target {
    const NAME: &'static str = "Caliptra I3C IBI Test";

    fn main() -> ! {
        // SAFETY: single call at boot; Caliptra ROM has already initialized
        // the I3C core and we are the only owner of the peripheral.
        let mut i3c = unsafe { CaliptraI3cTarget::new() };
        pw_log::info!("I3C IBI test: waiting for private write");

        let mut buf = [0u8; 64];
        const MAX_POLLS: u32 = 10_000_000;
        let mut polls = 0u32;
        loop {
            if let Some(len) = i3c.rx_read(&mut buf) {
                if len >= 4 && buf[..4] == *b"DONE" {
                    pw_log::info!("I3C IBI test: received DONE");
                    exit(0);
                }
                i3c.ibi_raise(IBI_MDB, &IBI_PAYLOAD);
                pw_log::info!("I3C IBI trace: raised IBI");
            }
            polls += 1;
            if polls >= MAX_POLLS {
                pw_log::info!("I3C IBI test: timed out waiting for write");
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
