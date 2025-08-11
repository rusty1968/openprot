//! # OpenTitan Platform Implementation
//!
//! This crate provides OpenTitan-specific implementations of the OpenPRoT HAL traits.
//! It bridges the gap between the generic HAL interfaces and the OpenTitan hardware
//! peripherals and drivers.
//!
//! ## Features
//!
//! - **HMAC**: Hardware-accelerated HMAC using OpenTitan's HMAC peripheral
//! - **KMAC**: Hardware-accelerated KMAC using OpenTitan's KMAC peripheral  
//! - **AES**: Hardware-accelerated AES encryption/decryption (optional)
//! - **RSA**: Hardware-accelerated RSA operations (optional)
//!
//! ## Usage
//!
//! ```rust,no_run
//! use openprot_platform_opentitan::hmac::HmacDevice;
//! use openprot_hal_blocking::digest::{DigestInit, DigestOp, Sha2_256};
//!
//! let mut hmac = HmacDevice::new();
//! let mut hasher = hmac.init(Sha2_256)?;
//! hasher.update(b"hello world")?;
//! let digest = hasher.finalize()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![no_std]

#[cfg(feature = "hmac")]
pub mod hmac;

#[cfg(feature = "kmac")]
pub mod kmac;

#[cfg(feature = "aes")]
pub mod aes;

#[cfg(feature = "rsa")]
pub mod rsa;

// Re-export common types for convenience
pub use openprot_hal_blocking::digest::{
    Digest, DigestAlgorithm, ErrorKind, Error, ErrorType,
    DigestInit, DigestOp, DigestCtrlReset,
    Sha2_256, Sha2_384, Sha2_512,
    Sha3_224, Sha3_256, Sha3_384, Sha3_512,
};
