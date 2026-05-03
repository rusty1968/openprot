// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#[path = "smc/host_block_device_mod.rs"]
mod smc;

use smc::device::block_device::SpiNorBlockDevice;
use smc::device::flash::{JedecId, SpiNorFlash};
use smc::fmc::FmcReady;
use smc::types::{FlashConfig, SmcError};

#[test]
fn block_device_from_flash_constructs_with_matching_capacity() {
    let mut fmc = FmcReady;
    let mut flash = SpiNorFlash::from_fmc(&mut fmc, FlashConfig::winbond_w25q64())
        .expect("flash facade constructor should succeed on host stub");

    let block = SpiNorBlockDevice::from_flash(&mut flash, FlashConfig::winbond_w25q64())
        .expect("block facade constructor should succeed with matching config");

    let info = block.info().expect("info should be available");
    assert_eq!(info.capacity_bytes, 8 * 1024 * 1024);
    assert_eq!(info.page_size, 256);
    assert_eq!(info.sector_size, 4096);
}

#[test]
fn block_device_from_jedec_id_rejects_unknown_device() {
    let mut fmc = FmcReady;
    let mut flash = SpiNorFlash::from_fmc(&mut fmc, FlashConfig::winbond_w25q64())
        .expect("flash facade constructor should succeed on host stub");

    let unknown = JedecId::from_bytes([0x00, 0x00, 0x00]);
    match SpiNorBlockDevice::from_jedec_id(&mut flash, unknown) {
        Ok(_) => panic!("unknown JEDEC should be rejected"),
        Err(err) => assert_eq!(err, SmcError::DeviceNotSupported),
    }
}

fn main() {}
