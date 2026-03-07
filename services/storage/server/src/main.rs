// Licensed under the Apache-2.0 license

//! Storage Server Application
//!
//! A userspace storage service that handles IPC requests from clients.
//! The flash backend is selected via Cargo feature flags (currently: `aspeed`).

#![no_main]
#![no_std]

use core::cell::RefCell;

use embedded_storage::nor_flash::{NorFlash, NorFlashError, ReadNorFlash};

use storage_api::{
    StorageError, StorageOp, StorageRequestHeader, StorageResponseHeader,
    PartitionInfo, MAX_PARTITION_NAME_LEN,
};
use storage_api::backend::{
    BootConfig, BootPartitionId, BootPartitionStatus,
    PartitionDef, nor_flash_err_to_storage,
};
use pw_status::Result;

use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

use app_storage_server::handle;

const MAX_REQUEST_SIZE: usize = 1024;
const MAX_RESPONSE_SIZE: usize = 1024;

// ---------------------------------------------------------------------------
// Feature-gated backend selection
// ---------------------------------------------------------------------------

#[cfg(feature = "aspeed")]
mod backend {
    use aspeed_ddk::spi::aspeed_norflash::AspeedNorFlash;
    use aspeed_ddk::spi::device::ChipSelectDevice;
    use aspeed_ddk::spi::fmccontroller::FmcController;
    use aspeed_ddk::spi::{SpiConfig, SpiData};
    use aspeed_ddk::spimonitor::SpipfNone;

    // TODO: Replace with actual peripheral access for your platform.
    //       This requires the FMC register block pointer and SCU config.
    //       The exact construction depends on how pw_kernel exposes
    //       memory-mapped peripherals to userspace.
    //
    // Example (when register access is available):
    //
    //   let fmc_regs = unsafe { &*ast1060_pac::Fmc::ptr() };
    //   let scu_regs = unsafe { &*ast1060_pac::Scu::ptr() };
    //   let mut fmc = FmcController::new(fmc_regs, 0, spi_config, SpiData::new(), None);
    //   fmc.init().unwrap();
    //   let cs_dev = ChipSelectDevice { bus: &mut fmc, cs: 0, spi_monitor: None::<SpipfNone> };
    //   AspeedNorFlash::new(cs_dev).unwrap()
    //
    // For now this is a placeholder that will be filled in when the
    // platform's peripheral mapping is wired up.

    pub type Flash<'a> = AspeedNorFlash<ChipSelectDevice<'a, FmcController<'a>, SpipfNone>>;
}

#[cfg(not(any(feature = "aspeed")))]
compile_error!(
    "No storage backend selected. Enable a backend feature, e.g. `--features aspeed`."
);

// ---------------------------------------------------------------------------
// Partition table
// ---------------------------------------------------------------------------

/// Default partition table.
///
/// Mirrors the Caliptra emulator's partition layout but simplified.
/// Works for both QEMU and real hardware — the flash capacity just
/// needs to be large enough to cover all partitions.
const PARTITIONS: &[PartitionDef] = &[
    PartitionDef {
        name: "boot_config",
        base_offset: 0x0000_0000,
        length: 0x0001_0000, // 64KB
    },
    PartitionDef {
        name: "image_a",
        base_offset: 0x0001_0000,
        length: 0x0008_0000, // 512KB
    },
    PartitionDef {
        name: "image_b",
        base_offset: 0x0009_0000,
        length: 0x0008_0000, // 512KB
    },
    PartitionDef {
        name: "log",
        base_offset: 0x0011_0000,
        length: 0x0002_0000, // 128KB
    },
];

// ---------------------------------------------------------------------------
// In-memory BootConfig (for initial bring-up / QEMU testing)
// ---------------------------------------------------------------------------

/// In-memory boot configuration.
///
/// Stores A/B partition state in RAM. For production use, this should
/// be persisted to the `boot_config` flash partition.
struct MemBootConfig {
    active: BootPartitionId,
    status_a: BootPartitionStatus,
    status_b: BootPartitionStatus,
    boot_count_a: u16,
    boot_count_b: u16,
    rollback_enabled: bool,
}

impl MemBootConfig {
    fn new() -> Self {
        Self {
            active: BootPartitionId::A,
            status_a: BootPartitionStatus::Valid,
            status_b: BootPartitionStatus::Invalid,
            boot_count_a: 0,
            boot_count_b: 0,
            rollback_enabled: false,
        }
    }
}

impl BootConfig for MemBootConfig {
    fn get_active_partition(&self) -> Result<BootPartitionId, storage_api::backend::BootConfigError> {
        Ok(self.active)
    }

    fn set_active_partition(&mut self, partition: BootPartitionId) -> Result<(), storage_api::backend::BootConfigError> {
        self.active = partition;
        Ok(())
    }

    fn get_partition_status(
        &self,
        partition: BootPartitionId,
    ) -> Result<BootPartitionStatus, storage_api::backend::BootConfigError> {
        match partition {
            BootPartitionId::A => Ok(self.status_a),
            BootPartitionId::B => Ok(self.status_b),
        }
    }

    fn set_partition_status(
        &mut self,
        partition: BootPartitionId,
        status: BootPartitionStatus,
    ) -> Result<(), storage_api::backend::BootConfigError> {
        match partition {
            BootPartitionId::A => self.status_a = status,
            BootPartitionId::B => self.status_b = status,
        }
        Ok(())
    }

    fn get_boot_count(&self, partition: BootPartitionId) -> Result<u16, storage_api::backend::BootConfigError> {
        match partition {
            BootPartitionId::A => Ok(self.boot_count_a),
            BootPartitionId::B => Ok(self.boot_count_b),
        }
    }

    fn increment_boot_count(&mut self, partition: BootPartitionId) -> Result<u16, storage_api::backend::BootConfigError> {
        match partition {
            BootPartitionId::A => {
                self.boot_count_a = self.boot_count_a.wrapping_add(1);
                Ok(self.boot_count_a)
            }
            BootPartitionId::B => {
                self.boot_count_b = self.boot_count_b.wrapping_add(1);
                Ok(self.boot_count_b)
            }
        }
    }

    fn is_rollback_enabled(&self) -> Result<bool, storage_api::backend::BootConfigError> {
        Ok(self.rollback_enabled)
    }

    fn set_rollback_enable(&mut self, enable: bool) -> Result<(), storage_api::backend::BootConfigError> {
        self.rollback_enabled = enable;
        Ok(())
    }

    fn persist(&mut self) -> Result<(), storage_api::backend::BootConfigError> {
        // In-memory: nothing to persist
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Server loop
// ---------------------------------------------------------------------------

fn storage_server_loop<F: NorFlash>(flash: &RefCell<F>) -> Result<()>
where
    F::Error: NorFlashError,
{
    pw_log::info!("Storage server starting");

    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut response_buf = [0u8; MAX_RESPONSE_SIZE];
    let mut boot_config = MemBootConfig::new();

    loop {
        // Wait for an IPC request
        syscall::object_wait(handle::STORAGE, Signals::READABLE, Instant::MAX)?;

        // Read the request
        let len = syscall::channel_read(handle::STORAGE, 0, &mut request_buf)?;

        if len < StorageRequestHeader::SIZE {
            let header = StorageResponseHeader::error(StorageError::InvalidDataLength);
            let header_bytes = zerocopy::IntoBytes::as_bytes(&header);
            response_buf[..StorageResponseHeader::SIZE].copy_from_slice(header_bytes);
            syscall::channel_respond(
                handle::STORAGE,
                &response_buf[..StorageResponseHeader::SIZE],
            )?;
            continue;
        }

        // Parse and dispatch
        let response_len = dispatch_storage_op(
            &request_buf[..len],
            &mut response_buf,
            flash,
            &mut boot_config,
            PARTITIONS,
        );
        syscall::channel_respond(handle::STORAGE, &response_buf[..response_len])?;
    }
}

fn dispatch_storage_op<F: NorFlash>(
    request: &[u8],
    response: &mut [u8],
    flash: &RefCell<F>,
    boot_config: &mut dyn BootConfig,
    partitions: &[PartitionDef],
) -> usize
where
    F::Error: NorFlashError,
{
    // Parse header
    let header_bytes = &request[..StorageRequestHeader::SIZE];
    let Some(header) = zerocopy::Ref::<_, StorageRequestHeader>::from_bytes(header_bytes).ok()
    else {
        return encode_error(response, StorageError::InvalidDataLength);
    };
    let header: &StorageRequestHeader = &*header;

    let op = match header.op() {
        Some(op) => op,
        None => return encode_error(response, StorageError::UnknownOp),
    };

    let payload = &request[StorageRequestHeader::SIZE..];
    let address = header.address();
    let length = header.length();
    let partition_index = header.partition_index;

    match op {
        // ---- Raw flash operations ----
        StorageOp::Read => do_flash_read(flash, address, length, response),
        StorageOp::Write => do_flash_write(flash, address, payload, length, response),
        StorageOp::Erase => do_flash_erase(flash, address, length, response),
        StorageOp::GetCapacity => do_get_capacity(flash, response),

        // ---- Partition operations ----
        StorageOp::ListPartitions => do_list_partitions(partitions, response),
        StorageOp::GetPartitionInfo => do_get_partition_info(partitions, partition_index, response),
        StorageOp::PartitionRead => {
            do_partition_read(flash, partitions, partition_index, address, length, response)
        }
        StorageOp::PartitionWrite => {
            do_partition_write(flash, partitions, partition_index, address, payload, length, response)
        }
        StorageOp::PartitionErase => {
            do_partition_erase(flash, partitions, partition_index, address, length, response)
        }

        // ---- Boot config operations ----
        StorageOp::GetActivePartition => do_get_active(boot_config, response),
        StorageOp::SetActivePartition => do_set_active(boot_config, partition_index, response),
        StorageOp::GetPartitionStatus => do_get_status(boot_config, partition_index, response),
        StorageOp::SetPartitionStatus => {
            do_set_status(boot_config, partition_index, address, response)
        }
        StorageOp::GetBootCount => do_get_boot_count(boot_config, partition_index, response),
        StorageOp::IncrementBootCount => {
            do_increment_boot_count(boot_config, partition_index, response)
        }
        StorageOp::IsRollbackEnabled => do_is_rollback(boot_config, response),
        StorageOp::SetRollbackEnable => do_set_rollback(boot_config, partition_index, response),
        StorageOp::PersistBootConfig => do_persist(boot_config, response),
    }
}

// ---------------------------------------------------------------------------
// Flash operations — generic over F: NorFlash
// ---------------------------------------------------------------------------

fn do_flash_read<F: NorFlash>(
    flash: &RefCell<F>,
    address: u32,
    length: u32,
    response: &mut [u8],
) -> usize
where
    F::Error: NorFlashError,
{
    let len = length as usize;
    if StorageResponseHeader::SIZE + len > response.len() {
        return encode_error(response, StorageError::OutOfBounds);
    }
    let output = &mut response[StorageResponseHeader::SIZE..StorageResponseHeader::SIZE + len];
    match flash.borrow_mut().read(address, output) {
        Ok(()) => encode_success(response, len as u32),
        Err(e) => encode_error(response, nor_flash_err_to_storage(e.kind())),
    }
}

fn do_flash_write<F: NorFlash>(
    flash: &RefCell<F>,
    address: u32,
    payload: &[u8],
    length: u32,
    response: &mut [u8],
) -> usize
where
    F::Error: NorFlashError,
{
    let len = length as usize;
    if payload.len() < len {
        return encode_error(response, StorageError::InvalidDataLength);
    }
    match flash.borrow_mut().write(address, &payload[..len]) {
        Ok(()) => encode_success(response, 0),
        Err(e) => encode_error(response, nor_flash_err_to_storage(e.kind())),
    }
}

fn do_flash_erase<F: NorFlash>(
    flash: &RefCell<F>,
    address: u32,
    length: u32,
    response: &mut [u8],
) -> usize
where
    F::Error: NorFlashError,
{
    match flash.borrow_mut().erase(address, address + length) {
        Ok(()) => encode_success(response, 0),
        Err(e) => encode_error(response, nor_flash_err_to_storage(e.kind())),
    }
}

fn do_get_capacity<F: NorFlash>(flash: &RefCell<F>, response: &mut [u8]) -> usize
where
    F::Error: NorFlashError,
{
    let cap = flash.borrow().capacity() as u32;
    let cap_bytes = cap.to_le_bytes();
    response[StorageResponseHeader::SIZE..StorageResponseHeader::SIZE + 4]
        .copy_from_slice(&cap_bytes);
    encode_success(response, 4)
}

// ---------------------------------------------------------------------------
// Partition operations
// ---------------------------------------------------------------------------

fn do_list_partitions(partitions: &[PartitionDef], response: &mut [u8]) -> usize {
    // Return count as u8
    let count = partitions.len() as u8;
    response[StorageResponseHeader::SIZE] = count;
    encode_success(response, 1)
}

fn do_get_partition_info(
    partitions: &[PartitionDef],
    index: u8,
    response: &mut [u8],
) -> usize {
    let idx = index as usize;
    if idx >= partitions.len() {
        return encode_error(response, StorageError::PartitionNotFound);
    }
    let p = &partitions[idx];
    let info = partition_def_to_info(p, index);
    let info_bytes = zerocopy::IntoBytes::as_bytes(&info);
    response[StorageResponseHeader::SIZE..StorageResponseHeader::SIZE + PartitionInfo::SIZE]
        .copy_from_slice(info_bytes);
    encode_success(response, PartitionInfo::SIZE as u32)
}

fn do_partition_read<F: NorFlash>(
    flash: &RefCell<F>,
    partitions: &[PartitionDef],
    index: u8,
    offset: u32,
    length: u32,
    response: &mut [u8],
) -> usize
where
    F::Error: NorFlashError,
{
    let idx = index as usize;
    if idx >= partitions.len() {
        return encode_error(response, StorageError::PartitionNotFound);
    }
    let p = &partitions[idx];
    let end = offset as usize + length as usize;
    if end > p.length {
        return encode_error(response, StorageError::OutOfBounds);
    }
    let abs_addr = p.base_offset as u32 + offset;
    do_flash_read(flash, abs_addr, length, response)
}

fn do_partition_write<F: NorFlash>(
    flash: &RefCell<F>,
    partitions: &[PartitionDef],
    index: u8,
    offset: u32,
    payload: &[u8],
    length: u32,
    response: &mut [u8],
) -> usize
where
    F::Error: NorFlashError,
{
    let idx = index as usize;
    if idx >= partitions.len() {
        return encode_error(response, StorageError::PartitionNotFound);
    }
    let p = &partitions[idx];
    let end = offset as usize + length as usize;
    if end > p.length {
        return encode_error(response, StorageError::OutOfBounds);
    }
    let abs_addr = p.base_offset as u32 + offset;
    do_flash_write(flash, abs_addr, payload, length, response)
}

fn do_partition_erase<F: NorFlash>(
    flash: &RefCell<F>,
    partitions: &[PartitionDef],
    index: u8,
    offset: u32,
    length: u32,
    response: &mut [u8],
) -> usize
where
    F::Error: NorFlashError,
{
    let idx = index as usize;
    if idx >= partitions.len() {
        return encode_error(response, StorageError::PartitionNotFound);
    }
    let p = &partitions[idx];
    let end = offset as usize + length as usize;
    if end > p.length {
        return encode_error(response, StorageError::OutOfBounds);
    }
    let abs_addr = p.base_offset as u32 + offset;
    do_flash_erase(flash, abs_addr, length, response)
}

// ---------------------------------------------------------------------------
// Boot config operations
// ---------------------------------------------------------------------------

fn partition_id_from_index(index: u8) -> Option<BootPartitionId> {
    match index {
        1 => Some(BootPartitionId::A),
        2 => Some(BootPartitionId::B),
        _ => None,
    }
}

fn boot_partition_id_to_u8(id: BootPartitionId) -> u8 {
    match id {
        BootPartitionId::A => 1,
        BootPartitionId::B => 2,
    }
}

fn boot_status_from_u32(v: u32) -> Option<BootPartitionStatus> {
    match v {
        0 => Some(BootPartitionStatus::Invalid),
        1 => Some(BootPartitionStatus::Valid),
        2 => Some(BootPartitionStatus::BootFailed),
        3 => Some(BootPartitionStatus::BootSuccessful),
        _ => None,
    }
}

fn boot_status_to_u8(s: BootPartitionStatus) -> u8 {
    match s {
        BootPartitionStatus::Invalid => 0,
        BootPartitionStatus::Valid => 1,
        BootPartitionStatus::BootFailed => 2,
        BootPartitionStatus::BootSuccessful => 3,
    }
}

fn do_get_active(boot_config: &dyn BootConfig, response: &mut [u8]) -> usize {
    match boot_config.get_active_partition() {
        Ok(id) => {
            response[StorageResponseHeader::SIZE] = boot_partition_id_to_u8(id);
            encode_success(response, 1)
        }
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

fn do_set_active(boot_config: &mut dyn BootConfig, index: u8, response: &mut [u8]) -> usize {
    let Some(id) = partition_id_from_index(index) else {
        return encode_error(response, StorageError::InvalidParam);
    };
    match boot_config.set_active_partition(id) {
        Ok(()) => encode_success(response, 0),
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

fn do_get_status(boot_config: &dyn BootConfig, index: u8, response: &mut [u8]) -> usize {
    let Some(id) = partition_id_from_index(index) else {
        return encode_error(response, StorageError::InvalidParam);
    };
    match boot_config.get_partition_status(id) {
        Ok(status) => {
            response[StorageResponseHeader::SIZE] = boot_status_to_u8(status);
            encode_success(response, 1)
        }
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

fn do_set_status(
    boot_config: &mut dyn BootConfig,
    index: u8,
    status_val: u32,
    response: &mut [u8],
) -> usize {
    let Some(id) = partition_id_from_index(index) else {
        return encode_error(response, StorageError::InvalidParam);
    };
    let Some(status) = boot_status_from_u32(status_val) else {
        return encode_error(response, StorageError::InvalidParam);
    };
    match boot_config.set_partition_status(id, status) {
        Ok(()) => encode_success(response, 0),
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

fn do_get_boot_count(boot_config: &dyn BootConfig, index: u8, response: &mut [u8]) -> usize {
    let Some(id) = partition_id_from_index(index) else {
        return encode_error(response, StorageError::InvalidParam);
    };
    match boot_config.get_boot_count(id) {
        Ok(count) => {
            let bytes = count.to_le_bytes();
            response[StorageResponseHeader::SIZE..StorageResponseHeader::SIZE + 2]
                .copy_from_slice(&bytes);
            encode_success(response, 2)
        }
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

fn do_increment_boot_count(
    boot_config: &mut dyn BootConfig,
    index: u8,
    response: &mut [u8],
) -> usize {
    let Some(id) = partition_id_from_index(index) else {
        return encode_error(response, StorageError::InvalidParam);
    };
    match boot_config.increment_boot_count(id) {
        Ok(count) => {
            let bytes = count.to_le_bytes();
            response[StorageResponseHeader::SIZE..StorageResponseHeader::SIZE + 2]
                .copy_from_slice(&bytes);
            encode_success(response, 2)
        }
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

fn do_is_rollback(boot_config: &dyn BootConfig, response: &mut [u8]) -> usize {
    match boot_config.is_rollback_enabled() {
        Ok(enabled) => {
            response[StorageResponseHeader::SIZE] = if enabled { 1 } else { 0 };
            encode_success(response, 1)
        }
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

fn do_set_rollback(boot_config: &mut dyn BootConfig, enable: u8, response: &mut [u8]) -> usize {
    match boot_config.set_rollback_enable(enable != 0) {
        Ok(()) => encode_success(response, 0),
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

fn do_persist(boot_config: &mut dyn BootConfig, response: &mut [u8]) -> usize {
    match boot_config.persist() {
        Ok(()) => encode_success(response, 0),
        Err(_) => encode_error(response, StorageError::BootConfigError),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn encode_error(response: &mut [u8], err: StorageError) -> usize {
    let header = StorageResponseHeader::error(err);
    let header_bytes = zerocopy::IntoBytes::as_bytes(&header);
    response[..StorageResponseHeader::SIZE].copy_from_slice(header_bytes);
    StorageResponseHeader::SIZE
}

fn encode_success(response: &mut [u8], payload_len: u32) -> usize {
    let header = StorageResponseHeader::success(payload_len);
    let header_bytes = zerocopy::IntoBytes::as_bytes(&header);
    response[..StorageResponseHeader::SIZE].copy_from_slice(header_bytes);
    StorageResponseHeader::SIZE + payload_len as usize
}

fn partition_def_to_info(p: &PartitionDef, index: u8) -> PartitionInfo {
    let mut name = [0u8; MAX_PARTITION_NAME_LEN];
    let name_bytes = p.name.as_bytes();
    let copy_len = core::cmp::min(name_bytes.len(), MAX_PARTITION_NAME_LEN - 1);
    name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
    PartitionInfo {
        name,
        base_offset: (p.base_offset as u32).to_le_bytes(),
        size: (p.length as u32).to_le_bytes(),
        index,
        flags: 0,
        _reserved: [0; 2],
    }
}

#[entry]
fn entry() -> ! {
    // TODO: Construct the actual AspeedNorFlash from FMC controller.
    //       See `mod backend` for the construction pattern.
    //       For now, the entry point shows the generic plumbing.
    //
    // let flash = RefCell::new(backend::create_flash());
    // if let Err(e) = storage_server_loop(&flash) { ... }

    // Placeholder — will be replaced when platform peripheral access is wired up.
    pw_log::error!("Storage server: backend construction not yet wired up");
    let _ = syscall::debug_shutdown(Err(pw_status::Error::Unimplemented));

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
