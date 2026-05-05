// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Backend trait that platform flash drivers implement.
//!
//! The shape mirrors the `FlashStorage` HIL from caliptra-mcu-sw but is
//! synchronous and buffer-borrowing rather than callback-based: the
//! server runtime drives concurrency, so backends only need to expose
//! a blocking-or-`WouldBlock` surface. `WouldBlock` is backend-internal
//! and is not encoded on the wire.

use crate::protocol::{FlashError, FlashGeometry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendError {
    InvalidOperation,
    InvalidAddress,
    InvalidLength,
    BufferTooSmall,
    Busy,
    Timeout,
    /// Operation cannot complete synchronously now.
    ///
    /// This is an internal backend/server scheduling signal. The server
    /// runtime should defer and retry after a progress signal, typically
    /// a completion interrupt, rather than exposing it directly on the wire.
    WouldBlock,
    /// Media-level failure (program/erase verify fail, ECC uncorrectable, …).
    IoError,
    /// Operation is blocked by backend policy or protection state.
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
            BackendError::WouldBlock => FlashError::Busy,
            BackendError::IoError => FlashError::IoError,
            BackendError::NotPermitted => FlashError::NotPermitted,
            BackendError::InternalError => FlashError::InternalError,
        }
    }
}

/// Static description of the flash device a backend exposes. Reported
/// to clients via `GetCapacity`.
///
/// The per-call payload cap is *not* part of `FlashInfo`: it is fixed
/// by the protocol (`MAX_PAYLOAD_SIZE`) and the same for every
/// backend. Clients reference the constant directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlashInfo {
    /// Total addressable bytes [0, capacity).
    pub capacity: u32,
    /// Smallest erasable unit, in bytes. Erase requests must be aligned
    /// and sized in multiples of this value.
    pub erase_size: u32,
}

/// Core backend operations required by the flash server runtime.
///
/// This trait is intentionally minimal: it captures the data-plane
/// operations and minimal static information (`FlashInfo`) needed to
/// service read/write/erase traffic.
pub trait FlashBackend {
    /// Per-call routing key. Single-CS backends set this to `()`; multi-CS
    /// backends set it to a CS index (e.g. `ChipSelect`) so the server
    /// runtime can dispatch each channel to the right device on a shared
    /// controller.
    type RouteKey: Copy;

    /// Static layout/features of the device selected by `key`.
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

    fn enable_interrupts(&mut self) -> Result<(), BackendError>;

    fn disable_interrupts(&mut self) -> Result<(), BackendError>;
}

/// Optional geometry-discovery extension for [`FlashBackend`].
///
/// Server code that exposes the `GetGeometry` opcode should bound the
/// backend type by this trait. A blanket impl provides a default
/// geometry derived from [`FlashBackend::info`], so simple backends do
/// not need to add any extra code.
pub trait FlashGeometryProvider: FlashBackend {
    /// Wire-shaped geometry for the device selected by `key`. Powers the
    /// `GetGeometry` opcode.
    ///
    /// Default derives from `info()`: a single erase granularity (the
    /// one already advertised in `FlashInfo`), 256-byte page,
    /// address-width inferred from capacity, no opaque flags. A
    /// backend that supports multiple erase granules (4 K + 64 K, etc.)
    /// or has backend-defined opaque flag bits should override.
    fn geometry(&self, key: Self::RouteKey) -> Result<FlashGeometry, BackendError> {
        let info = self.info(key);
        let erase_bitmap = if info.erase_size != 0 && info.erase_size.is_power_of_two() {
            info.erase_size
        } else {
            0
        };
        let address_width: u8 = if info.capacity > 0x0100_0000 { 4 } else { 3 };
        Ok(FlashGeometry::new(
            info.capacity,
            256,
            erase_bitmap,
            info.erase_size,
            address_width,
            0,
        ))
    }
}

impl<T: FlashBackend> FlashGeometryProvider for T {}
