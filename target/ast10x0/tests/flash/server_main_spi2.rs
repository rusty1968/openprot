// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_flash_server_spi2::{handle, signals};
use flash_backend::Backend;
use flash_server::runtime;
use userspace::entry;
use userspace::syscall::{self, Signals};

#[entry]
fn entry() -> ! {
    let mut backend = Backend::new_spi2();

    let _ = syscall::wait_group_add(
        handle::WG,
        handle::FLASH,
        Signals::READABLE,
        handle::FLASH as usize,
    );
    let _ = syscall::wait_group_add(
        handle::WG,
        handle::SPI2_IRQ,
        signals::SPI2,
        handle::SPI2_IRQ as usize,
    );

    runtime::run(&mut backend, handle::WG, handle::SPI2_IRQ, signals::SPI2);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
