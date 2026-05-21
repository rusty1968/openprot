// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! RFC Precise Errors implementation.
//!
//! Layout:
//! - bit 31: external module namespace flag
//! - bits 30..16: module id (15 bits)
//! - bits 15..0: module-local code
#![cfg_attr(not(test), no_std)]

use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Error(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorModule(u32);

/// Canonical module IDs.
///
/// Keep one ID per crate. These IDs should remain stable once published.
///
/// Policy: project-owned crates use the internal namespace (`ErrorModule::new`).
pub mod module_ids {
    use super::ErrorModule;

    // util/*
    pub const UTIL_ERROR: ErrorModule = ErrorModule::new(0x0001);
    pub const UTIL_REGCPY: ErrorModule = ErrorModule::new(0x0002);

    // services/mctp/*
    pub const MCTP_API: ErrorModule = ErrorModule::new(0x0100);
    pub const MCTP_CLIENT: ErrorModule = ErrorModule::new(0x0101);
    pub const MCTP_SERVER: ErrorModule = ErrorModule::new(0x0102);
    pub const MCTP_ECHO: ErrorModule = ErrorModule::new(0x0103);
    pub const MCTP_TRANSPORT_I2C: ErrorModule = ErrorModule::new(0x0104);
}

impl Error {
    pub const OK: Self = Self::status(Status::Ok);
    pub const CANCELLED: Self = Self::status(Status::Cancelled);
    pub const UNKNOWN: Self = Self::status(Status::Unknown);
    pub const INVALID_ARGUMENT: Self = Self::status(Status::InvalidArgument);
    pub const DEADLINE_EXCEEDED: Self = Self::status(Status::DeadlineExceeded);
    pub const NOT_FOUND: Self = Self::status(Status::NotFound);
    pub const ALREADY_EXISTS: Self = Self::status(Status::AlreadyExists);
    pub const PERMISSION_DENIED: Self = Self::status(Status::PermissionDenied);
    pub const RESOURCE_EXHAUSTED: Self = Self::status(Status::ResourceExhausted);
    pub const FAILED_PRECONDITION: Self = Self::status(Status::FailedPrecondition);
    pub const ABORTED: Self = Self::status(Status::Aborted);
    pub const OUT_OF_RANGE: Self = Self::status(Status::OutOfRange);
    pub const UNIMPLEMENTED: Self = Self::status(Status::Unimplemented);
    pub const INTERNAL: Self = Self::status(Status::Internal);
    pub const UNAVAILABLE: Self = Self::status(Status::Unavailable);
    pub const DATA_LOSS: Self = Self::status(Status::DataLoss);
    pub const UNAUTHENTICATED: Self = Self::status(Status::Unauthenticated);

    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn status(status: Status) -> Self {
        Self(status as u32)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn module(self) -> u16 {
        ((self.0 >> 16) & 0x7fff) as u16
    }

    pub const fn code(self) -> u16 {
        (self.0 & 0xffff) as u16
    }

    pub const fn is_external(self) -> bool {
        (self.0 & 0x8000_0000) != 0
    }

    pub const fn is_ok(self) -> bool {
        self.0 == 0
    }
}

impl ErrorModule {
    pub const fn new(module: u16) -> Self {
        assert!(module < 0x8000);
        Self((module as u32) << 16)
    }

    pub const fn new_ext(module: u16) -> Self {
        assert!(module < 0x8000);
        Self(0x8000_0000 | ((module as u32) << 16))
    }

    pub const fn id(self) -> u16 {
        ((self.0 >> 16) & 0x7fff) as u16
    }

    pub const fn is_external(self) -> bool {
        (self.0 & 0x8000_0000) != 0
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn error(self, code: u16) -> Error {
        Error(self.0 | (code as u32))
    }
}

#[repr(u16)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Status {
    Ok = 0,
    Cancelled = 1,
    Unknown = 2,
    InvalidArgument = 3,
    DeadlineExceeded = 4,
    NotFound = 5,
    AlreadyExists = 6,
    PermissionDenied = 7,
    ResourceExhausted = 8,
    FailedPrecondition = 9,
    Aborted = 10,
    OutOfRange = 11,
    Unimplemented = 12,
    Internal = 13,
    Unavailable = 14,
    DataLoss = 15,
    Unauthenticated = 16,
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("raw", &self.0)
            .field("external", &self.is_external())
            .field("module", &self.module())
            .field("code", &self.code())
            .finish()
    }
}

impl fmt::Debug for ErrorModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorModule")
            .field("raw", &self.0)
            .field("external", &self.is_external())
            .field("id", &self.id())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, ErrorModule, Status, module_ids};

    #[test]
    fn status_constants_match_rfc_codes() {
        assert_eq!(Error::OK.raw(), 0);
        assert_eq!(Error::CANCELLED.raw(), 1);
        assert_eq!(Error::UNKNOWN.raw(), 2);
        assert_eq!(Error::INVALID_ARGUMENT.raw(), 3);
        assert_eq!(Error::DEADLINE_EXCEEDED.raw(), 4);
        assert_eq!(Error::NOT_FOUND.raw(), 5);
        assert_eq!(Error::ALREADY_EXISTS.raw(), 6);
        assert_eq!(Error::PERMISSION_DENIED.raw(), 7);
        assert_eq!(Error::RESOURCE_EXHAUSTED.raw(), 8);
        assert_eq!(Error::FAILED_PRECONDITION.raw(), 9);
        assert_eq!(Error::ABORTED.raw(), 10);
        assert_eq!(Error::OUT_OF_RANGE.raw(), 11);
        assert_eq!(Error::UNIMPLEMENTED.raw(), 12);
        assert_eq!(Error::INTERNAL.raw(), 13);
        assert_eq!(Error::UNAVAILABLE.raw(), 14);
        assert_eq!(Error::DATA_LOSS.raw(), 15);
        assert_eq!(Error::UNAUTHENTICATED.raw(), 16);
        assert_eq!(Error::status(Status::Internal), Error::INTERNAL);
    }

    #[test]
    fn module_encoding_round_trip() {
        let module = ErrorModule::new(0x1234);
        let err = module.error(0x00ab);

        assert_eq!(module.id(), 0x1234);
        assert!(!module.is_external());
        assert_eq!(err.module(), 0x1234);
        assert_eq!(err.code(), 0x00ab);
        assert!(!err.is_external());
        assert_eq!(err.raw(), 0x1234_00ab);
    }

    #[test]
    fn external_module_sets_top_bit() {
        let module = ErrorModule::new_ext(0x0065);
        let err = module.error(0x0007);

        assert!(module.is_external());
        assert_eq!(module.id(), 0x0065);
        assert!(err.is_external());
        assert_eq!(err.module(), 0x0065);
        assert_eq!(err.code(), 0x0007);
        assert_eq!(err.raw(), 0x8065_0007);
    }

    #[test]
    fn ok_detection_matches_raw_zero() {
        assert!(Error::OK.is_ok());
        assert!(!Error::INTERNAL.is_ok());
        assert!(!ErrorModule::new(1).error(0).is_ok());
    }

    #[test]
    #[should_panic]
    fn module_id_must_fit_15_bits() {
        let _ = ErrorModule::new(0x8000);
    }

    #[test]
    fn module_ids_are_unique() {
        let ids = [
            module_ids::UTIL_ERROR.id(),
            module_ids::UTIL_REGCPY.id(),
            module_ids::MCTP_API.id(),
            module_ids::MCTP_CLIENT.id(),
            module_ids::MCTP_SERVER.id(),
            module_ids::MCTP_ECHO.id(),
            module_ids::MCTP_TRANSPORT_I2C.id(),
        ];

        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i], ids[j], "duplicate module id: {:#06x}", ids[i]);
            }
        }
    }

    #[test]
    fn module_ids_use_internal_namespace() {
        let modules = [
            module_ids::UTIL_ERROR,
            module_ids::UTIL_REGCPY,
            module_ids::MCTP_API,
            module_ids::MCTP_CLIENT,
            module_ids::MCTP_SERVER,
            module_ids::MCTP_ECHO,
            module_ids::MCTP_TRANSPORT_I2C,
        ];

        for m in modules {
            assert!(!m.is_external(), "module id unexpectedly external: {m:?}");
        }
    }
}
