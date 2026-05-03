// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_flash_client::handle;
use flash_client::FlashClient;
use userspace::entry;
use userspace::syscall;

#[entry]
fn entry() -> ! {
    let client = FlashClient::new(handle::FLASH);

    match client.exists() {
        Ok(true) => {
            pw_log::info!("flash exists check passed");
            let _ = syscall::debug_shutdown(Ok(()));
        }
        Ok(false) => {
            pw_log::error!("flash exists check reported absent");
            let _ = syscall::debug_shutdown(Err(pw_status::Error::Unknown));
        }
        Err(_) => {
            pw_log::error!("flash exists IPC failed");
            let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
        }
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
