// Licensed under the Apache-2.0 license

//! Platform-specific traits for OpenPRoT
//!
//! This crate provides a unified interface to platform-specific cryptographic
//! traits for different operating systems and embedded platforms.
//!
//! # Platform Support
//!
//! - **Hubris**: Microkernel OS with task isolation and IDL compatibility
//! - **Tock**: Embedded OS with async support and capsule integration  
//! - **Linux**: Full-featured OS with kernel crypto API and HSM support
//!
//! # Usage
//!
//! Choose the appropriate platform-specific traits for your target environment:
//!
//! ```rust
//! // For Hubris applications
//! use openprot_platform_traits::hubris::{HubrisDigestDevice, HubrisCryptoError};
//!
//! // For Tock applications  
//! use openprot_platform_traits::tock::{TockDigestDevice, TockCryptoError};
//!
//! // For Linux applications
//! use openprot_platform_traits::linux::{LinuxDigestDevice, LinuxCryptoError};
//! ```
//!
//! # Design Philosophy
//!
//! Instead of using complex generic trait bounds that don't work well with
//! IDL code generation and embedded constraints, these platform-specific
//! traits provide:
//!
//! - **Concrete Types**: All associated types are concrete for better compatibility
//! - **OS Integration**: Each trait is tailored to its target platform's strengths
//! - **Zero Cost**: No runtime overhead compared to direct implementations
//! - **Type Safety**: Eliminates need for unsafe code in most cases

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// Hubris OS-specific traits
///
/// Provides traits specifically designed for the Hubris microkernel,
/// including task isolation, IDL compatibility, and embedded constraints.
pub mod hubris {
    pub use openprot_platform_traits_hubris::*;
}

/// Tock OS-specific traits
///
/// Provides traits specifically designed for the Tock embedded OS,
/// including async operation support, capsule integration, and grant region management.
pub mod tock {
    pub use openprot_platform_traits_tock::*;
}

/// Linux-specific traits
///
/// Provides traits for Linux environments, including support for kernel crypto APIs,
/// hardware security modules, and userspace crypto libraries with thread safety.
pub mod linux {
    pub use openprot_platform_traits_linux::*;
}

/// Common trait aliases for easier migration
///
/// These aliases help with migrating from generic traits to platform-specific ones.
pub mod common {
    /// Re-export the most commonly used platform trait based on target
    #[cfg(target_os = "none")]
    pub use crate::hubris::*;
    
    #[cfg(target_os = "linux")]
    pub use crate::linux::*;
    
    // Note: Tock doesn't have a standard target_os, so it needs explicit selection
}
