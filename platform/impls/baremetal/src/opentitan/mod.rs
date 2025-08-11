//! # OpenTitan RISC-V Platform Implementation
//!
//! This module provides OpenTitan-specific implementations of the OpenPRoT HAL traits.
//! OpenTitan is an open-source silicon root of trust platform built around the Ibex
//! RISC-V processor core (RV32IMC architecture).
//!
//! ## Hardware Platform
//!
//! - **Architecture**: RISC-V RV32IMC
//! - **Core**: Ibex (lowRISC implementation)
//! - **Target Triple**: `riscv32imc-unknown-none-elf`
//! - **Memory**: On-chip SRAM, flash storage
//! - **Security**: Hardware root of trust, secure boot
//!
//! ## Cryptographic Hardware Features
//!
//! - **HMAC Engine**: Hardware-accelerated HMAC with SHA-2 support (SHA-256/384/512)
//! - **KMAC Engine**: Hardware-accelerated KMAC with SHA-3/Keccak support
//! - **AES Engine**: Hardware AES encryption/decryption
//! - **OTBN**: OpenTitan Big Number accelerator for public key cryptography
//! - **CSRNG**: Cryptographically Secure Random Number Generator
//! - **Key Manager**: Secure key derivation and storage
//! - **Flash Controller**: Secure flash memory management
//!
//! ## Implementation Strategy
//!
//! The implementations in this module serve as adapters between the OpenPRoT HAL
//! traits and the existing OpenTitan hardware drivers. This allows:
//!
//! 1. **Code Reuse**: Leverage existing, tested OpenTitan drivers
//! 2. **Type Safety**: Maintain compile-time guarantees about algorithm support
//! 3. **Performance**: Direct hardware access without abstraction overhead
//! 4. **Maintainability**: Changes to OpenTitan drivers are isolated here
//! 5. **RISC-V Optimization**: Take advantage of RISC-V ISA features
//!
//! ## Module Organization
//!
//! - [`hmac`] - HMAC hardware engine integration (SHA-2 family)
//! - [`kmac`] - KMAC hardware engine integration (SHA-3/Keccak family)

pub mod hmac;
pub mod kmac;

// Re-export the main device types for convenience
pub use hmac::Hmac;
pub use kmac::Kmac;
