// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use ast10x0_peripherals::smc::{FlashConfig, SmcConfig, SmcController};

/// Policy for handling unknown JEDEC IDs at board-integration level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownJedecPolicy {
    /// Reject unknown devices and fail closed.
    StrictReject,
    /// Allow integration-layer fallback using configured geometry.
    ConservativeConfigured,
}

/// Board-level descriptor for AST10x0 SMC flash topology.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ast10x0BoardDescriptor {
    pub controller: SmcController,
    pub cs0: Option<FlashConfig>,
    pub cs1: Option<FlashConfig>,
    pub unknown_jedec_policy: UnknownJedecPolicy,
}

impl Ast10x0BoardDescriptor {
    /// Convert descriptor data into a driver-facing SMC controller config.
    pub const fn smc_config(self) -> SmcConfig {
        SmcConfig {
            controller_id: self.controller,
            cs0: self.cs0,
            cs1: self.cs1,
            dma_enabled: false,
            enable_interrupts: false,
        }
    }

    /// Default descriptor for the AST10x0 QEMU setup (single 1 MiB flash on CS0).
    pub const fn ast10x0_qemu_default() -> Self {
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
        }
    }
}