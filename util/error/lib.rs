// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Precise error handling for firmware across the project.
//!
//! This crate provides a unified error type that encodes errors as u32 values with bitfield structure.
//! This allows for global uniqueness, traceability, and reduced memory footprint in embedded systems.
//!
//! # Error Structure
//!
//! A u32 error value is subdivided into:
//! - Bits 31-30: Error category (status code)
//! - Bits 29-16: Module identifier
//! - Bits 15-0: Error code within the module
//!
//! # Example
//!
//! ```no_run
//! use util_error::{Error, ErrorModule};
//!
//! pub const ERR_ECDSA: ErrorModule = ErrorModule::new(0x1234);
//!
//! pub const ERR_ECDSA_BUSY: Error = ERR_ECDSA.error(1);
//! pub const ERR_ECDSA_INVALID_SIGNATURE: Error = ERR_ECDSA.error(2);
//! ```

#![no_std]
#![cfg_attr(feature = "defmt", derive(defmt::Format))]

use core::num::NonZeroU32;

/// Represents a unified error value.
///
/// The error is encoded as a NonZeroU32 with bitfield structure that includes
/// status code, module identifier, and error code. The zero value is reserved
/// to represent success (no error).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Error(NonZeroU32);

/// Represents an error module identifier and status code prefix.
///
/// This provides a namespace for error codes within a specific module.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ErrorModule(u32);

impl Error {
    /// Creates a new error from a raw u32 value at compile time.
    ///
    /// # Panics
    /// Panics if value is 0 (reserved for success).
    pub const fn new_const(val: u32) -> Self {
        match NonZeroU32::new(val) {
            Some(val) => Error(val),
            None => panic!("Error cannot be 0 (reserved for success)"),
        }
    }

    /// Creates a new error from a raw u32 value.
    ///
    /// # Panics
    /// Panics if value is 0 (reserved for success).
    pub const fn from_raw(value: u32) -> Self {
        Self::new_const(value)
    }

    /// Returns the raw u32 value.
    pub const fn as_u32(self) -> u32 {
        self.0.get()
    }

    /// Gets the error code value (bits 15-0).
    pub const fn code(self) -> u16 {
        (self.0.get() & 0xFFFF) as u16
    }

    /// Gets the module identifier (bits 29-16).
    pub const fn module(self) -> u16 {
        ((self.0.get() >> 16) & 0x3FFF) as u16
    }

    /// Checks if this is an extended module (bit 31 set in module field).
    pub const fn is_extended_module(self) -> bool {
        (self.0.get() & 0x8000_0000) != 0
    }
}

impl ErrorModule {
    /// Creates a new error module with the given 14-bit module identifier.
    ///
    /// # Panics
    /// Panics if module >= 0x4000 (exceeds 14 bits).
    pub const fn new(module: u16) -> Self {
        assert!(module < 0x4000, "module must be less than 0x4000");
        ErrorModule((module as u32) << 16)
    }

    /// Creates a new extended error module with the given 14-bit module identifier.
    ///
    /// The extended bit (0x8000_0000) is set to distinguish extended modules.
    ///
    /// # Panics
    /// Panics if module >= 0x4000 (exceeds 14 bits).
    pub const fn new_ext(module: u16) -> Self {
        assert!(module < 0x4000, "module must be less than 0x4000");
        ErrorModule(0x8000_0000 | ((module as u32) << 16))
    }

    /// Creates an error with this module and the given error code.
    pub const fn error(self, code: u16) -> Error {
        let combined = self.0 | (code as u32);
        // Safety: self.0 is non-zero (set in new/new_ext), so the OR result is non-zero
        Error(unsafe { NonZeroU32::new_unchecked(combined) })
    }

    /// Returns the module identifier.
    pub const fn module_id(self) -> u16 {
        ((self.0 >> 16) & 0x3FFF) as u16
    }
}

/// Shorthand for Result with Error as the error type.
pub type Result<T> = core::result::Result<T, Error>;

/// Trait for types that can be converted to a precise Error.
///
/// This trait enables bridge implementations that convert from existing error types
/// (like `ToPreciseError`, HAL-specific errors, or third-party libraries) into the unified
/// `Error` type. This supports gradual migration of error handling across the codebase.
pub trait IntoError {
    /// Converts this error into a precise Error.
    fn into_error(self) -> Error;
}

impl IntoError for Error {
    fn into_error(self) -> Error {
        self
    }
}

/// Trait for types that can be mapped into a precise Error.
///
/// This trait provides a bridge for existing error handling code that uses the
/// standard `core::fmt::Error` pattern or similar error traits. Implementations
/// should map domain-specific error kinds into precise Error codes.
pub trait ToPreciseError: core::fmt::Debug {
    /// Converts this error kind into a precise Error.
    ///
    /// Implementations should map their specific error types to module and error codes.
    fn to_precise_error(&self) -> Error;
}

/// Default implementation for `core::fmt::Error`.
///
/// Maps all formatting failures to a generic util-error code.
impl From<core::fmt::Error> for Error {
    fn from(_: core::fmt::Error) -> Self {
        const CATEGORY_GENERIC_ERROR: u32 = 1 << 30;
        const MODULE_UTIL_ERROR: u16 = 0;
        const CODE_FMT_ERROR: u16 = 1;

        let combined = CATEGORY_GENERIC_ERROR | ((MODULE_UTIL_ERROR as u32) << 16) | (CODE_FMT_ERROR as u32);
        // Safety: combined is non-zero (has CATEGORY_GENERIC_ERROR bit set)
        Error(unsafe { NonZeroU32::new_unchecked(combined) })
    }
}

/// Helper macro for defining module-specific error builders with ToPreciseError support.
///
/// # Example
///
/// ```ignore
/// define_error_module!(
///     pub const ERR_ECDSA: 0x1234,
///     pub const ERR_ECDSA_BUSY = 1,
///     pub const ERR_ECDSA_INVALID_SIG = 2,
///     pub const ERR_ECDSA_KEYGEN = 3,
/// );
/// ```
#[macro_export]
macro_rules! define_error_module {
    (pub const $module_name:ident : $module_id:expr, $(pub const $error_name:ident = $error_code:expr),* $(,)?) => {
        pub const $module_name: $crate::ErrorModule = $crate::ErrorModule::new($module_id);
        $(pub const $error_name: $crate::Error = $module_name.error($error_code);)*
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let module = ErrorModule::new(0x1234);
        let error = module.error(0x5678);
        assert_eq!(error.code(), 0x5678);
        assert_eq!(error.module(), 0x1234);
        assert!(!error.is_extended_module());
    }

    #[test]
    fn test_extended_module() {
        let module = ErrorModule::new_ext(0x1234);
        let error = module.error(0x5678);
        assert_eq!(error.code(), 0x5678);
        assert_eq!(error.module(), 0x1234);
        assert!(error.is_extended_module());
    }

    #[test]
    fn test_error_from_raw() {
        let error = Error::from_raw(0x1234_5678);
        assert_eq!(error.as_u32(), 0x1234_5678);
    }

    #[test]
    fn test_into_error_trait() {
        let module = ErrorModule::new(0x5555);
        let error = module.error(42);
        let converted: Error = error.into_error();
        assert_eq!(converted.module(), 0x5555);
        assert_eq!(converted.code(), 42);
    }

    #[test]
    fn test_fmt_error_conversion() {
        let fmt_err = core::fmt::Error;
        let _precise_err: Error = fmt_err.into();
        // Successfully converted
    }

    #[test]
    fn test_macro_error_definition() {
        define_error_module!(
            pub const TEST_MODULE: 0x1111,
            pub const TEST_ERR_A = 1,
            pub const TEST_ERR_B = 2,
        );

        assert_eq!(TEST_ERR_A.code(), 1);
        assert_eq!(TEST_ERR_B.code(), 2);
        assert_eq!(TEST_ERR_A.module(), 0x1111);
    }
}
