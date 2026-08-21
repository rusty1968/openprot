// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_main]
#![no_std]

use app_ext_flash_server::handle;
use ast10x0_board::BmcResetGate;
use ast10x0_peripherals::scu::SpiMonitorInstance;
use ast10x0_peripherals::smc::{ChipSelect, FlashConfig, SmcController};
use flash_backend::{Ast10x0SpiExternalFlashDriver, NoWaitBlocking, SpiExternalFlashParams};
use hal_flash::{BlockingFlash, GatedFlash};
use services_flash_server::FlashIpcServer;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;
use util_ipc::IpcHandle;

/// IPC buffer: must hold the largest request/response. Reads are bounded by
/// the client's buffer and this size; 4 KiB of payload + opcode/status headroom.
const IPC_BUF_SIZE: usize = 4352;

/// External BMC flash device profile. 32 MiB (W25Q256-class) with the 256 B
/// program page / 4 KiB erase sector geometry the `FlashDriver` trait assumes.
const EXT_FLASH_CONFIG: FlashConfig = FlashConfig {
    capacity_mb: 32,
    page_size: 256,
    sector_size: 4096,
    block_size: 65536,
    spi_clock_mhz: 50,
};

#[entry]
fn entry() {
    let params = SpiExternalFlashParams {
        controller: SmcController::Spi1,
        cs: ChipSelect::Cs0,
        monitor: SpiMonitorInstance::Spim0,
        config: EXT_FLASH_CONFIG,
    };

    // SAFETY: this process is the sole owner of the SPI1 controller and its
    // read window (mapped in system.json5); the kernel target applied the
    // SPI1/SPIM0 pinmux and passthrough routing before starting any process;
    // no other task programs the SCU internal SPI-master mux; runs once.
    let driver = match unsafe { Ast10x0SpiExternalFlashDriver::new(params) } {
        Ok(d) => d,
        Err(e) => {
            pw_log::error!("ext flash server: SPI1 init failed: {:08x}", e.0.get() as u32);
            let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
            loop {}
        }
    };
    let flash = BlockingFlash {
        driver,
        blocking: NoWaitBlocking,
    };
    // Refuse every bus-accessing op unless the BMC is held in reset. In this
    // image the kernel target asserts that hold; in production the orchestrator
    // owns it around its reset effects.
    let gated = GatedFlash::new(flash, BmcResetGate::prot());
    let mut server = FlashIpcServer::new(gated);
    let mut buf = [0u8; IPC_BUF_SIZE];

    pw_log::info!("ext flash server: ready");
    loop {
        if syscall::object_wait(handle::EXT_FLASH, Signals::READABLE, Instant::MAX).is_err() {
            continue;
        }
        if let Err(e) = server.handle_one(&IpcHandle::new(handle::EXT_FLASH), &mut buf) {
            pw_log::error!("ext flash server: request failed: {:08x}", e.0.get() as u32);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
