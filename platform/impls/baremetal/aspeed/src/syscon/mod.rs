//! System Controller (SYSCON) implementation for AST1060
//!
//! This module provides a comprehensive system control implementation based on
//! the aspeed-rust project, with OpenPRoT HAL trait implementations separated
//! for clean architecture.
//!
//! ## Modules
//! 
//! - [`syscon`] - Core system controller implementation (hardware abstraction)
//! - [`syscon_traits`] - OpenPRoT HAL trait implementations
//!
//! ## Re-exports
//! 
//! This module re-exports the key types and functions from both modules for
//! easy access by consumers.

pub mod syscon;
pub mod syscon_traits;

// Re-export key types from base module
pub use syscon::{
    SysCon, Error, ClockId, ResetId, ClockConfig, 
    I3CClkSource, HCLKSource, HPLL_FREQ, mhz
};

// The trait implementations are automatically available when you import the types
