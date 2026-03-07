// Licensed under the Apache-2.0 license

//! Storage Client Test Application
//!
//! Tests the storage server by making IPC requests for flash I/O,
//! partition operations, and boot configuration management.

#![no_main]
#![no_std]

use app_storage_client::handle;
use storage_api::{PartitionId, PartitionStatus};
use storage_client::StorageClient;
use pw_status::{Result, Error};
use userspace::entry;
use userspace::syscall;

// ---------------------------------------------------------------------------
// Flash I/O tests
// ---------------------------------------------------------------------------

fn test_flash_capacity(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing GetCapacity...");

    let cap = storage.get_capacity().map_err(|_| Error::Internal)?;
    // QEMU_PARTITIONS total: 64KB + 512KB + 512KB + 128KB = 1216KB = 0x130000
    // Actually MemFlash<FLASH_SIZE> where FLASH_SIZE = 0x120000
    pw_log::info!("Flash capacity: {} bytes", cap);

    if cap == 0 {
        pw_log::error!("Capacity is zero!");
        return Err(Error::Unknown);
    }

    pw_log::info!("GetCapacity: PASS");
    Ok(())
}

fn test_flash_write_read(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing flash write + read...");

    let test_data: [u8; 16] = [
        0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE,
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
    ];
    let address: u32 = 0x1000;

    // Erase first (so we can write)
    storage.erase(address, 4096).map_err(|_| Error::Internal)?;

    // Write
    storage.write(address, &test_data).map_err(|_| Error::Internal)?;

    // Read back
    let mut readback = [0u8; 16];
    storage.read(address, &mut readback).map_err(|_| Error::Internal)?;

    if readback != test_data {
        pw_log::error!("Flash read != write!");
        pw_log::error!(
            "Expected: {:02x} {:02x} {:02x} {:02x}",
            test_data[0] as u32,
            test_data[1] as u32,
            test_data[2] as u32,
            test_data[3] as u32,
        );
        pw_log::error!(
            "Got:      {:02x} {:02x} {:02x} {:02x}",
            readback[0] as u32,
            readback[1] as u32,
            readback[2] as u32,
            readback[3] as u32,
        );
        return Err(Error::Unknown);
    }

    pw_log::info!("Flash write+read: PASS");
    Ok(())
}

fn test_flash_erase(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing flash erase...");

    let address: u32 = 0x2000;

    // Write some data
    let data = [0x42u8; 32];
    storage.erase(address, 4096).map_err(|_| Error::Internal)?;
    storage.write(address, &data).map_err(|_| Error::Internal)?;

    // Erase
    storage.erase(address, 4096).map_err(|_| Error::Internal)?;

    // Read back — should be 0xFF (erased)
    let mut readback = [0u8; 32];
    storage.read(address, &mut readback).map_err(|_| Error::Internal)?;

    if readback.iter().any(|&b| b != 0xFF) {
        pw_log::error!("Erase did not set all bytes to 0xFF!");
        return Err(Error::Unknown);
    }

    pw_log::info!("Flash erase: PASS");
    Ok(())
}

fn test_flash_and_semantics(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing flash AND-write semantics...");

    let address: u32 = 0x3000;

    // Erase
    storage.erase(address, 4096).map_err(|_| Error::Internal)?;

    // Write 0xFF (should keep erased state)
    let data1 = [0xFFu8; 4];
    storage.write(address, &data1).map_err(|_| Error::Internal)?;

    // Write 0xF0 (should AND with 0xFF → 0xF0)
    let data2 = [0xF0u8; 4];
    storage.write(address, &data2).map_err(|_| Error::Internal)?;

    // Write 0x0F (should AND with 0xF0 → 0x00)
    let data3 = [0x0Fu8; 4];
    storage.write(address, &data3).map_err(|_| Error::Internal)?;

    let mut readback = [0u8; 4];
    storage.read(address, &mut readback).map_err(|_| Error::Internal)?;

    if readback != [0x00, 0x00, 0x00, 0x00] {
        pw_log::error!("AND semantics failed!");
        pw_log::error!(
            "Expected: 00 00 00 00, got: {:02x} {:02x} {:02x} {:02x}",
            readback[0] as u32,
            readback[1] as u32,
            readback[2] as u32,
            readback[3] as u32,
        );
        return Err(Error::Unknown);
    }

    pw_log::info!("Flash AND-write: PASS");
    Ok(())
}

// ---------------------------------------------------------------------------
// Partition tests
// ---------------------------------------------------------------------------

fn test_partition_info(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing partition info...");

    // Partition 0 = boot_config (64KB at 0x00000000)
    let info = storage.get_partition_info(0).map_err(|_| Error::Internal)?;
    let size = info.size();
    pw_log::info!("Partition 0: size={} bytes", size);

    if size != 0x0001_0000 {
        pw_log::error!("Expected 64KB, got {} bytes", size);
        return Err(Error::Unknown);
    }

    // Partition 1 = image_a (512KB at 0x00010000)
    let info1 = storage.get_partition_info(1).map_err(|_| Error::Internal)?;
    let offset1 = info1.base_offset();
    pw_log::info!("Partition 1: base_offset=0x{:08x}", offset1);

    if offset1 != 0x0001_0000 {
        pw_log::error!("Expected offset 0x10000, got 0x{:08x}", offset1);
        return Err(Error::Unknown);
    }

    pw_log::info!("Partition info: PASS");
    Ok(())
}

fn test_partition_read_write(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing partition read/write...");

    // Use partition 3 (log, 128KB at 0x110000) for testing
    let part_idx: u8 = 3;
    let offset: u32 = 0;

    // Erase first 4KB of the log partition
    storage
        .partition_erase(part_idx, offset, 4096)
        .map_err(|_| Error::Internal)?;

    // Write test pattern
    let pattern: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    storage
        .partition_write(part_idx, offset, &pattern)
        .map_err(|_| Error::Internal)?;

    // Read back
    let mut readback = [0u8; 8];
    storage
        .partition_read(part_idx, offset, &mut readback)
        .map_err(|_| Error::Internal)?;

    if readback != pattern {
        pw_log::error!("Partition read != write!");
        return Err(Error::Unknown);
    }

    pw_log::info!("Partition read/write: PASS");
    Ok(())
}

// ---------------------------------------------------------------------------
// Boot config tests
// ---------------------------------------------------------------------------

fn test_boot_config_active(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing boot config active partition...");

    // Default active should be A
    let active = storage
        .get_active_partition()
        .map_err(|_| Error::Internal)?;

    if active != PartitionId::A {
        pw_log::error!("Default active should be A, got {:?}", active);
        return Err(Error::Unknown);
    }

    // Switch to B
    storage
        .set_active_partition(PartitionId::B)
        .map_err(|_| Error::Internal)?;

    let active2 = storage
        .get_active_partition()
        .map_err(|_| Error::Internal)?;

    if active2 != PartitionId::B {
        pw_log::error!("Expected B after set, got {:?}", active2);
        return Err(Error::Unknown);
    }

    // Switch back to A
    storage
        .set_active_partition(PartitionId::A)
        .map_err(|_| Error::Internal)?;

    pw_log::info!("Boot config active: PASS");
    Ok(())
}

fn test_boot_config_status(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing boot config partition status...");

    // Set partition A to BootSuccessful
    storage
        .set_partition_status(PartitionId::A, PartitionStatus::BootSuccessful)
        .map_err(|_| Error::Internal)?;

    let status = storage
        .get_partition_status(PartitionId::A)
        .map_err(|_| Error::Internal)?;

    if status != PartitionStatus::BootSuccessful {
        pw_log::error!("Expected BootSuccessful, got {:?}", status);
        return Err(Error::Unknown);
    }

    // Set partition B to Valid
    storage
        .set_partition_status(PartitionId::B, PartitionStatus::Valid)
        .map_err(|_| Error::Internal)?;

    let status_b = storage
        .get_partition_status(PartitionId::B)
        .map_err(|_| Error::Internal)?;

    if status_b != PartitionStatus::Valid {
        pw_log::error!("Expected Valid for B, got {:?}", status_b);
        return Err(Error::Unknown);
    }

    pw_log::info!("Boot config status: PASS");
    Ok(())
}

fn test_boot_count(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing boot count...");

    // Initial count should be 0
    let count0 = storage
        .get_boot_count(PartitionId::A)
        .map_err(|_| Error::Internal)?;

    pw_log::info!("Initial boot count A: {}", count0 as u32);

    // Increment
    let count1 = storage
        .increment_boot_count(PartitionId::A)
        .map_err(|_| Error::Internal)?;

    if count1 != count0 + 1 {
        pw_log::error!("Expected {}, got {}", (count0 + 1) as u32, count1 as u32);
        return Err(Error::Unknown);
    }

    // Increment again
    let count2 = storage
        .increment_boot_count(PartitionId::A)
        .map_err(|_| Error::Internal)?;

    if count2 != count1 + 1 {
        pw_log::error!("Expected {}, got {}", (count1 + 1) as u32, count2 as u32);
        return Err(Error::Unknown);
    }

    pw_log::info!("Boot count: PASS");
    Ok(())
}

fn test_rollback(storage: &StorageClient) -> Result<()> {
    pw_log::info!("Testing rollback config...");

    // Default should be disabled
    let enabled = storage
        .is_rollback_enabled()
        .map_err(|_| Error::Internal)?;

    if enabled {
        pw_log::error!("Rollback should be disabled by default!");
        return Err(Error::Unknown);
    }

    // Enable
    storage
        .set_rollback_enable(true)
        .map_err(|_| Error::Internal)?;

    let enabled2 = storage
        .is_rollback_enabled()
        .map_err(|_| Error::Internal)?;

    if !enabled2 {
        pw_log::error!("Rollback should be enabled after set!");
        return Err(Error::Unknown);
    }

    // Disable
    storage
        .set_rollback_enable(false)
        .map_err(|_| Error::Internal)?;

    // Persist (no-op for memory backend, but tests the path)
    storage
        .persist_boot_config()
        .map_err(|_| Error::Internal)?;

    pw_log::info!("Rollback config: PASS");
    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn run_storage_tests() -> Result<()> {
    pw_log::info!("Starting storage client tests");

    let storage = StorageClient::new(handle::STORAGE);

    // Flash I/O tests
    test_flash_capacity(&storage)?;
    test_flash_write_read(&storage)?;
    test_flash_erase(&storage)?;
    test_flash_and_semantics(&storage)?;

    // Partition tests
    test_partition_info(&storage)?;
    test_partition_read_write(&storage)?;

    // Boot config tests
    test_boot_config_active(&storage)?;
    test_boot_config_status(&storage)?;
    test_boot_count(&storage)?;
    test_rollback(&storage)?;

    pw_log::info!("All storage tests PASSED!");
    Ok(())
}

#[entry]
fn entry() -> ! {
    pw_log::info!("🔄 RUNNING");

    let ret = run_storage_tests();

    if ret.is_err() {
        pw_log::error!("❌ FAILED");
        let _ = syscall::debug_shutdown(ret);
    } else {
        pw_log::info!("✅ PASSED");
        let _ = syscall::debug_shutdown(Ok(()));
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
