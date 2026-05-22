// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use openprot_mctp_api::Stack;
use openprot_mctp_client_ipc::IpcMctpClient;
use openprot_mctp_echo::{prepare_listener, run};
use userspace::{entry, syscall};

#[entry]
fn entry() {
    let stack = Stack::new(IpcMctpClient::new(app_mctp_echo_client::handle::MCTP));
    let mut listener = match prepare_listener(&stack) {
        Ok(listener) => listener,
        Err(e) => {
            pw_log::error!("echo setup failed: code={}", e.code as u32);
            syscall::process_exit(1);
        }
    };

    run(&mut listener);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
