#![no_std]

//! # Aspeed Platform Implementation
//!
//! This crate provides OpenPRoT HAL trait implementations for Aspeed AST1060
//! system-on-chip hardware. It includes support for cryptographic acceleration
//! through the HACE (Hash and Crypto Engine) peripheral and system control
//! functionality for clock and reset management.
//!
//! ## Features
//!
//! - Hardware-accelerated SHA-2 digest operations (SHA-256, SHA-384, SHA-512)
//! - System controller for clock and reset management
//! - Zero-copy operations with DMA support
//! - OpenPRoT HAL trait implementations for digest operations
//!
//! ## Hardware Support
//!
//! - **AST1060**: Aspeed's ARM Cortex-M4 based SoC
//! - **HACE**: Hardware Hash and Crypto Engine
//! - **SCU**: System Control Unit for clock/reset management
//!
//! ## Usage
//!
//! ### Cryptographic Operations
//!
//! ```rust,no_run
//! use openprot_platform_aspeed::hash::{HaceController, HashContext};
//! use openprot_platform_aspeed::syscon::{SysCon, ClockId, ResetId};
//! use openprot_hal_blocking::digest::{DigestInit, DigestOp, Sha2_256};
//! use ast1060_pac::Peripherals;
//!
//! // Get peripheral access
//! let peripherals = unsafe { Peripherals::steal() };
//!
//! // Initialize system controller and enable HACE
//! let mut syscon = SysCon::new(peripherals.scu);
//! syscon.init_hace()?;
//!
//! // Initialize hardware controller
//! let mut controller = HaceController::new(peripherals.hace);
//! 
//! // Start a SHA-256 digest operation
//! let mut context = controller.init(Sha2_256)?;
//! context.update(b"data to hash")?;
//! let digest = context.finalize()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ### System Control
//!
//! ```rust,no_run
//! use openprot_platform_aspeed::syscon::{SysCon, ClockId, ResetId};
//! use ast1060_pac::Peripherals;
//!
//! let peripherals = unsafe { Peripherals::steal() };
//! let mut syscon = SysCon::new(peripherals.scu);
//!
//! // Enable individual clocks
//! syscon.enable_clock(ClockId::ClkYCLK)?;    // HACE clock
//! syscon.enable_clock(ClockId::ClkRSACLK)?;  // RSA/ECC clock
//!
//! // Control resets
//! syscon.reset_deassert(ResetId::RstHACE)?;  // Bring HACE out of reset
//! # Ok::<(), openprot_platform_aspeed::syscon::Error>(())
//! ```

pub mod hash;
pub mod syscon;

// Re-export the most commonly used types
pub use hash::{HaceController, AspeedHashContext, HashAlgo, HashContext};
pub use syscon::{SysCon, ClockId, ResetId, Error as SysconError, ClockConfig};