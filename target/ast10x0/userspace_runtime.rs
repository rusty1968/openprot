// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

/// Production fail-stop path for AST10x0 userspace services.
///
/// Logs a fatal event and enters a non-returning spin loop.
pub fn fail_stop(service_name: &str, status_code: u32) -> ! {
    pw_log::error!("{} fatal status=0x{:08x}", service_name, status_code);
    loop {
        core::hint::spin_loop();
    }
}
