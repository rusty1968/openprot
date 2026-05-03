// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#[path = "types.rs"]
pub mod types;

#[path = "helpers.rs"]
pub mod helpers;

pub mod fmc {
    use crate::smc::types::{ChipSelect, FlashConfig, SmcError, TransferMode};

    pub struct FmcReady;

    impl FmcReady {
        pub fn capacity_bytes(&self) -> Result<usize, SmcError> {
            Ok(8 * 1024 * 1024)
        }

        pub fn cs_capacity_bytes(&self, _cs: ChipSelect) -> Result<usize, SmcError> {
            Ok(8 * 1024 * 1024)
        }

        pub fn cs_config(&self, _cs: ChipSelect) -> Result<FlashConfig, SmcError> {
            Ok(FlashConfig::winbond_w25q64())
        }

        pub fn read(&self, _offset: u32, _buf: &mut [u8]) -> Result<usize, SmcError> {
            Ok(0)
        }

        pub fn transceive_user(
            &self,
            _cs: ChipSelect,
            _cmd: &[u8],
            _tx_payload: &[u8],
            _rx: &mut [u8],
            _mode: TransferMode,
        ) -> Result<(), SmcError> {
            Ok(())
        }
    }
}

pub mod spi {
    use crate::smc::types::{ChipSelect, FlashConfig, SmcError, TransferMode};

    pub struct SpiReady;

    impl SpiReady {
        pub fn capacity_bytes(&self) -> Result<usize, SmcError> {
            Ok(8 * 1024 * 1024)
        }

        pub fn cs_capacity_bytes(&self, _cs: ChipSelect) -> Result<usize, SmcError> {
            Ok(8 * 1024 * 1024)
        }

        pub fn cs_config(&self, _cs: ChipSelect) -> Result<FlashConfig, SmcError> {
            Ok(FlashConfig::winbond_w25q64())
        }

        pub fn read(&self, _offset: u32, _buf: &mut [u8]) -> Result<usize, SmcError> {
            Ok(0)
        }

        pub fn transceive_user(
            &self,
            _cs: ChipSelect,
            _cmd: &[u8],
            _tx_payload: &[u8],
            _rx: &mut [u8],
            _mode: TransferMode,
        ) -> Result<(), SmcError> {
            Ok(())
        }
    }
}

pub mod device {
    #[path = "flash.rs"]
    pub mod flash;

    #[path = "block_device.rs"]
    pub mod block_device;
}
