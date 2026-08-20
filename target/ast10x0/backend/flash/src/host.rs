// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! External BMC/PCH host SPI flash backend.
//!
//! The SPI monitor (`ast10x0_board::spi_monitor`) enforces the host's access
//! policy while the host owns its flash bus. This driver is the complementary
//! path: it lets the RoT take temporary ownership of a monitored SPI1/SPI2 bus
//! and read, erase, or program the host flash itself, as required for PFR
//! verification and recovery.
//!
//! Each operation routes the RoT internal SPI master onto the monitored bus via
//! the SCU internal mux, runs, then tears the route down so the bus returns to
//! host passthrough between operations. The RoT never holds the bus across
//! calls, keeping the host's access window as small as possible.

use core::num::NonZero;

use ast10x0_peripherals::scu::{
    ScuRegisters, SpiMonitorInstance, SpiMonitorSource, SpimGpioOriVal,
};
use ast10x0_peripherals::smc::{
    ChipSelect, FlashConfig, SmcConfig, SmcController, SmcTopology, SpiNorFlash, SpiNorFlashDevice,
    SpiReady, SpiUninit,
};
use hal_flash_driver::{FlashAddress, FlashDriver};
use util_error::{self as error, ErrorCode};
use util_types::PowerOf2Usize;

use crate::{map_smc_error, JedecId, DEFAULT_CONFIG};

/// Parameters describing a monitored host flash to attach.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpiHostFlashParams {
    /// SPI controller wired to the host flash bus (`Spi1` or `Spi2`).
    pub controller: SmcController,
    /// Chip select the flash sits on.
    pub cs: ChipSelect,
    /// SPI monitor path (SPIPF instance) the bus is routed through.
    pub monitor: SpiMonitorInstance,
    /// Device geometry/timing profile.
    pub config: FlashConfig,
}

/// RoT-master bus route held for the duration of one host-flash operation.
///
/// Dropping it restores the SPIM proprietary pin state and clears the internal
/// SPI-master detour, returning the bus to host passthrough.
struct BusRoute<'a> {
    scu: &'a ScuRegisters,
    proprietary: Option<SpimGpioOriVal>,
}

impl Drop for BusRoute<'_> {
    fn drop(&mut self) {
        if let Some(state) = self.proprietary.take() {
            self.scu.spim_proprietary_post_config(state);
        }
        self.scu.clear_spim_internal_master_route();
    }
}

/// Route the RoT internal SPI master onto `source`'s bus through `monitor`.
fn route_bus<'a>(
    scu: &'a ScuRegisters,
    source: SpiMonitorSource,
    monitor: SpiMonitorInstance,
) -> Result<BusRoute<'a>, ErrorCode> {
    scu.set_spim_internal_mux(source, monitor as u8 + 1)
        .map_err(|_| error::FLASH_AST10X0_HARDWARE_ERROR)?;
    let proprietary = scu.spim_proprietary_pre_config();
    Ok(BusRoute { scu, proprietary })
}

/// Map a SPI controller to its monitor source and host bus topology.
fn source_and_topology(
    controller: SmcController,
) -> Result<(SpiMonitorSource, SmcTopology), ErrorCode> {
    match controller {
        SmcController::Spi1 => Ok((
            SpiMonitorSource::Spi1,
            SmcTopology::HostSpi { master_idx: 0 },
        )),
        SmcController::Spi2 => Ok((
            SpiMonitorSource::Spi2,
            SmcTopology::NormalSpi { master_idx: 2 },
        )),
        SmcController::Fmc => Err(error::FLASH_AST10X0_INVALID_CHIP_SELECT),
    }
}

/// Driver for an external BMC/PCH host SPI flash reached through a SPI monitor.
pub struct Ast10x0SpiHostFlashDriver {
    spi: SpiReady,
    scu: ScuRegisters,
    config: FlashConfig,
    cs: ChipSelect,
    source: SpiMonitorSource,
    monitor: SpiMonitorInstance,
}

impl Ast10x0SpiHostFlashDriver {
    /// Attach to a monitored host flash on SPI1 or SPI2.
    ///
    /// `params.config` must share the geometry the `FlashDriver` trait constants
    /// assume (256 B program page, 4 KiB erase sector); profiles that differ are
    /// rejected with `FLASH_AST10X0_DEVICE_NOT_SUPPORTED`. Capacities above
    /// 16 MiB automatically use 4-byte addressing in the SMC device layer.
    ///
    /// # Safety
    /// The calling process must be the sole owner of the selected SPI controller
    /// block and must coordinate access to the shared SCU: this driver programs
    /// the SCU internal SPI-master mux (SCU0F0) around every operation. The SPI
    /// monitor policy for `params.monitor` must not be locked in a way that
    /// blocks the RoT internal master, and the controller pinmux must already be
    /// applied by the kernel target's pre-task init. Call at most once per bus.
    pub unsafe fn new(params: SpiHostFlashParams) -> Result<Self, ErrorCode> {
        let SpiHostFlashParams {
            controller,
            cs,
            monitor,
            config,
        } = params;

        if config.page_size as usize != Self::PROGRAM_WINDOW_SIZE
            || config.sector_size as usize != Self::PAGE_SIZE
        {
            return Err(error::FLASH_AST10X0_DEVICE_NOT_SUPPORTED);
        }

        let (source, topology) = source_and_topology(controller)?;
        let (cs0, cs1) = match cs {
            ChipSelect::Cs0 => (Some(config), None),
            ChipSelect::Cs1 => (None, Some(config)),
        };
        let smc_config = SmcConfig {
            controller_id: controller,
            cs0,
            cs1,
            dma_enabled: false,
            enable_interrupts: false,
            topology,
        };

        // SAFETY: sole ownership of the SPI controller block per the contract above.
        let uninit = unsafe { SpiUninit::new(controller, smc_config) }.map_err(map_smc_error)?;
        let mut spi = uninit.init().map_err(map_smc_error)?;
        spi.spi_nor_read_init(cs).map_err(map_smc_error)?;

        // SAFETY: caller coordinates SCU access per the contract above.
        let scu = unsafe { ScuRegisters::new_global_unlocked() };

        Ok(Self {
            spi,
            scu,
            config,
            cs,
            source,
            monitor,
        })
    }

    /// Read and decode the host flash JEDEC ID (`READ_ID`, `0x9F`).
    ///
    /// Useful for presence detection and identity checks before verify/recovery.
    pub fn read_jedec(&mut self) -> Result<JedecId, ErrorCode> {
        let _route = route_bus(&self.scu, self.source, self.monitor)?;
        SpiNorFlash::from_spi_cs(&mut self.spi, self.config, self.cs)
            .map_err(map_smc_error)?
            .jedec()
            .map_err(map_smc_error)
    }
}

impl FlashDriver for Ast10x0SpiHostFlashDriver {
    type Error = ErrorCode;

    /// Default erase page: one 4 KiB sector.
    const PAGE_SIZE: usize = DEFAULT_CONFIG.sector_size as usize;
    /// SPI NOR program page: writes must not cross a 256-byte boundary.
    const PROGRAM_WINDOW_SIZE: usize = DEFAULT_CONFIG.page_size as usize;
    const MAX_READ_SIZE: usize = 4096;
    const READ_ALIGNMENT: usize = 4;
    const PROGRAM_ALIGNMENT: usize = 1;

    fn size(&self) -> NonZero<usize> {
        NonZero::new(self.config.capacity_mb as usize * 1024 * 1024).unwrap()
    }

    fn erasable_sizes_bitmap(&mut self) -> Result<u32, Self::Error> {
        // Only 4 KiB sector erase is implemented by the peripheral driver.
        Ok(1u32 << self.config.sector_size.trailing_zeros())
    }

    fn read(&mut self, start_addr: FlashAddress, buf: &mut [u8]) -> Result<(), Self::Error> {
        let len = buf.len();
        let _route = route_bus(&self.scu, self.source, self.monitor)?;
        let n = SpiNorFlash::from_spi_cs(&mut self.spi, self.config, self.cs)
            .map_err(map_smc_error)?
            .read(start_addr.offset(), buf)
            .map_err(map_smc_error)?;
        if n != len {
            return Err(error::FLASH_AST10X0_SHORT_READ);
        }
        Ok(())
    }

    fn start_erase(
        &mut self,
        start_addr: FlashAddress,
        size: PowerOf2Usize,
    ) -> Result<(), Self::Error> {
        if size.get() != self.config.sector_size as usize {
            return Err(error::FLASH_GENERIC_ERASE_INVALID_SIZE);
        }
        let _route = route_bus(&self.scu, self.source, self.monitor)?;
        // Blocks until the device's WIP bit clears inside the peripheral driver.
        SpiNorFlash::from_spi_cs(&mut self.spi, self.config, self.cs)
            .map_err(map_smc_error)?
            .erase_sector(start_addr.offset())
            .map_err(map_smc_error)
    }

    fn start_program(&mut self, start_addr: FlashAddress, data: &[u8]) -> Result<(), Self::Error> {
        let _route = route_bus(&self.scu, self.source, self.monitor)?;
        // Blocks until the device's WIP bit clears inside the peripheral driver.
        SpiNorFlash::from_spi_cs(&mut self.spi, self.config, self.cs)
            .map_err(map_smc_error)?
            .program_page(start_addr.offset(), data)
            .map_err(map_smc_error)?;
        Ok(())
    }

    fn is_busy(&mut self) -> bool {
        false
    }

    fn complete_op(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
