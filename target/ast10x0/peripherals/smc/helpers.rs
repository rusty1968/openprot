// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Pure helper logic for SMC address and timing calculations.

use core::convert::TryFrom;

use crate::smc::types::ChipSelect;
use crate::smc::types::FlashConfig;
use crate::smc::types::SmcConfig;
use crate::smc::types::SmcError;

const SMC_WINDOW_SIZE_BYTES: usize = 256 * 1024 * 1024;
const DMA_DRAM_MASK: u32 = 0x000BFFFC;
pub(crate) const SPI_CTRL_FREQ_MASK: u32 = 0x0F00_0F00;

pub(crate) struct ValidatedDmaRead {
    pub flash_start: usize,
    pub flash_end: usize,
    pub dram_addr: u32,
    pub dma_len_reg: u32,
}

pub(crate) fn flash_capacity_bytes(config: Option<FlashConfig>) -> Result<usize, SmcError> {
    match config {
        Some(config) => (config.capacity_mb as usize)
            .checked_mul(1024 * 1024)
            .ok_or(SmcError::InvalidCapacity),
        None => Ok(0),
    }
}

pub(crate) fn cs_capacity_bytes(
    config: &SmcConfig,
    cs: ChipSelect,
) -> Result<usize, SmcError> {
    let slot = match cs {
        ChipSelect::Cs0 => config.cs0,
        ChipSelect::Cs1 => config.cs1,
    };
    match slot {
        Some(_) => flash_capacity_bytes(slot),
        None => Err(SmcError::InvalidChipSelect),
    }
}

pub(crate) fn total_capacity_bytes(
    cs0: Option<FlashConfig>,
    cs1: Option<FlashConfig>,
) -> Result<usize, SmcError> {
    let cs0_size = flash_capacity_bytes(cs0)?;
    let cs1_size = flash_capacity_bytes(cs1)?;
    let total = cs0_size.checked_add(cs1_size).ok_or(SmcError::InvalidCapacity)?;
    if total > SMC_WINDOW_SIZE_BYTES {
        return Err(SmcError::InvalidCapacity);
    }
    Ok(total)
}

pub(crate) fn validate_mapped_range(
    offset: u32,
    len: usize,
    capacity_bytes: usize,
) -> Result<usize, SmcError> {
    let offset = offset as usize;
    let end = offset.checked_add(len).ok_or(SmcError::InvalidCapacity)?;
    if end > capacity_bytes {
        return Err(SmcError::InvalidCapacity);
    }
    Ok(offset)
}

pub(crate) fn validate_dma_read(
    flash_offset: u32,
    dram_addr: usize,
    len: u32,
    capacity_bytes: usize,
) -> Result<ValidatedDmaRead, SmcError> {
    if len == 0 {
        return Err(SmcError::InvalidCapacity);
    }

    let flash_start = validate_mapped_range(flash_offset, len as usize, capacity_bytes)?;
    let flash_end = flash_start + len as usize;
    let dram_addr = u32::try_from(dram_addr).map_err(|_| SmcError::InvalidCapacity)?;
    if dram_addr & 0x3 != 0 || dram_addr != (dram_addr & DMA_DRAM_MASK) {
        return Err(SmcError::InvalidCapacity);
    }

    Ok(ValidatedDmaRead {
        flash_start,
        flash_end,
        dram_addr,
        dma_len_reg: len - 1,
    })
}

/// Encode a memory segment into hardware register format.
///
/// Hardware uses 4 KB units for addressing.
pub(crate) fn encode_segment(start: usize, end: usize) -> Result<u32, SmcError> {
    let start_4k = (start >> 12) as u32;
    let end_4k = (end >> 12) as u32;

    if start_4k > 0xFFFF || end_4k > 0xFFFF {
        return Err(SmcError::InvalidCapacity);
    }

    Ok((end_4k << 16) | start_4k)
}

/// Calculate AST-compatible SPI clock divider field for CS control registers.
///
/// Ported from aspeed-rust's silicon-tested divisor search logic.
pub(crate) fn spi_freq_div(sysclk_mhz: u32, max_freq_mhz: u32) -> Result<u32, SmcError> {
    if max_freq_mhz == 0 {
        return Err(SmcError::HardwareError);
    }

    let div_arr = [15u32, 7, 14, 6, 13, 5, 12, 4, 11, 3, 10, 2, 9, 1, 8, 0];

    for i in 0..0x0f {
        for (j, div_val) in div_arr.iter().copied().enumerate() {
            if i == 0 && j == 0 {
                continue;
            }

            let divisor = (j as u32) + 1 + (i * 16);
            let freq = sysclk_mhz / divisor;
            if max_freq_mhz >= freq {
                return Ok((i << 24) | (div_val << 8));
            }
        }
    }

    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_segment() {
        let seg = encode_segment(0, 16 * 1024 * 1024).unwrap();
        let start_4k = seg & 0xFFFF;
        let end_4k = (seg >> 16) & 0xFFFF;
        assert_eq!(start_4k, 0);
        assert_eq!(end_4k, 4096);
    }

    fn decode_divisor(encoded: u32) -> u32 {
        let table = [15u32, 7, 14, 6, 13, 5, 12, 4, 11, 3, 10, 2, 9, 1, 8, 0];
        let hi = (encoded >> 24) & 0x0f;
        let lo = (encoded >> 8) & 0x0f;
        let j = table
            .iter()
            .position(|&v| v == lo)
            .expect("encoded lo nibble must exist in divisor table") as u32;
        j + 1 + (hi * 16)
    }

    #[test]
    fn test_spi_freq_div_25mhz_bound() {
        let encoded = spi_freq_div(200, 25).unwrap();
        let divisor = decode_divisor(encoded);
        assert_eq!(200 / divisor, 25);
        assert!(encoded & !SPI_CTRL_FREQ_MASK == 0);
    }

    #[test]
    fn test_spi_freq_div_50mhz_bound() {
        let encoded = spi_freq_div(200, 50).unwrap();
        let divisor = decode_divisor(encoded);
        assert_eq!(200 / divisor, 50);
        assert!(encoded & !SPI_CTRL_FREQ_MASK == 0);
    }

    #[test]
    fn test_spi_freq_div_rejects_zero_target() {
        assert!(spi_freq_div(200, 0).is_err());
    }

    #[test]
    fn test_segment_overflow() {
        let result = encode_segment(0, 512 * 1024 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_total_capacity_overflow() {
        let result = total_capacity_bytes(
            Some(FlashConfig {
                capacity_mb: 128,
                page_size: 256,
                sector_size: 4096,
                block_size: 65536,
                spi_clock_mhz: 25,
            }),
            Some(FlashConfig {
                capacity_mb: 129,
                page_size: 256,
                sector_size: 4096,
                block_size: 65536,
                spi_clock_mhz: 25,
            }),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_mapped_range_accepts_exact_fit() {
        let offset = validate_mapped_range(4092, 4, 4096).unwrap();
        assert_eq!(offset, 4092);
    }

    #[test]
    fn test_validate_mapped_range_rejects_overflow() {
        let result = validate_mapped_range(u32::MAX, 8, SMC_WINDOW_SIZE_BYTES);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_mapped_range_rejects_out_of_bounds() {
        let result = validate_mapped_range(1024, 1, 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_dma_read_accepts_valid_request() {
        let validated = validate_dma_read(0x1000, 0x0008_0000, 512, 16 * 1024 * 1024).unwrap();
        assert_eq!(validated.flash_start, 0x1000);
        assert_eq!(validated.flash_end, 0x1200);
        assert_eq!(validated.dram_addr, 0x0008_0000);
        assert_eq!(validated.dma_len_reg, 511);
    }

    #[test]
    fn test_validate_dma_read_rejects_zero_length() {
        let result = validate_dma_read(0, 0x0008_0000, 0, 16 * 1024 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_dma_read_rejects_unaligned_dram() {
        let result = validate_dma_read(0, 0x0008_0002, 256, 16 * 1024 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_dma_read_rejects_masked_dram_bits() {
        let result = validate_dma_read(0, 0x1000_0000, 256, 16 * 1024 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_dma_read_rejects_flash_range_overflow() {
        let result = validate_dma_read(0x00ff_ff00, 0x0008_0000, 0x200, 16 * 1024 * 1024);
        assert!(result.is_err());
    }
}
