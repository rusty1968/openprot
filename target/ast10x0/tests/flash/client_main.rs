// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_flash_client::handle;
use flash_client::FlashClient;
use userspace::entry;
use userspace::syscall;

const PAGE_SIZE: usize = 256;
const FMC_SECTOR_SIZE: u32 = 4096;
// Last 4 KiB sector of the 1 MiB FMC flash. The QEMU m25p80 model is
// blank at boot (the kernel image is loaded into CPU address space, not
// written to the SPI flash backing) so any sector is safe; we use the
// last one as a convention that maps to "off the end of the image" on
// real silicon.
const FMC_TEST_OFFSET: u32 = 0xFF000;

fn check_exists(controller_id: u32, client: &FlashClient) -> Result<(), pw_status::Error> {
    match client.exists() {
        Ok(true) => {
            pw_log::info!(
                "flash exists check passed controller={}",
                controller_id as u32
            );
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
            pw_log::error!(
                "flash exists IPC failed controller={}",
                controller_id as u32
            );
            Err(pw_status::Error::Internal)
        }
    }
}

/// Exercise the mini-BMC FMC self-update flow described in
/// `target/ast10x0/peripherals/smc/planning/use-case.md`:
///
///   1. erase the destination sector,
///   2. verify post-erase sector reads back as 0xFF,
///   3. program a known pattern,
///   4. verify by read-back.
///
/// Identification (JEDEC) is covered separately by [`check_exists`]; the
/// "read existing image" step from the use-case doc is not exercised on
/// QEMU because the m25p80 backing is blank at boot.
fn check_fmc_self_update_flow(client: &FlashClient) -> Result<(), pw_status::Error> {
    client
        .erase(FMC_TEST_OFFSET, FMC_SECTOR_SIZE)
        .map_err(|_| pw_status::Error::Internal)?;
    pw_log::info!("fmc erase passed offset=0x{:x}", FMC_TEST_OFFSET as u32);

    let mut erase_buf = [0u8; PAGE_SIZE];
    let n = client
        .read(FMC_TEST_OFFSET, &mut erase_buf)
        .map_err(|_| pw_status::Error::Internal)?;
    if n != PAGE_SIZE || erase_buf.iter().any(|&b| b != 0xFF) {
        pw_log::error!("fmc post-erase verify failed (n={})", n as u32);
        return Err(pw_status::Error::Unknown);
    }
    pw_log::info!("fmc post-erase verify passed");

    let pattern: [u8; PAGE_SIZE] = core::array::from_fn(|i| (i as u8) ^ 0xA5);
    let n = client
        .write(FMC_TEST_OFFSET, &pattern)
        .map_err(|_| pw_status::Error::Internal)?;
    if n != PAGE_SIZE {
        pw_log::error!("fmc program returned short count (n={})", n as u32);
        return Err(pw_status::Error::Unknown);
    }
    pw_log::info!("fmc program passed");

    let mut readback = [0u8; PAGE_SIZE];
    let n = client
        .read(FMC_TEST_OFFSET, &mut readback)
        .map_err(|_| pw_status::Error::Internal)?;
    if n != PAGE_SIZE || readback != pattern {
        pw_log::error!("fmc readback verify failed (n={})", n as u32);
        return Err(pw_status::Error::Unknown);
    }
    pw_log::info!("fmc self-update flow passed");

    Ok(())
}

#[entry]
fn entry() -> ! {
    let fmc = FlashClient::new(handle::FLASH_FMC);
    let spi1 = FlashClient::new(handle::FLASH_SPI1);
    let spi2 = FlashClient::new(handle::FLASH_SPI2);

    let status = check_exists(0, &fmc)
        .and_then(|_| check_exists(1, &spi1))
        .and_then(|_| check_exists(2, &spi2))
        .and_then(|_| check_fmc_self_update_flow(&fmc));

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
