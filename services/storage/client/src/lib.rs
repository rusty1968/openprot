// Licensed under the Apache-2.0 license

//! Storage Client Library
//!
//! Provides an ergonomic API for applications to access the storage server
//! over IPC. Supports raw flash I/O, partition-based access, and boot
//! configuration management.
//!
//! # Quick Start
//!
//! ```ignore
//! use storage_client::StorageClient;
//!
//! let storage = StorageClient::new(handle::STORAGE);
//!
//! // Raw flash read
//! let mut buf = [0u8; 256];
//! storage.read(0x1000, &mut buf)?;
//!
//! // Partition-based write
//! storage.partition_write(1, 0, b"firmware data")?;
//!
//! // Boot config
//! let active = storage.get_active_partition()?;
//! storage.set_active_partition(PartitionId::B)?;
//! storage.persist_boot_config()?;
//! ```

#![no_std]

use storage_api::{
    PartitionId, PartitionInfo, PartitionStatus, StorageError, StorageOp,
    StorageRequestHeader, StorageResponseHeader, MAX_CHUNK_SIZE,
};
use userspace::syscall;
use userspace::time::Instant;

const MAX_BUF_SIZE: usize = 1024;

/// Error type for storage client operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    /// IPC syscall failed
    IpcError(pw_status::Error),
    /// Server returned an error
    ServerError(StorageError),
    /// Response too short or malformed
    InvalidResponse,
    /// Buffer too small for response data
    BufferTooSmall,
}

impl From<pw_status::Error> for ClientError {
    fn from(e: pw_status::Error) -> Self {
        ClientError::IpcError(e)
    }
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IpcError(e) => write!(f, "IPC: {:?}", e),
            Self::ServerError(e) => write!(f, "server: {:?}", e),
            Self::InvalidResponse => write!(f, "malformed response"),
            Self::BufferTooSmall => write!(f, "buffer too small"),
        }
    }
}

/// Storage IPC client.
///
/// Wraps an IPC channel handle to the storage server with typed methods
/// for flash I/O, partition operations, and boot configuration.
pub struct StorageClient {
    handle: u32,
}

impl StorageClient {
    /// Bind to the storage server channel.
    pub const fn new(handle: u32) -> Self {
        Self { handle }
    }

    // -----------------------------------------------------------------------
    // Raw flash operations
    // -----------------------------------------------------------------------

    /// Read `buf.len()` bytes from flash at `address`.
    pub fn read(&self, address: u32, buf: &mut [u8]) -> Result<(), ClientError> {
        let mut offset = 0usize;
        while offset < buf.len() {
            let chunk_len = core::cmp::min(buf.len() - offset, MAX_CHUNK_SIZE);
            let resp = self.transact_read(
                StorageOp::Read,
                0,
                address + offset as u32,
                chunk_len as u32,
            )?;
            let payload = resp.payload();
            if payload.len() < chunk_len {
                return Err(ClientError::InvalidResponse);
            }
            buf[offset..offset + chunk_len].copy_from_slice(&payload[..chunk_len]);
            offset += chunk_len;
        }
        Ok(())
    }

    /// Write `data` to flash at `address`.
    pub fn write(&self, address: u32, data: &[u8]) -> Result<(), ClientError> {
        let mut offset = 0usize;
        while offset < data.len() {
            let chunk_len = core::cmp::min(data.len() - offset, MAX_CHUNK_SIZE);
            self.transact_write(
                StorageOp::Write,
                0,
                address + offset as u32,
                &data[offset..offset + chunk_len],
            )?;
            offset += chunk_len;
        }
        Ok(())
    }

    /// Erase `length` bytes starting at `address`.
    pub fn erase(&self, address: u32, length: u32) -> Result<(), ClientError> {
        self.transact_simple(StorageOp::Erase, 0, address, length)
    }

    /// Get total flash capacity in bytes.
    pub fn get_capacity(&self) -> Result<u32, ClientError> {
        let resp = self.transact_read(StorageOp::GetCapacity, 0, 0, 0)?;
        let payload = resp.payload();
        if payload.len() < 4 {
            return Err(ClientError::InvalidResponse);
        }
        Ok(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
    }

    // -----------------------------------------------------------------------
    // Partition operations
    // -----------------------------------------------------------------------

    /// Read from a partition at the given offset.
    pub fn partition_read(
        &self,
        partition_index: u8,
        offset: u32,
        buf: &mut [u8],
    ) -> Result<(), ClientError> {
        let mut pos = 0usize;
        while pos < buf.len() {
            let chunk_len = core::cmp::min(buf.len() - pos, MAX_CHUNK_SIZE);
            let resp = self.transact_read(
                StorageOp::PartitionRead,
                partition_index,
                offset + pos as u32,
                chunk_len as u32,
            )?;
            let payload = resp.payload();
            if payload.len() < chunk_len {
                return Err(ClientError::InvalidResponse);
            }
            buf[pos..pos + chunk_len].copy_from_slice(&payload[..chunk_len]);
            pos += chunk_len;
        }
        Ok(())
    }

    /// Write to a partition at the given offset.
    pub fn partition_write(
        &self,
        partition_index: u8,
        offset: u32,
        data: &[u8],
    ) -> Result<(), ClientError> {
        let mut pos = 0usize;
        while pos < data.len() {
            let chunk_len = core::cmp::min(data.len() - pos, MAX_CHUNK_SIZE);
            self.transact_write(
                StorageOp::PartitionWrite,
                partition_index,
                offset + pos as u32,
                &data[pos..pos + chunk_len],
            )?;
            pos += chunk_len;
        }
        Ok(())
    }

    /// Erase within a partition.
    pub fn partition_erase(
        &self,
        partition_index: u8,
        offset: u32,
        length: u32,
    ) -> Result<(), ClientError> {
        self.transact_simple(StorageOp::PartitionErase, partition_index, offset, length)
    }

    /// Get partition info by index.
    pub fn get_partition_info(
        &self,
        partition_index: u8,
    ) -> Result<PartitionInfo, ClientError> {
        let resp = self.transact_read(
            StorageOp::GetPartitionInfo,
            partition_index,
            0,
            0,
        )?;
        let payload = resp.payload();
        if payload.len() < PartitionInfo::SIZE {
            return Err(ClientError::InvalidResponse);
        }
        let info = zerocopy::FromBytes::read_from_bytes(&payload[..PartitionInfo::SIZE])
            .map_err(|_| ClientError::InvalidResponse)?;
        Ok(info)
    }

    // -----------------------------------------------------------------------
    // Boot config operations
    // -----------------------------------------------------------------------

    /// Get the active boot partition.
    pub fn get_active_partition(&self) -> Result<PartitionId, ClientError> {
        let resp = self.transact_read(StorageOp::GetActivePartition, 0, 0, 0)?;
        let payload = resp.payload();
        if payload.is_empty() {
            return Err(ClientError::InvalidResponse);
        }
        PartitionId::from_u8(payload[0]).ok_or(ClientError::InvalidResponse)
    }

    /// Set the active boot partition.
    pub fn set_active_partition(&self, partition: PartitionId) -> Result<(), ClientError> {
        self.transact_simple(StorageOp::SetActivePartition, partition as u8, 0, 0)
    }

    /// Get partition boot status.
    pub fn get_partition_status(
        &self,
        partition: PartitionId,
    ) -> Result<PartitionStatus, ClientError> {
        let resp = self.transact_read(
            StorageOp::GetPartitionStatus,
            partition as u8,
            0,
            0,
        )?;
        let payload = resp.payload();
        if payload.is_empty() {
            return Err(ClientError::InvalidResponse);
        }
        PartitionStatus::from_u8(payload[0]).ok_or(ClientError::InvalidResponse)
    }

    /// Set partition boot status.
    pub fn set_partition_status(
        &self,
        partition: PartitionId,
        status: PartitionStatus,
    ) -> Result<(), ClientError> {
        self.transact_simple(
            StorageOp::SetPartitionStatus,
            partition as u8,
            status as u32,
            0,
        )
    }

    /// Get boot count for a partition.
    pub fn get_boot_count(&self, partition: PartitionId) -> Result<u16, ClientError> {
        let resp = self.transact_read(StorageOp::GetBootCount, partition as u8, 0, 0)?;
        let payload = resp.payload();
        if payload.len() < 2 {
            return Err(ClientError::InvalidResponse);
        }
        Ok(u16::from_le_bytes([payload[0], payload[1]]))
    }

    /// Increment boot count for a partition.
    pub fn increment_boot_count(&self, partition: PartitionId) -> Result<u16, ClientError> {
        let resp =
            self.transact_read(StorageOp::IncrementBootCount, partition as u8, 0, 0)?;
        let payload = resp.payload();
        if payload.len() < 2 {
            return Err(ClientError::InvalidResponse);
        }
        Ok(u16::from_le_bytes([payload[0], payload[1]]))
    }

    /// Check if rollback is enabled.
    pub fn is_rollback_enabled(&self) -> Result<bool, ClientError> {
        let resp = self.transact_read(StorageOp::IsRollbackEnabled, 0, 0, 0)?;
        let payload = resp.payload();
        if payload.is_empty() {
            return Err(ClientError::InvalidResponse);
        }
        Ok(payload[0] != 0)
    }

    /// Enable or disable rollback.
    pub fn set_rollback_enable(&self, enable: bool) -> Result<(), ClientError> {
        self.transact_simple(
            StorageOp::SetRollbackEnable,
            if enable { 1 } else { 0 },
            0,
            0,
        )
    }

    /// Persist boot config to flash.
    pub fn persist_boot_config(&self) -> Result<(), ClientError> {
        self.transact_simple(StorageOp::PersistBootConfig, 0, 0, 0)
    }

    // -----------------------------------------------------------------------
    // Internal helpers — single channel_transact round-trip
    // -----------------------------------------------------------------------

    /// Build a request header.
    fn build_header(
        op: StorageOp,
        partition_index: u8,
        address: u32,
        length: u32,
    ) -> StorageRequestHeader {
        StorageRequestHeader {
            op: op as u8,
            partition_index,
            flags: 0,
            _reserved: 0,
            address: address.to_le_bytes(),
            length: length.to_le_bytes(),
        }
    }

    /// Check a response for errors.
    fn check_response(buf: &[u8], len: usize) -> Result<(), ClientError> {
        if len < StorageResponseHeader::SIZE {
            return Err(ClientError::InvalidResponse);
        }
        let resp: StorageResponseHeader =
            zerocopy::FromBytes::read_from_bytes(&buf[..StorageResponseHeader::SIZE])
                .map_err(|_| ClientError::InvalidResponse)?;
        if resp.error != StorageError::Ok as u8 {
            let err = StorageError::from_u8(resp.error).unwrap_or(StorageError::Failed);
            return Err(ClientError::ServerError(err));
        }
        Ok(())
    }

    /// Simple op: send header, expect header-only response.
    fn transact_simple(
        &self,
        op: StorageOp,
        partition_index: u8,
        address: u32,
        length: u32,
    ) -> Result<(), ClientError> {
        let header = Self::build_header(op, partition_index, address, length);
        let header_bytes = zerocopy::IntoBytes::as_bytes(&header);
        let mut request = [0u8; StorageRequestHeader::SIZE];
        request.copy_from_slice(header_bytes);

        let mut response = [0u8; StorageResponseHeader::SIZE];
        let rlen = syscall::channel_transact(
            self.handle,
            &request,
            &mut response,
            Instant::MAX,
        )?;
        Self::check_response(&response, rlen)
    }

    /// Read op: send header, expect header + payload response.
    fn transact_read(
        &self,
        op: StorageOp,
        partition_index: u8,
        address: u32,
        length: u32,
    ) -> Result<ResponseBuf, ClientError> {
        let header = Self::build_header(op, partition_index, address, length);
        let header_bytes = zerocopy::IntoBytes::as_bytes(&header);
        let mut request = [0u8; StorageRequestHeader::SIZE];
        request.copy_from_slice(header_bytes);

        let mut response = ResponseBuf::new();
        let rlen = syscall::channel_transact(
            self.handle,
            &request,
            &mut response.buf,
            Instant::MAX,
        )?;
        response.len = rlen;
        Self::check_response(&response.buf, rlen)?;
        Ok(response)
    }

    /// Write op: send header + payload, expect header-only response.
    fn transact_write(
        &self,
        op: StorageOp,
        partition_index: u8,
        address: u32,
        data: &[u8],
    ) -> Result<(), ClientError> {
        let header = Self::build_header(op, partition_index, address, data.len() as u32);
        let header_bytes = zerocopy::IntoBytes::as_bytes(&header);
        let mut request = [0u8; MAX_BUF_SIZE];
        request[..StorageRequestHeader::SIZE].copy_from_slice(header_bytes);
        request[StorageRequestHeader::SIZE..StorageRequestHeader::SIZE + data.len()]
            .copy_from_slice(data);
        let total = StorageRequestHeader::SIZE + data.len();

        let mut response = [0u8; StorageResponseHeader::SIZE];
        let rlen = syscall::channel_transact(
            self.handle,
            &request[..total],
            &mut response,
            Instant::MAX,
        )?;
        Self::check_response(&response, rlen)
    }
}

/// Internal buffer for responses with payload.
struct ResponseBuf {
    buf: [u8; MAX_BUF_SIZE],
    len: usize,
}

impl ResponseBuf {
    fn new() -> Self {
        Self {
            buf: [0u8; MAX_BUF_SIZE],
            len: 0,
        }
    }

    fn payload(&self) -> &[u8] {
        if self.len > StorageResponseHeader::SIZE {
            &self.buf[StorageResponseHeader::SIZE..self.len]
        } else {
            &[]
        }
    }
}
