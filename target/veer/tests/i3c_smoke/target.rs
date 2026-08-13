// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! I3C smoke test.
//!
//! Verifies that:
//! 1. `enable_rx_interrupt()` runs without trapping.
//! 2. A private write sent by the test harness over TCP port 65534 is
//!    received correctly with the expected payload [0x01, 0x02, 0x03, 0x04].

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
    const NAME: &'static str = "Caliptra I3C Smoke Test";

    fn main() -> ! {
        // SAFETY: single call at boot; Caliptra ROM has already initialized
        // the I3C core and we are the only owner of the peripheral.
        let mut i3c = unsafe { CaliptraI3cTarget::new() };
        i3c.enable_rx_interrupt();
        pw_log::info!("I3C smoke test: waiting for private write");

        let mut buf = [0u8; 64];
        // Poll until we receive a write or exhaust the iteration budget.
        const MAX_POLLS: u32 = 10_000_000;
        let mut polls = 0u32;
        loop {
            if polls % 1_000_000 == 0 {
                let pending = i3c.rx_pending();
                pw_log::info!(
                    "I3C smoke trace: poll={} rx_pending={}",
                    polls,
                    if pending { 1u32 } else { 0u32 }
                );
            }

            if let Some(len) = i3c.rx_read(&mut buf) {
                pw_log::info!(
                    "I3C smoke trace: rx_read len={} first4={:02x} {:02x} {:02x} {:02x}",
                    len as u32,
                    buf[0],
                    buf[1],
                    buf[2],
                    buf[3]
                );
                let expected = [0x01u8, 0x02, 0x03, 0x04];
                if len >= 4 && buf[..4] == expected {
                    pw_log::info!("I3C smoke test: received expected payload OK");
                    exit(0);
                } else {
                    pw_log::info!("I3C smoke test: unexpected payload len={}", len as u32);
                    exit(1);
                }
            }
            polls += 1;
            if polls >= MAX_POLLS {
                pw_log::info!("I3C smoke test: timed out waiting for write");
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
