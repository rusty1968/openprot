// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Backend trait that platform flash drivers implement.
//!
//! The shape mirrors the `FlashStorage` HIL from caliptra-mcu-sw but is
//! synchronous and buffer-borrowing rather than callback-based: the
//! server runtime drives concurrency, so backends only need to expose
//! a blocking-or-`WouldBlock` surface.

use bitflags::bitflags;

use crate::protocol::FlashError;

bitflags! {
    /// Hardware-agnostic interrupt sources a flash backend can raise.
    ///
    /// This is currently an internal contract between the flash server
    /// runtime and platform backends. The client API remains synchronous.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct IrqMask: u16 {
        /// A previously-issued operation that returned `WouldBlock` can
        /// now be retried.
        const OPERATION_COMPLETE = 0x0001;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendError {
    InvalidOperation,
    InvalidAddress,
    InvalidLength,
    BufferTooSmall,
    Busy,
    Timeout,
    /// Backend cannot complete synchronously at this time; the server
    /// runtime should retry after `OPERATION_COMPLETE` fires.
    WouldBlock,
    /// Media-level failure (program/erase verify fail, ECC uncorrectable, …).
    IoError,
    /// Region is write-protected, locked, or otherwise refused.
    NotPermitted,
    InternalError,
}

impl From<BackendError> for FlashError {
    fn from(value: BackendError) -> Self {
        match value {
            BackendError::InvalidOperation => FlashError::InvalidOperation,
            BackendError::InvalidAddress => FlashError::InvalidAddress,
            BackendError::InvalidLength => FlashError::InvalidLength,
            BackendError::BufferTooSmall => FlashError::BufferTooSmall,
            BackendError::Busy => FlashError::Busy,
            BackendError::Timeout => FlashError::Timeout,
            BackendError::WouldBlock => FlashError::WouldBlock,
            BackendError::IoError => FlashError::IoError,
            BackendError::NotPermitted => FlashError::NotPermitted,
            BackendError::InternalError => FlashError::InternalError,
        }
    }
}

/// Static description of the flash region a backend exposes. Reported
/// to clients via `GetCapacity` / `GetChunkSize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlashInfo {
    /// Total addressable bytes [0, capacity).
    pub capacity: u32,
    /// Largest single read/write the backend will accept.
    pub chunk_size: u32,
    /// Smallest erasable unit, in bytes. Erase requests must be aligned
    /// and sized in multiples of this value.
    pub erase_size: u32,
}

pub trait FlashBackend {
    /// Per-call routing key. Single-CS backends set this to `()`; multi-CS
    /// backends set it to a CS index (e.g. `ChipSelect`) so the server
    /// runtime can dispatch each channel to the right device on a shared
    /// controller.
    type RouteKey: Copy;

    /// Static layout/capability of the device selected by `key`.
    fn info(&self, key: Self::RouteKey) -> FlashInfo;

    /// Probe whether the flash device selected by `key` is present and
    /// responsive.
    ///
    /// Default implementation assumes presence so existing backends remain
    /// source-compatible until they opt into a hardware-backed probe.
    fn exists(&mut self, _key: Self::RouteKey) -> Result<bool, BackendError> {
        Ok(true)
    }

    /// Read up to `out.len()` bytes from the device selected by `key`,
    /// starting at the device-relative `address`, into `out`. Returns the
    /// number of bytes actually read.
    fn read(
        &mut self,
        key: Self::RouteKey,
        address: u32,
        out: &mut [u8],
    ) -> Result<usize, BackendError>;

    /// Write `data` to the device selected by `key`, starting at the
    /// device-relative `address`. Returns the number of bytes actually
    /// written.
    fn write(
        &mut self,
        key: Self::RouteKey,
        address: u32,
        data: &[u8],
    ) -> Result<usize, BackendError>;

    /// Erase `length` bytes on the device selected by `key`, starting at
    /// the device-relative `address`. Both must be multiples of
    /// `FlashInfo::erase_size`.
    fn erase(
        &mut self,
        key: Self::RouteKey,
        address: u32,
        length: u32,
    ) -> Result<(), BackendError>;

    /// Enable backend-side interrupt sources. Default: no-op for
    /// fully-synchronous backends. Interrupts are controller-wide, not
    /// per-CS, so this method does not take a `RouteKey`.
    fn enable_interrupts(&mut self, _mask: IrqMask) -> Result<(), BackendError> {
        Ok(())
    }

    /// Disable backend-side interrupt sources. Default: no-op.
    fn disable_interrupts(&mut self, _mask: IrqMask) -> Result<(), BackendError> {
        Ok(())
    }
}
