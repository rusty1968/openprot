// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Wire protocol for the flash driver IPC channel.
//!
//! The operation set mirrors the flash storage HIL used in caliptra-mcu-sw
//! (`runtime/kernel/drivers/flash`), reframed as an opcode + packed-header
//! protocol matching the conventions of the other OpenPRoT userspace
//! drivers (see `drivers/usart/api`).

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// Maximum payload bytes carried in a single request or response.
///
/// Larger logical I/O is split into chunks by the client. This is a
/// protocol constant — every backend honours the same value, so clients
/// can reference it directly rather than querying the server.
pub const MAX_PAYLOAD_SIZE: usize = 256;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashOp {
    /// Probe the driver. Response carries no value.
    Exists = 0x01,
    /// Total bytes of flash exposed by this backend. Result in `value`.
    GetCapacity = 0x02,
    /// Read `length` bytes starting at `address`. Response payload carries
    /// the bytes read; `value` is the byte count.
    Read = 0x03,
    /// Write the request payload (`payload_len` bytes) starting at
    /// `address`. `length` must equal `payload_len`. `value` returns the
    /// byte count actually written.
    Write = 0x04,
    /// Erase `length` bytes starting at `address`.
    Erase = 0x05,
    /// Discover device geometry — capacity, page size, supported erase
    /// granularities, address width, opaque flags. Response carries
    /// one `FlashGeometry` record in the payload. The response-header
    /// `value` word is unused for this opcode (set to 0 and ignored).
    GetGeometry = 0x06,
}

impl TryFrom<u8> for FlashOp {
    type Error = FlashError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Exists),
            0x02 => Ok(Self::GetCapacity),
            0x03 => Ok(Self::Read),
            0x04 => Ok(Self::Write),
            0x05 => Ok(Self::Erase),
            0x06 => Ok(Self::GetGeometry),
            _ => Err(FlashError::InvalidOperation),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashError {
    Success = 0x00,
    InvalidOperation = 0x01,
    InvalidAddress = 0x02,
    InvalidLength = 0x03,
    BufferTooSmall = 0x04,
    Busy = 0x05,
    Timeout = 0x06,
    /// Operation cannot complete right now; the server/runtime may defer
    /// completion until the backend signals progress via interrupt.
    WouldBlock = 0x07,
    /// Underlying media reported an I/O error (e.g. flash program failure).
    IoError = 0x08,
    /// Address/length straddles a region the backend refuses to touch
    /// (e.g. write-protected partition).
    NotPermitted = 0x09,
    InternalError = 0xFF,
}

impl From<u8> for FlashError {
    fn from(value: u8) -> Self {
        match value {
            0x00 => Self::Success,
            0x01 => Self::InvalidOperation,
            0x02 => Self::InvalidAddress,
            0x03 => Self::InvalidLength,
            0x04 => Self::BufferTooSmall,
            0x05 => Self::Busy,
            0x06 => Self::Timeout,
            0x07 => Self::WouldBlock,
            0x08 => Self::IoError,
            0x09 => Self::NotPermitted,
            _ => Self::InternalError,
        }
    }
}

/// Request header on the wire. 16 bytes, little-endian, packed.
///
/// `address` and `length` are interpreted per `op_code`; see `FlashOp`.
/// `payload_len` is the number of bytes that immediately follow this
/// header in the request frame (zero for read/erase/probe ops).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct FlashRequestHeader {
    pub op_code: u8,
    pub flags: u8,
    pub payload_len: u16,
    pub address: u32,
    pub length: u32,
    pub reserved: u32,
}

impl FlashRequestHeader {
    pub const SIZE: usize = 16;

    pub fn new(op: FlashOp, address: u32, length: u32, payload_len: u16) -> Self {
        Self {
            op_code: op as u8,
            flags: 0,
            payload_len: payload_len.to_le(),
            address: address.to_le(),
            length: length.to_le(),
            reserved: 0,
        }
    }

    pub fn operation(&self) -> Result<FlashOp, FlashError> {
        FlashOp::try_from(self.op_code)
    }

    pub fn address_value(&self) -> u32 {
        u32::from_le(self.address)
    }

    pub fn length_value(&self) -> u32 {
        u32::from_le(self.length)
    }

    pub fn payload_length(&self) -> usize {
        u16::from_le(self.payload_len) as usize
    }
}

/// Response header on the wire. 8 bytes, little-endian, packed.
///
/// `value` is a per-op return word — capacity, chunk size, bytes
/// processed, etc. `payload_len` counts bytes that follow this header
/// (non-zero only for `Read`).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct FlashResponseHeader {
    pub status: u8,
    pub reserved: u8,
    pub payload_len: u16,
    pub value: u32,
}

impl FlashResponseHeader {
    pub const SIZE: usize = 8;

    pub fn success(value: u32, payload_len: u16) -> Self {
        Self {
            status: FlashError::Success as u8,
            reserved: 0,
            payload_len: payload_len.to_le(),
            value: value.to_le(),
        }
    }

    pub fn error(error: FlashError) -> Self {
        Self {
            status: error as u8,
            reserved: 0,
            payload_len: 0,
            value: 0,
        }
    }

    pub fn is_success(&self) -> bool {
        self.status == FlashError::Success as u8
    }

    pub fn error_code(&self) -> FlashError {
        FlashError::from(self.status)
    }

    pub fn value_word(&self) -> u32 {
        u32::from_le(self.value)
    }

    pub fn payload_length(&self) -> usize {
        u16::from_le(self.payload_len) as usize
    }
}

/// Static device geometry returned in the `GetGeometry` response payload.
/// 24 bytes, little-endian, packed.
///
/// `erase_sizes` is a bitmap: bit `n` set means the backend supports an
/// erase opcode of `1 << n` bytes (e.g. 4 KiB | 32 KiB | 64 KiB =
/// `(1<<12) | (1<<15) | (1<<16)`).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct FlashGeometry {
    pub capacity: u32,
    pub page_size: u32,
    pub erase_sizes: u32,
    pub min_erase_align: u32,
    pub address_width: u8,
    pub flags: u8,
    pub _rsv: [u8; 6],
}

impl FlashGeometry {
    pub const SIZE: usize = 24;

    pub fn new(
        capacity: u32,
        page_size: u32,
        erase_sizes: u32,
        min_erase_align: u32,
        address_width: u8,
        flags: u8,
    ) -> Self {
        Self {
            capacity: capacity.to_le(),
            page_size: page_size.to_le(),
            erase_sizes: erase_sizes.to_le(),
            min_erase_align: min_erase_align.to_le(),
            address_width,
            flags,
            _rsv: [0; 6],
        }
    }

    pub fn capacity_value(&self) -> u32 {
        u32::from_le(self.capacity)
    }

    pub fn page_size_value(&self) -> u32 {
        u32::from_le(self.page_size)
    }

    pub fn erase_sizes_bitmap(&self) -> u32 {
        u32::from_le(self.erase_sizes)
    }

    pub fn min_erase_align_value(&self) -> u32 {
        u32::from_le(self.min_erase_align)
    }

    pub fn raw_flags(&self) -> u8 {
        self.flags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use zerocopy::{FromBytes, IntoBytes};

    #[test]
    fn flash_op_try_from_known_codes() {
        let cases = [
            (0x01u8, FlashOp::Exists),
            (0x02, FlashOp::GetCapacity),
            (0x03, FlashOp::Read),
            (0x04, FlashOp::Write),
            (0x05, FlashOp::Erase),
            (0x06, FlashOp::GetGeometry),
        ];
        for (byte, op) in cases {
            assert_eq!(FlashOp::try_from(byte).unwrap(), op);
            assert_eq!(op as u8, byte);
        }
    }

    #[test]
    fn flash_op_try_from_unknown_is_invalid_operation() {
        for byte in [0x00u8, 0x07, 0x08, 0x09, 0x10, 0x42, 0xFF] {
            assert_eq!(
                FlashOp::try_from(byte).unwrap_err(),
                FlashError::InvalidOperation
            );
        }
    }

    #[test]
    fn flash_error_from_known_codes_round_trip() {
        let cases = [
            (0x00u8, FlashError::Success),
            (0x01, FlashError::InvalidOperation),
            (0x02, FlashError::InvalidAddress),
            (0x03, FlashError::InvalidLength),
            (0x04, FlashError::BufferTooSmall),
            (0x05, FlashError::Busy),
            (0x06, FlashError::Timeout),
            (0x07, FlashError::WouldBlock),
            (0x08, FlashError::IoError),
            (0x09, FlashError::NotPermitted),
            (0xFF, FlashError::InternalError),
        ];
        for (byte, err) in cases {
            assert_eq!(FlashError::from(byte), err);
            assert_eq!(err as u8, byte);
        }
    }

    #[test]
    fn flash_error_from_unknown_byte_is_internal_error() {
        for byte in [0x0Au8, 0x10, 0x42, 0x80, 0xFE] {
            assert_eq!(FlashError::from(byte), FlashError::InternalError);
        }
    }

    #[test]
    fn request_header_size_matches_const_and_wire_layout() {
        assert_eq!(core::mem::size_of::<FlashRequestHeader>(), 16);
        assert_eq!(FlashRequestHeader::SIZE, 16);
        let hdr = FlashRequestHeader::new(FlashOp::Read, 0, 0, 0);
        assert_eq!(hdr.as_bytes().len(), 16);
    }

    #[test]
    fn request_header_new_then_accessors() {
        let hdr = FlashRequestHeader::new(FlashOp::Read, 0xDEAD_BEEF, 0x10, 0);
        assert_eq!(hdr.operation().unwrap(), FlashOp::Read);
        assert_eq!(hdr.address_value(), 0xDEAD_BEEF);
        assert_eq!(hdr.length_value(), 0x10);
        assert_eq!(hdr.payload_length(), 0);

        let hdr = FlashRequestHeader::new(FlashOp::Write, 0x4000, 0x80, 0x80);
        assert_eq!(hdr.operation().unwrap(), FlashOp::Write);
        assert_eq!(hdr.address_value(), 0x4000);
        assert_eq!(hdr.length_value(), 0x80);
        assert_eq!(hdr.payload_length(), 0x80);
    }

    #[test]
    fn request_header_encodes_little_endian_on_wire() {
        let hdr = FlashRequestHeader::new(FlashOp::Read, 0x0403_0201, 0x0807_0605, 0x0A09);
        let bytes = hdr.as_bytes();
        assert_eq!(bytes[0], FlashOp::Read as u8);
        assert_eq!(bytes[1], 0);
        assert_eq!(&bytes[2..4], &[0x09, 0x0A]);
        assert_eq!(&bytes[4..8], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(&bytes[8..12], &[0x05, 0x06, 0x07, 0x08]);
        assert_eq!(&bytes[12..16], &[0, 0, 0, 0]);
    }

    #[test]
    fn request_header_round_trip_through_bytes() {
        let original = FlashRequestHeader::new(FlashOp::Erase, 0xCAFE_BABE, 0x1000, 0);
        let bytes = original.as_bytes();
        let decoded = FlashRequestHeader::ref_from_bytes(bytes).unwrap();
        assert_eq!(decoded.operation().unwrap(), FlashOp::Erase);
        assert_eq!(decoded.address_value(), 0xCAFE_BABE);
        assert_eq!(decoded.length_value(), 0x1000);
        assert_eq!(decoded.payload_length(), 0);
    }

    #[test]
    fn request_header_decode_invalid_op_byte_surfaces_error() {
        let mut bytes = [0u8; 16];
        bytes[0] = 0xAB;
        let hdr = FlashRequestHeader::ref_from_bytes(&bytes[..]).unwrap();
        assert_eq!(hdr.operation().unwrap_err(), FlashError::InvalidOperation);
    }

    #[test]
    fn request_header_decode_short_buffer_fails() {
        let bytes = [0u8; 15];
        assert!(FlashRequestHeader::ref_from_bytes(&bytes[..]).is_err());
    }

    #[test]
    fn response_header_size_matches_const_and_wire_layout() {
        assert_eq!(core::mem::size_of::<FlashResponseHeader>(), 8);
        assert_eq!(FlashResponseHeader::SIZE, 8);
        let hdr = FlashResponseHeader::success(0, 0);
        assert_eq!(hdr.as_bytes().len(), 8);
    }

    #[test]
    fn response_header_success_builder_matches_accessors() {
        let hdr = FlashResponseHeader::success(0x1234_5678, 0x80);
        assert!(hdr.is_success());
        assert_eq!(hdr.error_code(), FlashError::Success);
        assert_eq!(hdr.value_word(), 0x1234_5678);
        assert_eq!(hdr.payload_length(), 0x80);
    }

    #[test]
    fn response_header_error_builder_zeroes_payload_and_value() {
        let hdr = FlashResponseHeader::error(FlashError::IoError);
        assert!(!hdr.is_success());
        assert_eq!(hdr.error_code(), FlashError::IoError);
        assert_eq!(hdr.value_word(), 0);
        assert_eq!(hdr.payload_length(), 0);
    }

    #[test]
    fn response_header_encodes_little_endian_on_wire() {
        let hdr = FlashResponseHeader::success(0x0807_0605, 0x0403);
        let bytes = hdr.as_bytes();
        assert_eq!(bytes[0], FlashError::Success as u8);
        assert_eq!(bytes[1], 0);
        assert_eq!(&bytes[2..4], &[0x03, 0x04]);
        assert_eq!(&bytes[4..8], &[0x05, 0x06, 0x07, 0x08]);
    }

    #[test]
    fn response_header_round_trip_through_bytes() {
        for err in [
            FlashError::Success,
            FlashError::InvalidAddress,
            FlashError::WouldBlock,
            FlashError::NotPermitted,
            FlashError::InternalError,
        ] {
            let original = if matches!(err, FlashError::Success) {
                FlashResponseHeader::success(0xAA, 0x55)
            } else {
                FlashResponseHeader::error(err)
            };
            let bytes = original.as_bytes();
            let decoded = FlashResponseHeader::ref_from_bytes(bytes).unwrap();
            assert_eq!(decoded.error_code(), err);
            assert_eq!(decoded.is_success(), matches!(err, FlashError::Success));
            if !matches!(err, FlashError::Success) {
                assert_eq!(decoded.value_word(), 0);
                assert_eq!(decoded.payload_length(), 0);
            }
        }
    }

    #[test]
    fn get_geometry_response_header_uses_payload_not_value_word() {
        let hdr = FlashResponseHeader::success(0, FlashGeometry::SIZE as u16);
        assert!(hdr.is_success());
        assert_eq!(hdr.value_word(), 0);
        assert_eq!(hdr.payload_length(), FlashGeometry::SIZE);
    }

    #[test]
    fn response_header_decode_short_buffer_fails() {
        let bytes = [0u8; 7];
        assert!(FlashResponseHeader::ref_from_bytes(&bytes[..]).is_err());
    }

    #[test]
    fn max_payload_size_is_protocol_constant() {
        assert_eq!(MAX_PAYLOAD_SIZE, 256);
    }

    #[test]
    fn flash_geometry_size_matches_const_and_wire_layout() {
        assert_eq!(core::mem::size_of::<FlashGeometry>(), 24);
        assert_eq!(FlashGeometry::SIZE, 24);
        let geom = FlashGeometry::new(0, 0, 0, 0, 3, 0);
        assert_eq!(geom.as_bytes().len(), 24);
    }

    #[test]
    fn flash_geometry_new_then_accessors() {
        let geom = FlashGeometry::new(
            0x0100_0000,
            256,
            (1u32 << 12) | (1u32 << 15) | (1u32 << 16),
            4096,
            3,
            0x03,
        );
        assert_eq!(geom.capacity_value(), 0x0100_0000);
        assert_eq!(geom.page_size_value(), 256);
        assert_eq!(
            geom.erase_sizes_bitmap(),
            (1u32 << 12) | (1u32 << 15) | (1u32 << 16)
        );
        assert_eq!(geom.min_erase_align_value(), 4096);
        assert_eq!(geom.address_width, 3);
        assert_eq!(geom.raw_flags(), 0x03);
    }

    #[test]
    fn flash_geometry_encodes_little_endian_on_wire() {
        let geom = FlashGeometry::new(
            0x0403_0201,
            0x0807_0605,
            0x0C0B_0A09,
            0x100F_0E0D,
            0x11,
            0x01,
        );
        let bytes = geom.as_bytes();
        assert_eq!(&bytes[0..4], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(&bytes[4..8], &[0x05, 0x06, 0x07, 0x08]);
        assert_eq!(&bytes[8..12], &[0x09, 0x0A, 0x0B, 0x0C]);
        assert_eq!(&bytes[12..16], &[0x0D, 0x0E, 0x0F, 0x10]);
        assert_eq!(bytes[16], 0x11); // address_width
        assert_eq!(bytes[17], 0x01); // flags = DMA_ELIGIBLE
        assert_eq!(&bytes[18..24], &[0; 6]);
    }

    #[test]
    fn flash_geometry_round_trip_through_bytes() {
        let original = FlashGeometry::new(
            0x0080_0000,
            512,
            1u32 << 16,
            65536,
            4,
            0x02,
        );
        let bytes = original.as_bytes();
        let decoded = FlashGeometry::ref_from_bytes(bytes).unwrap();
        assert_eq!(decoded.capacity_value(), 0x0080_0000);
        assert_eq!(decoded.page_size_value(), 512);
        assert_eq!(decoded.erase_sizes_bitmap(), 1u32 << 16);
        assert_eq!(decoded.min_erase_align_value(), 65536);
        assert_eq!(decoded.address_width, 4);
        assert_eq!(decoded.raw_flags(), 0x02);
    }

    #[test]
    fn flash_geometry_raw_flags_round_trip_unknown_bits() {
        let mut bytes = [0u8; FlashGeometry::SIZE];
        bytes[17] = 0xFF; // all flag bits including unknown
        let geom = FlashGeometry::ref_from_bytes(&bytes[..]).unwrap();
        assert_eq!(geom.raw_flags(), 0xFF);
    }

}
