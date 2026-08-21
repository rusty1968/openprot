// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! External-flash IPC client smoke test.
//!
//! This test is intentionally READ-ONLY: it exercises the connect/geometry
//! handshake and a read against the live BMC host flash without erasing or
//! programming it. Bringing the RoT master onto the host bus is already the
//! interesting part; destructive coverage belongs on a dedicated part, not the
//! platform's real BMC flash.

#![no_main]
#![no_std]

use app_ext_flash_client::handle;
use hal_flash::{Flash, FlashAddress};
use services_flash_client::FlashIpcClient;
use userspace::entry;
use userspace::syscall;
use util_ipc::IpcHandle;

/// Matches the server's `EXT_FLASH_CONFIG` (32 MiB, 4 KiB erase sector).
const TOTAL: usize = 32 * 1024 * 1024;
const SECTOR: usize = 4096;

fn fail(msg: &str) -> ! {
    pw_log::error!("ext flash client FAIL: {}", msg as &str);
    let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
    loop {}
}

#[entry]
fn entry() {
    let mut flash = match FlashIpcClient::new(IpcHandle::new(handle::EXT_FLASH)) {
        Ok(c) => c,
        Err(_) => fail("connect/geometry"),
    };

    // Geometry matches the server's external-flash config.
    let Ok((total, page, bitmap)) = flash.geometry() else {
        fail("geometry");
    };
    if total.get() != TOTAL {
        fail("total size");
    }
    if page.get() != SECTOR {
        fail("page size");
    }
    if bitmap != 1 << 12 {
        fail("erase bitmap");
    }

    // Non-destructive read: pull the first 64 bytes off the host flash. Success
    // proves the RoT-master route, SPI1 init, and read path all work end to end.
    let mut buf = [0u8; 64];
    if flash.read(FlashAddress::new(0), &mut buf).is_err() {
        fail("read");
    }

    pw_log::info!("ext flash client: read OK, first byte 0x{:02x}", buf[0] as u32);
    let _ = syscall::debug_shutdown(Ok(()));
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
