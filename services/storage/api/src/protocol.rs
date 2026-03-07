// Licensed under the Apache-2.0 license

//! Storage IPC Protocol Definitions
//!
//! Wire format for storage requests and responses between client and server.
//! Modeled after Caliptra MCU-SW's FlashStorage trait but adapted for
//! OpenPRoT's IPC channel architecture.

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// Maximum data payload per IPC message.
/// Chunk-based I/O: large transfers are split into MAX_CHUNK_SIZE pieces.
pub const MAX_CHUNK_SIZE: usize = 512;

/// Maximum number of partitions supported.
pub const MAX_PARTITIONS: usize = 8;

/// Maximum partition name length.
pub const MAX_PARTITION_NAME_LEN: usize = 16;

// ---------------------------------------------------------------------------
// Storage operation codes
// ---------------------------------------------------------------------------

/// Storage operation codes sent from client to server.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageOp {
    // Flash operations (0x01-0x04)
    /// Read `length` bytes from `address` (or partition offset)
    Read = 0x01,
    /// Write payload to `address` (or partition offset)
    Write = 0x02,
    /// Erase `length` bytes starting at `address`
    Erase = 0x03,
    /// Query flash capacity (total bytes)
    GetCapacity = 0x04,

    // Partition operations (0x10-0x14)
    /// List all configured partitions
    ListPartitions = 0x10,
    /// Get info about a specific partition (by index)
    GetPartitionInfo = 0x11,
    /// Read from a named partition (bounds-checked)
    PartitionRead = 0x12,
    /// Write to a named partition (bounds-checked)
    PartitionWrite = 0x13,
    /// Erase within a named partition (bounds-checked)
    PartitionErase = 0x14,

    // Boot config operations (0x20-0x2A)
    /// Get the active boot partition (A or B)
    GetActivePartition = 0x20,
    /// Set the active boot partition
    SetActivePartition = 0x21,
    /// Get partition status (Invalid/Valid/BootFailed/BootSuccessful)
    GetPartitionStatus = 0x22,
    /// Set partition status
    SetPartitionStatus = 0x23,
    /// Get boot count for a partition
    GetBootCount = 0x24,
    /// Increment boot count for a partition
    IncrementBootCount = 0x25,
    /// Check if rollback is enabled
    IsRollbackEnabled = 0x26,
    /// Enable or disable rollback
    SetRollbackEnable = 0x27,
    /// Persist boot config to flash
    PersistBootConfig = 0x28,
}

impl StorageOp {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::Read),
            0x02 => Some(Self::Write),
            0x03 => Some(Self::Erase),
            0x04 => Some(Self::GetCapacity),
            0x10 => Some(Self::ListPartitions),
            0x11 => Some(Self::GetPartitionInfo),
            0x12 => Some(Self::PartitionRead),
            0x13 => Some(Self::PartitionWrite),
            0x14 => Some(Self::PartitionErase),
            0x20 => Some(Self::GetActivePartition),
            0x21 => Some(Self::SetActivePartition),
            0x22 => Some(Self::GetPartitionStatus),
            0x23 => Some(Self::SetPartitionStatus),
            0x24 => Some(Self::GetBootCount),
            0x25 => Some(Self::IncrementBootCount),
            0x26 => Some(Self::IsRollbackEnabled),
            0x27 => Some(Self::SetRollbackEnable),
            0x28 => Some(Self::PersistBootConfig),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

/// Storage error codes returned by the server.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// No error
    Ok = 0,
    /// Generic failure
    Failed = 1,
    /// Device busy, retry later
    Busy = 2,
    /// Invalid parameter (address, length, partition index)
    InvalidParam = 3,
    /// Size exceeds partition or device bounds
    OutOfBounds = 4,
    /// Operation not supported by this backend
    NotSupported = 5,
    /// Device not available
    NoDevice = 6,
    /// Invalid data length in request
    InvalidDataLength = 7,
    /// Unknown operation code
    UnknownOp = 8,
    /// Partition not found
    PartitionNotFound = 9,
    /// Boot config error
    BootConfigError = 10,
    /// Checksum verification failed
    ChecksumError = 11,
}

impl StorageError {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Ok),
            1 => Some(Self::Failed),
            2 => Some(Self::Busy),
            3 => Some(Self::InvalidParam),
            4 => Some(Self::OutOfBounds),
            5 => Some(Self::NotSupported),
            6 => Some(Self::NoDevice),
            7 => Some(Self::InvalidDataLength),
            8 => Some(Self::UnknownOp),
            9 => Some(Self::PartitionNotFound),
            10 => Some(Self::BootConfigError),
            11 => Some(Self::ChecksumError),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Request & Response headers
// ---------------------------------------------------------------------------

/// Request header (fixed 12 bytes).
///
/// Layout:
///   [0]     op: StorageOp
///   [1]     partition_index: u8 (for partition ops) / flags
///   [2..4]  reserved
///   [4..8]  address: u32 (LE) — flash address or partition offset
///   [8..12] length: u32 (LE) — byte count for read/erase, or payload length for write
///   [12..]  payload (for write ops)
#[repr(C)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct StorageRequestHeader {
    pub op: u8,
    pub partition_index: u8,
    pub flags: u8,
    pub _reserved: u8,
    pub address: [u8; 4],
    pub length: [u8; 4],
}

impl StorageRequestHeader {
    pub const SIZE: usize = 12;

    pub fn op(&self) -> Option<StorageOp> {
        StorageOp::from_u8(self.op)
    }

    pub fn address(&self) -> u32 {
        u32::from_le_bytes(self.address)
    }

    pub fn length(&self) -> u32 {
        u32::from_le_bytes(self.length)
    }
}

/// Response header (fixed 8 bytes).
///
/// Layout:
///   [0]     error: StorageError
///   [1..4]  reserved
///   [4..8]  length: u32 (LE) — response payload length
///   [8..]   payload (for read responses)
#[repr(C)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct StorageResponseHeader {
    pub error: u8,
    pub _reserved: [u8; 3],
    pub length: [u8; 4],
}

impl StorageResponseHeader {
    pub const SIZE: usize = 8;

    pub fn success(payload_len: u32) -> Self {
        Self {
            error: StorageError::Ok as u8,
            _reserved: [0; 3],
            length: payload_len.to_le_bytes(),
        }
    }

    pub fn error(err: StorageError) -> Self {
        Self {
            error: err as u8,
            _reserved: [0; 3],
            length: [0; 4],
        }
    }

    pub fn payload_length(&self) -> u32 {
        u32::from_le_bytes(self.length)
    }
}

// ---------------------------------------------------------------------------
// Boot config types (from Caliptra)
// ---------------------------------------------------------------------------

/// Boot partition identifier.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionId {
    None = 0,
    A = 1,
    B = 2,
}

impl PartitionId {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::A),
            2 => Some(Self::B),
            _ => None,
        }
    }
}

/// Boot partition status.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionStatus {
    Invalid = 0,
    Valid = 1,
    BootFailed = 2,
    BootSuccessful = 3,
}

impl PartitionStatus {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Invalid),
            1 => Some(Self::Valid),
            2 => Some(Self::BootFailed),
            3 => Some(Self::BootSuccessful),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Partition info (returned by ListPartitions / GetPartitionInfo)
// ---------------------------------------------------------------------------

/// Partition descriptor returned by the server.
#[repr(C)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct PartitionInfo {
    /// Null-terminated partition name (max 16 bytes)
    pub name: [u8; MAX_PARTITION_NAME_LEN],
    /// Base offset in flash
    pub base_offset: [u8; 4],
    /// Partition size in bytes
    pub size: [u8; 4],
    /// Partition index
    pub index: u8,
    /// Flags (reserved)
    pub flags: u8,
    pub _reserved: [u8; 2],
}

impl PartitionInfo {
    pub const SIZE: usize = 28;

    pub fn base_offset(&self) -> u32 {
        u32::from_le_bytes(self.base_offset)
    }

    pub fn size(&self) -> u32 {
        u32::from_le_bytes(self.size)
    }
}
