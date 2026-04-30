// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use openprot_mctp_api::{MctpListener, MctpRespChannel, Stack};
use openprot_mctp_client::IpcMctpClient;
use pw_status::Error;
use userspace::{entry, syscall};

#[entry]
fn entry() -> ! {
    let stack = Stack::new(IpcMctpClient::new(app_mctp_echo_client::handle::MCTP));
    if let Err(e) = stack.set_eid(8) {
        pw_log::error!("set_eid failed: code={}", e.code as u8);
        let _ = syscall::debug_shutdown(Err(Error::Internal));
        loop {}
    }
    let Ok(mut listener) = stack.listener(1, 0) else {
        pw_log::error!("listener registration failed");
        let _ = syscall::debug_shutdown(Err(Error::Internal));
        loop {}
    };
    let mut buf = [0u8; 255];
    loop {
        match listener.recv(&mut buf) {
            Ok((_meta, msg, mut resp)) => {
                let _ = resp.send(msg);
            }
            Err(_) => {}
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
