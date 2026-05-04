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

    /// Dual-CS variant of the FMC default. CS1 mirrors CS0's geometry; on
    /// QEMU CS1 is unconnected so JEDEC reads return `0xFF` and writes
    /// surface as `IoError`. The descriptor itself is environment-agnostic
    /// — clients keyed off CS1 see a coherent device whose physical state
    /// depends on the board.
    pub fn ast10x0_qemu_default_dual_cs() -> Self {
        let cs0 = FlashConfig {
            capacity_mb: 1,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 25,
        };
        Self {
            controller: SmcController::Fmc,
            cs0: Some(cs0),
            cs1: Some(cs0),
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: None,
            monitor_policy: MonitorPolicy::empty(),
        }
    }

    /// Dual-CS variant of the SPI1 default. Both CSes use the W25Q256
    /// geometry; the SPIPF policy and lock are unchanged.
    pub fn ast10x0_qemu_default_spi1_dual_cs() -> Self {
        Self {
            controller: SmcController::Spi1,
            cs0: Some(FlashConfig::winbond_w25q256()),
            cs1: Some(FlashConfig::winbond_w25q256()),
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: Some(SpimWiring::default_spi1_via_spim0()),
            monitor_policy: presets::bmc_default_policy(),
        }
    }

    /// Dual-CS variant of the SPI2 default.
    pub fn ast10x0_qemu_default_spi2_dual_cs() -> Self {
        Self {
            controller: SmcController::Spi2,
            cs0: Some(FlashConfig::winbond_w25q256()),
            cs1: Some(FlashConfig::winbond_w25q256()),
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: Some(SpimWiring::default_spi2_via_spim2()),
            monitor_policy: presets::bmc_default_policy(),
        }
    }

    // ── AST1060 EVB descriptors derived from aspeed-rust ──────────────────
    //
    // The constructors below transcribe the per-controller constants from
    // `aspeed-rust/src/spi/spitest.rs` (FMC_CONFIG / SPI0_CONFIG /
    // SPI1_CONFIG plus FMC_CS*_CAPACITY / SPI_CS*_CAPACITY). Mapping
    // (already documented in `default_spi1_via_spim0` / `_spi2_via_spim2`
    // above): aspeed `SPI0` ≡ smc-work `Spi1`, aspeed `SPI1` ≡ smc-work
    // `Spi2`.
    //
    // Caveats — read before flashing real silicon:
    // 1. **Capacity is the test ceiling, not chip read.** SPI controllers
    //    use 64 MB (`SPI_CS*_CAPACITY = 0x0400_0000`), which sized
    //    aspeed-rust's tests; the actual EVB part may be smaller. Confirm
    //    against the schematic / JEDEC ID before trusting `info()`
    //    capacity. The Macronix MX25L8006E reference at spitest.rs:64 is
    //    1 MB and is just where page/sector defaults come from — not the
    //    SPI part.
    // 2. **Pinctrl is NOT carried.** aspeed-rust applies
    //    `PINCTRL_FMC_QUAD` / `PINCTRL_SPIM0_QUAD_DEFAULT` /
    //    `PINCTRL_SPI1_QUAD` / `PINCTRL_SPIM2_PINCTRL0` /
    //    `PINCTRL_SPIM3_PINCTRL0` / `PINCTRL_SPI2_QUAD` (SCU414/418
    //    multi-function pin programming) at controller init. Our
    //    `SpimWiring` only covers SCU0F0 routing. The kernel target's
    //    `main()` is responsible for the multi-function pin programming
    //    before any flash server starts.
    // 3. **Quad pins are committed, quad transfer modes are not wired.**
    //    Even with quad pinctrl applied externally, OpenPRoT today does
    //    not program quad-IO into the CS control register (parity-gaps
    //    §D3). AHB reads will run at 1-1-1 regardless of pin width until
    //    D3 lands.
    // 4. **`master_idx` / `ctrl_type` are not modeled.** aspeed-rust
    //    uses these to gate timing-calibration skipping and SPIM
    //    bracketing (parity-gaps §B11). The constructors here pick the
    //    `_via_spimN` wiring corresponding to aspeed-rust's choice but
    //    cannot encode the calibration-skip rule.
    // 5. **Target frequency, not measured bus speed.** aspeed-rust's
    //    50 MHz target feeds `spi_freq_div`, which picks the closest
    //    divisor *below* target. With HCLK currently hard-coded to
    //    200 MHz in OpenPRoT (parity-gaps §D7), the actual SCK rate
    //    will only match silicon when HCLK is also 200 MHz.
    //
    // Geometry shared by all three controllers below (Macronix MX25L8006E
    // and equivalents — see `aspeed-rust/src/spi/spitest.rs:64-65`):
    //   page_size = 256, sector_size = 4096, block_size = 65536.
    // Target SCK = 50 MHz (aspeed-rust spitest.rs:87/100/113).

    /// AST1060 EVB FMC: 1 MB CS0 + 1 MB CS1 at 50 MHz target,
    /// `BootSpi` / `master_idx = 0`.
    /// Source: `aspeed-rust/src/spi/spitest.rs:79-90`,
    /// `FMC_CS0_CAPACITY` / `FMC_CS1_CAPACITY` at lines 58-59.
    pub fn ast1060_evb_fmc_aspeed_rust_derived() -> Self {
        let cfg = FlashConfig {
            capacity_mb: 1,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 50,
        };
        Self {
            controller: SmcController::Fmc,
            cs0: Some(cfg),
            cs1: Some(cfg),
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: None,
            monitor_policy: MonitorPolicy::empty(),
        }
    }

    /// AST1060 EVB SPI1 (aspeed SPI0): 64 MB CS0 routed through SPIM0,
    /// 50 MHz target, `HostSpi` / `master_idx = 0`.
    /// Source: `aspeed-rust/src/spi/spitest.rs:92-103, 60`. Pinctrl
    /// (`PINCTRL_SPIM0_QUAD_DEFAULT` + `PINCTRL_SPI1_QUAD`) and the
    /// SCU0F0 = `0x0000_fff0` write at spitest.rs:417 must be programmed
    /// by the kernel target before any user-space flash server runs.
    pub fn ast1060_evb_spi1_aspeed_rust_derived() -> Self {
        let cfg = FlashConfig {
            capacity_mb: 64,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 50,
        };
        Self {
            controller: SmcController::Spi1,
            cs0: Some(cfg),
            cs1: None,
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: Some(SpimWiring::default_spi1_via_spim0()),
            monitor_policy: presets::bmc_default_policy(),
        }
    }

    /// AST1060 EVB SPI2 (aspeed SPI1): 64 MB CS0 + CS1 routed through
    /// SPIM2, 50 MHz target, `NormalSpi` / `master_idx = 2`.
    /// Source: `aspeed-rust/src/spi/spitest.rs:105-116, 60-61`. aspeed-rust
    /// re-routes CS1 through SPIM3 mid-test (spitest.rs:710-714); that
    /// per-transaction reroute is incompatible with OpenPRoT's lock-once
    /// SPIPF model (parity-gaps §B10). The descriptor here keeps both
    /// CSes on SPIM2; CS1 access requires a separate descriptor or a
    /// reworked SPIPF flow. Kernel pinctrl groups required:
    /// `PINCTRL_SPIM2_PINCTRL0`, `PINCTRL_SPIM3_PINCTRL0`,
    /// `PINCTRL_SPI2_QUAD`.
    /// **`timing_calibration_disabled = false` in aspeed-rust** — the
    /// only controller that enables calibration. Until parity-gaps §D9
    /// lands, this descriptor relies on POR timing.
    pub fn ast1060_evb_spi2_aspeed_rust_derived() -> Self {
        let cfg = FlashConfig {
            capacity_mb: 64,
            page_size: 256,
            sector_size: 4096,
            block_size: 65536,
            spi_clock_mhz: 50,
        };
        Self {
            controller: SmcController::Spi2,
            cs0: Some(cfg),
            cs1: Some(cfg),
            unknown_jedec_policy: UnknownJedecPolicy::StrictReject,
            spim_wiring: Some(SpimWiring::default_spi2_via_spim2()),
            monitor_policy: presets::bmc_default_policy(),
        }
    }
}