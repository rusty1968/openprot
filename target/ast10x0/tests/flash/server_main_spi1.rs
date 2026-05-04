// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_flash_server_spi1::{handle, signals};
use flash_backend::{Backend, Cs as ChipSelect};
use flash_server::runtime::{self, ChannelBinding};
use userspace::entry;
use userspace::syscall::{self, Signals};

#[entry]
fn entry() -> ! {
    let Ok(mut backend) = Backend::new_spi1_pre_wired() else {
        // Init failed at boot. No recovery path for a flash server without
        // a flash; halt the userspace task. `loop {}` is used (not panic!)
        // to satisfy the project's no_panics_test on this binary.
        loop {}
    };

    let _ = syscall::wait_group_add(
        handle::WG,
        handle::FLASH,
        Signals::READABLE,
        handle::FLASH as usize,
    );
    let _ = syscall::wait_group_add(
        handle::WG,
        handle::SPI1_IRQ,
        signals::SPI1,
        handle::SPI1_IRQ as usize,
    );

    let channels = [ChannelBinding {
        handle: handle::FLASH,
        key: ChipSelect::Cs0,
    }];
    runtime::run_routed(
        &mut backend,
        handle::WG,
        &channels,
        handle::SPI1_IRQ,
        signals::SPI1,
    );
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
