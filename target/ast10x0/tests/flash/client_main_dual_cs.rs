// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_flash_client_dual_cs::handle;
use flash_client::{ClientError, FlashClient};
use userspace::entry;
use userspace::syscall;

const PAGE_SIZE: usize = 256;
const FMC_SECTOR_SIZE: u32 = 4096;
const FMC_TEST_OFFSET: u32 = 0xFF000;
const CS_CAPACITY: u32 = 1024 * 1024;

fn check_per_cs_capacity(cs0: &FlashClient, cs1: &FlashClient) -> Result<(), pw_status::Error> {
    let cs0_cap = cs0.capacity().map_err(|_| pw_status::Error::Internal)?;
    let cs1_cap = cs1.capacity().map_err(|_| pw_status::Error::Internal)?;
    if cs0_cap != CS_CAPACITY || cs1_cap != CS_CAPACITY {
        pw_log::error!(
            "dual_cs capacity mismatch cs0={} cs1={} (expected {})",
            cs0_cap as u32,
            cs1_cap as u32,
            CS_CAPACITY as u32
        );
        return Err(pw_status::Error::Unknown);
    }
    pw_log::info!(
        "dual_cs per-cs capacity ok cs0={} cs1={}",
        cs0_cap as u32,
        cs1_cap as u32
    );
    Ok(())
}

fn check_cs0_exists(cs0: &FlashClient) -> Result<(), pw_status::Error> {
    match cs0.exists() {
        Ok(true) => {
            pw_log::info!("dual_cs cs0 exists ok");
            Ok(())
        }
        Ok(false) => {
            pw_log::error!("dual_cs cs0 reported absent");
            Err(pw_status::Error::Unknown)
        }
        Err(_) => {
            pw_log::error!("dual_cs cs0 exists IPC failed");
            Err(pw_status::Error::Internal)
        }
    }
}

fn check_cs1_absent_on_qemu(cs1: &FlashClient) -> Result<(), pw_status::Error> {
    // QEMU's m25p80 model is on CS0 only; CS1 is unconnected so its JEDEC
    // ID reads back as `[0xFF, 0xFF, 0xFF]` and the backend reports the
    // device as absent. Locking the contract against the observed value
    // catches any silent misrouting onto CS0.
    match cs1.exists() {
        Ok(false) => {
            pw_log::info!("dual_cs cs1 reported absent on qemu (expected)");
            Ok(())
        }
        Ok(true) => {
            pw_log::error!("dual_cs cs1 reported present on qemu (unexpected)");
            Err(pw_status::Error::Unknown)
        }
        Err(_) => {
            pw_log::error!("dual_cs cs1 exists IPC failed");
            Err(pw_status::Error::Internal)
        }
    }
}

fn check_cs0_round_trip(cs0: &FlashClient) -> Result<(), pw_status::Error> {
    cs0.erase(FMC_TEST_OFFSET, FMC_SECTOR_SIZE)
        .map_err(|_| pw_status::Error::Internal)?;

    let pattern: [u8; PAGE_SIZE] = core::array::from_fn(|i| (i as u8) ^ 0x5A);
    let n = cs0
        .write(FMC_TEST_OFFSET, &pattern)
        .map_err(|_| pw_status::Error::Internal)?;
    if n != PAGE_SIZE {
        pw_log::error!("dual_cs cs0 program short count (n={})", n as u32);
        return Err(pw_status::Error::Unknown);
    }

    let mut readback = [0u8; PAGE_SIZE];
    let n = cs0
        .read(FMC_TEST_OFFSET, &mut readback)
        .map_err(|_| pw_status::Error::Internal)?;
    if n != PAGE_SIZE || readback != pattern {
        pw_log::error!("dual_cs cs0 readback mismatch (n={})", n as u32);
        return Err(pw_status::Error::Unknown);
    }
    pw_log::info!("dual_cs cs0 round-trip passed");
    Ok(())
}

fn check_cs1_isolated_from_cs0(cs1: &FlashClient) -> Result<(), pw_status::Error> {
    // After cs0 has been written, cs1.exists() must still report absent on
    // QEMU — i.e. there is no cross-channel contamination. Without channel
    // routing this would be the same physical CS and the JEDEC probe would
    // succeed.
    match cs1.exists() {
        Ok(false) => {
            pw_log::info!("dual_cs cs1 still absent after cs0 traffic (no leak)");
            Ok(())
        }
        Ok(true) => {
            pw_log::error!("dual_cs cs1 unexpectedly present after cs0 traffic");
            Err(pw_status::Error::Unknown)
        }
        Err(_) => Err(pw_status::Error::Internal),
    }
}

fn check_cs1_writes_fail_on_qemu(cs1: &FlashClient) -> Result<(), pw_status::Error> {
    // Programming an unconnected CS on QEMU surfaces as a backend-side
    // error (the verify-after-write or the program loop fails). The
    // exact error code isn't guaranteed across QEMU versions; we just
    // require it not to silently succeed.
    let pattern = [0xA5u8; PAGE_SIZE];
    match cs1.write(FMC_TEST_OFFSET, &pattern) {
        Ok(_) => {
            pw_log::error!("dual_cs cs1 program unexpectedly succeeded on qemu");
            Err(pw_status::Error::Unknown)
        }
        Err(ClientError::ServerError(_)) | Err(ClientError::IpcError(_)) => {
            pw_log::info!("dual_cs cs1 program failed as expected on qemu");
            Ok(())
        }
        Err(_) => Err(pw_status::Error::Internal),
    }
}

#[entry]
fn entry() -> ! {
    let cs0 = FlashClient::new(handle::FLASH_CS0);
    let cs1 = FlashClient::new(handle::FLASH_CS1);

    // Order matters: poking CS1 on QEMU (where it is unconnected) can leave
    // the shared m25p80 model in a state that times out subsequent CS0 ops,
    // so do the CS0 round-trip before any CS1 traffic.
    let status = check_per_cs_capacity(&cs0, &cs1)
        .and_then(|_| check_cs0_exists(&cs0))
        .and_then(|_| check_cs0_round_trip(&cs0))
        .and_then(|_| check_cs1_absent_on_qemu(&cs1))
        .and_then(|_| check_cs1_isolated_from_cs0(&cs1))
        .and_then(|_| check_cs1_writes_fail_on_qemu(&cs1));

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
