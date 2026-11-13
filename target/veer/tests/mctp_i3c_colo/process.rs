// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Process for the colocated i3c + MCTP server (`veer_mctp_i3c_colo`), the same in
//! every image that runs it. The system config gives the process a WaitGroup
//! (`wg`), the I3C interrupt (`i3c_irq`, raising `i3c`) and the `mctp` channel.

#![no_main]
#![no_std]

use mctp_i3c_colo_codegen::{handle, signals};
use userspace::process_entry;
use userspace::syscall;
use veer_mctp_i3c_colo::Handles;

#[process_entry("mctp_i3c_colo")]
fn entry() {
    let handles = Handles {
        wg: handle::WG,
        i3c_irq: handle::I3C_IRQ,
        i3c_signals: signals::I3C,
        mctp: handle::MCTP,
    };
    if veer_mctp_i3c_colo::run(handles).is_err() {
        pw_log::error!("colocated server exiting with error");
        let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
    }
    #[expect(clippy::empty_loop)]
    loop {}
}
