//! # Baremetal Platform Implementations
//!
//! This crate provides baremetal/embedded platform implementations of the OpenPRoT
//! HAL traits. It includes support for various embedded platforms and microcontrollers.
//!
//! ## Supported Platforms
//!
//! - **OpenTitan**: Open-source silicon root of trust platform
//!   - **Architecture**: RISC-V RV32IMC (Ibex core)
//!   - **Target**: `riscv32imc-unknown-none-elf`
//!   - **Hardware**: HMAC, KMAC, Hardware RNG, Key Manager
//!   - **Features**: Hardware acceleration for cryptographic operations
//!
//! ## Feature Flags
//!
//! - `opentitan`: Enable OpenTitan RISC-V platform implementations
//!
//! ## Architecture
//!
//! The implementations are organized by target platform:
//!
//! ```text
//! baremetal/
//! ├── opentitan/     # OpenTitan RISC-V (Ibex RV32IMC) implementations
//! │   ├── hmac.rs    # HMAC hardware driver integration
//! │   ├── kmac.rs    # KMAC hardware driver integration
//! │   └── mod.rs     # Platform module
//! └── lib.rs         # Main library entry point
//! ```
//!
//! ## OpenTitan RISC-V Target
//!
//! When targeting OpenTitan hardware, ensure your Rust toolchain supports:
//! - Target: `riscv32imc-unknown-none-elf`
//! - ISA: RV32IMC (Integer + Multiplication/Division + Compressed)
//! - Core: Ibex (lowRISC implementation)
//! - Environment: Bare metal (no_std, no_main)
//!
//! ## Usage
//!
//! ```rust,no_run
//! #[cfg(feature = "opentitan")]
//! use openprot_platform_baremetal::opentitan::HmacDevice;
//! use openprot_hal_blocking::digest::{DigestInit, Sha2_256};
//!
//! #[cfg(feature = "opentitan")]
//! fn example() -> Result<(), Box<dyn core::error::Error>> {
//!     let mut hmac = HmacDevice::new();
//!     let mut hasher = hmac.init(Sha2_256)?;
//!     hasher.update(b"hello world")?;
//!     let digest = hasher.finalize()?;
//!     Ok(())
//! }
//! ```

#![no_std]
#![deny(missing_docs)]

#[cfg(feature = "opentitan")]
pub mod opentitan;

// Re-export commonly used types for convenience
pub use openprot_hal_blocking::digest;
