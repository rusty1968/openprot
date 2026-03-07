// Licensed under the Apache-2.0 license

//! Storage Backend Traits
//!
//! The flash backend interface is the standard `embedded_storage::nor_flash::NorFlash`
//! trait — no custom wrapper.  This module re-exports it for convenience and
//! defines the `BootConfig` trait for A/B partition management.

// Re-export the standard NOR flash traits so downstream crates can
// depend on `storage_api::backend::nor_flash` without adding
// `embedded-storage` directly.
pub mod nor_flash {
    pub use embedded_storage::nor_flash::*;
}

use crate::StorageError;
use nor_flash::NorFlashErrorKind;

/// Map a [`NorFlashErrorKind`] to the IPC-level [`StorageError`].
pub fn nor_flash_err_to_storage(kind: NorFlashErrorKind) -> StorageError {
    match kind {
        NorFlashErrorKind::NotAligned => StorageError::InvalidParam,
        NorFlashErrorKind::OutOfBounds => StorageError::OutOfBounds,
        _ => StorageError::Failed,
    }
}

/// Partition definition for flash storage.
///
/// Inspired by Caliptra's `FlashPartition` struct.
#[derive(Debug, Clone, Copy)]
pub struct PartitionDef {
    /// Partition name (null-terminated, max 16 bytes)
    pub name: &'static str,
    /// Base offset in flash
    pub base_offset: usize,
    /// Partition size in bytes
    pub length: usize,
}

/// Boot configuration interface.
///
/// Mirrors Caliptra MCU-SW's `BootConfig` trait for managing A/B
/// partition boot state, boot counts, and rollback policy.
pub trait BootConfig {
    /// Get the active boot partition.
    fn get_active_partition(&self) -> Result<BootPartitionId, BootConfigError>;

    /// Set the active boot partition.
    fn set_active_partition(&mut self, partition: BootPartitionId) -> Result<(), BootConfigError>;

    /// Get partition status.
    fn get_partition_status(
        &self,
        partition: BootPartitionId,
    ) -> Result<BootPartitionStatus, BootConfigError>;

    /// Set partition status.
    fn set_partition_status(
        &mut self,
        partition: BootPartitionId,
        status: BootPartitionStatus,
    ) -> Result<(), BootConfigError>;

    /// Get the boot count for a partition.
    fn get_boot_count(&self, partition: BootPartitionId) -> Result<u16, BootConfigError>;

    /// Increment the boot count for a partition.
    fn increment_boot_count(&mut self, partition: BootPartitionId) -> Result<u16, BootConfigError>;

    /// Check if rollback is enabled.
    fn is_rollback_enabled(&self) -> Result<bool, BootConfigError>;

    /// Enable or disable rollback.
    fn set_rollback_enable(&mut self, enable: bool) -> Result<(), BootConfigError>;

    /// Persist boot config changes to flash.
    fn persist(&mut self) -> Result<(), BootConfigError>;
}

/// Boot partition identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootPartitionId {
    A,
    B,
}

/// Boot partition status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootPartitionStatus {
    Invalid,
    Valid,
    BootFailed,
    BootSuccessful,
}

/// Boot config error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootConfigError {
    InvalidPartition,
    InvalidStatus,
    StorageError,
    ReadFailed,
    WriteFailed,
}
