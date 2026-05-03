// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_flash_client::handle;
use flash_client::FlashClient;
use userspace::entry;
use userspace::syscall;

fn check_exists(controller_id: u32, client: &FlashClient) -> Result<(), pw_status::Error> {
    match client.exists() {
        Ok(true) => {
            pw_log::info!("flash exists check passed controller={}", controller_id as u32);
            Ok(())
        }
        Ok(false) => {
            pw_log::error!(
                "flash exists check reported absent controller={}",
                controller_id as u32
            );
            Err(pw_status::Error::Unknown)
        }
        Err(_) => {
            pw_log::error!("flash exists IPC failed controller={}", controller_id as u32);
            Err(pw_status::Error::Internal)
        }
    }
}

#[entry]
fn entry() -> ! {
    let fmc = FlashClient::new(handle::FLASH_FMC);
    let spi1 = FlashClient::new(handle::FLASH_SPI1);
    let spi2 = FlashClient::new(handle::FLASH_SPI2);

    let status = check_exists(0, &fmc)
        .and_then(|_| check_exists(1, &spi1))
        .and_then(|_| check_exists(2, &spi2));

    let _ = match status {
        Ok(()) => syscall::debug_shutdown(Ok(())),
        Err(e) => syscall::debug_shutdown(Err(e)),
    };

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
