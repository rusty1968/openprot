// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_flash_client::handle;
use flash_client::FlashClient;
use userspace::entry;
use userspace::syscall;

const MIB: u32 = 1024 * 1024;

struct FlashExpectation {
    controller_id: u32,
    chip_size_bytes: u32,
    cs_count: u32,
}

fn check_exists_and_capacity(
    expectation: &FlashExpectation,
    client: &FlashClient,
) -> Result<(), pw_status::Error> {
    match client.exists() {
        Ok(true) => {
            pw_log::info!(
                "flash exists check passed controller={}",
                expectation.controller_id as u32
            );
        }
        Ok(false) => {
            pw_log::error!(
                "flash exists check reported absent controller={}",
                expectation.controller_id as u32
            );
            return Err(pw_status::Error::Unknown);
        }
        Err(_) => {
            pw_log::error!(
                "flash exists IPC failed controller={}",
                expectation.controller_id as u32
            );
            return Err(pw_status::Error::Internal);
        }
    }

    match client.capacity() {
        Ok(capacity) if capacity == expectation.chip_size_bytes => {
            pw_log::info!(
                "flash capacity check passed controller={} expected_size={} cs_count={}",
                expectation.controller_id as u32,
                expectation.chip_size_bytes as u32,
                expectation.cs_count as u32
            );
            Ok(())
        }
        Ok(capacity) => {
            pw_log::error!(
                "flash capacity mismatch controller={} expected_size={} actual_size={} cs_count={}",
                expectation.controller_id as u32,
                expectation.chip_size_bytes as u32,
                capacity as u32,
                expectation.cs_count as u32
            );
            Err(pw_status::Error::Unknown)
        }
        Err(_) => {
            pw_log::error!(
                "flash capacity IPC failed controller={}",
                expectation.controller_id as u32
            );
            Err(pw_status::Error::Internal)
        }
    }
}

#[entry]
fn entry() -> ! {
    let fmc_expectation = FlashExpectation {
        controller_id: 0,
        chip_size_bytes: 1 * MIB,
        cs_count: 2,
    };
    let spi1_expectation = FlashExpectation {
        controller_id: 1,
        chip_size_bytes: 32 * MIB,
        cs_count: 2,
    };
    let spi2_expectation = FlashExpectation {
        controller_id: 2,
        chip_size_bytes: 32 * MIB,
        cs_count: 2,
    };

    let fmc = FlashClient::new(handle::FLASH_FMC);
    let spi1 = FlashClient::new(handle::FLASH_SPI1);
    let spi2 = FlashClient::new(handle::FLASH_SPI2);

    let status = check_exists_and_capacity(&fmc_expectation, &fmc)
        .and_then(|_| check_exists_and_capacity(&spi1_expectation, &spi1))
        .and_then(|_| check_exists_and_capacity(&spi2_expectation, &spi2));

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
