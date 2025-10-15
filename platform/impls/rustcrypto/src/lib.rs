// Licensed under the Apache-2.0 license

//! RustCrypto implementation of OpenPRoT HAL traits
//!
//! This crate provides implementations of OpenPRoT HAL traits using the
//! RustCrypto ecosystem, offering high-quality, audited cryptographic
//! implementations for embedded and general-purpose use.
//!
//! # Features

#![no_std]

pub mod cipher;
pub mod digest;

// Re-export commonly used ECDSA types

// Re-export commonly used cipher types
pub use cipher::{Aes256CtrCipher, Aes256GcmCipher, RustCryptoCipherError};

// Re-export commonly used digest types
pub use digest::Software as SoftwareDigest;
pub use digest::{OwnedDigestController, OwnedHasher};
