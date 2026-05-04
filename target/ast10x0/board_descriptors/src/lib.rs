// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use ast10x0_peripherals::smc::{FlashConfig, SmcConfig, SmcController};
use ast10x0_peripherals::spimonitor::MonitorPolicy;

pub mod spim_wiring;

pub use spim_wiring::{
    apply_spim_wiring, presets, SpimWiring, SpimWiringError,
};

/// Policy for handling unknown JEDEC IDs at board-integration level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownJedecPolicy {
    /// Reject unknown devices and fail closed.
    StrictReject,
    /// Allow integration-layer fallback using configured geometry.
    ConservativeConfigured,
}

/// Board-level descriptor for AST10x0 SMC flash topology.
///
/// Not `Eq`/`PartialEq` because the embedded `MonitorPolicy` is not (yet);
/// callers compare individual fields if they need equality.
#[derive(Clone, Debug)]
pub struct Ast10x0BoardDescriptor {
    pub controller: SmcController,
    pub cs0: Option<FlashConfig>,
    pub cs1: Option<FlashConfig>,
    pub unknown_jedec_policy: UnknownJedecPolicy,
    /// SPIM routing for SPI controllers. Must be `None` for FMC and
    /// `Some(_)` for Spi1/Spi2.
    pub spim_wiring: Option<SpimWiring>,
    /// SPIPF policy programmed and locked when `spim_wiring` is `Some(_)`.
    /// Ignored for FMC descriptors.
    pub monitor_policy: MonitorPolicy,
}

impl Ast10x0BoardDescriptor {
    /// Convert descriptor data into a driver-facing SMC controller config.
    pub fn smc_config(&self) -> SmcConfig {
        SmcConfig {
            controller_id: self.controller,
            cs0: self.cs0,
            cs1: self.cs1,
            dma_enabled: false,
            enable_interrupts: false,
        }
    }

    /// Default descriptor for the AST10x0 QEMU setup (single 1 MiB flash on
    /// CS0 of the FMC controller). FMC has no SPIM path.
    pub fn ast10x0_qemu_default() -> Self {
        Self {
            controller: SmcController::Fmc,
            cs0: Some(FlashConfig {
                capacity_mb: 1,
                page_size: 256,
                sector_size: 4096,
                block_size: 65536,
                spi_clock_mhz: 25,
            }),
            cs1: None,
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: None,
            monitor_policy: MonitorPolicy::empty(),
        }
    }

    /// Default descriptor for SPI1 (aspeed-rust SPI0) wired through SPIM0
    /// with the BMC default opcode allow-list policy.
    pub fn ast10x0_qemu_default_spi1() -> Self {
        Self {
            controller: SmcController::Spi1,
            cs0: Some(FlashConfig::winbond_w25q256()),
            cs1: None,
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: Some(SpimWiring::default_spi1_via_spim0()),
            monitor_policy: presets::bmc_default_policy(),
        }
    }

    /// Default descriptor for SPI2 (aspeed-rust SPI1) wired through SPIM2
    /// with the BMC default opcode allow-list policy.
    pub fn ast10x0_qemu_default_spi2() -> Self {
        Self {
            controller: SmcController::Spi2,
            cs0: Some(FlashConfig::winbond_w25q256()),
            cs1: None,
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: Some(SpimWiring::default_spi2_via_spim2()),
            monitor_policy: presets::bmc_default_policy(),
        }
    }
}