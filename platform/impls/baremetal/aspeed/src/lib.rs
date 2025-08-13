#![no_std]

//! # Aspeed Platform Implementation
//!
//! This crate provides OpenPRoT HAL trait implementations for Aspeed AST1060
//! system-on-chip hardware. It includes support for cryptographic acceleration
//! through the HACE (Hash and Crypto Engine) peripheral.
//!
//! ## Features
//!
//! - Hardware-accelerated SHA-2 digest operations (SHA-256, SHA-384, SHA-512)
//! - Zero-copy operations with DMA support
//! - OpenPRoT HAL trait implementations for digest operations
//!
//! ## Hardware Support
//!
//! - **AST1060**: Aspeed's ARM Cortex-M4 based SoC
//! - **HACE**: Hardware Hash and Crypto Engine
//!
//! ## Usage
//!
//! ```rust,no_run
//! use openprot_platform_aspeed::hash::{HaceController, HashContext};
//! use openprot_hal_blocking::digest::{DigestInit, DigestOp, Sha2_256};
//!
//! // Initialize hardware controller
//! let mut controller = HaceController::new();
//! 
//! // Start a SHA-256 digest operation
//! let mut context = controller.init(Sha2_256)?;
//! context.update(b"data to hash")?;
//! let digest = context.finalize()?;
//! # Ok::<(), core::convert::Infallible>(())
//! ```

pub mod hash;

// Re-export the most commonly used types
pub use hash::{HaceController, AspeedHashContext, HashAlgo, HashContext};