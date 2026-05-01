// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Safe interrupt handling
//!
//! Provides safe enums for decoding interrupts, suitable for use in ISR context.

use super::types::SmcController;

/// SMC interrupt types
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcInterrupt {
    /// DMA transfer completed successfully
    DmaComplete = 0,
    /// DMA transfer was aborted
    DmaError = 1,
    /// Command was aborted
    CommandAbort = 2,
    /// Write protect signal active
    WriteProtected = 3,
    /// Unknown or no interrupt
    Unknown = 255,
}

impl TryFrom<u8> for SmcInterrupt {
    type Error = ();

    fn try_from(value: u8) -> core::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(SmcInterrupt::DmaComplete),
            1 => Ok(SmcInterrupt::DmaError),
            2 => Ok(SmcInterrupt::CommandAbort),
            3 => Ok(SmcInterrupt::WriteProtected),
            _ => Err(()),
        }
    }
}

/// Decoder for SMC interrupt status register
///
/// Safe to call from ISR context. Returns the decoded interrupt type.
pub struct SmcInterruptDecoder;

impl SmcInterruptDecoder {
    /// Decode interrupt control register bits
    pub fn decode(intr_ctrl: u32) -> SmcInterrupt {
        // Bit fields from hardware register:
        // [11] = DMA_STATUS (DMA complete)
        // [10] = CMD_ABORT_STATUS (command abort)
        // [9] = WRITE_PROTECT_STATUS
        if (intr_ctrl & (1 << 11)) != 0 {
            SmcInterrupt::DmaComplete
        } else if (intr_ctrl & (1 << 10)) != 0 {
            SmcInterrupt::CommandAbort
        } else if (intr_ctrl & (1 << 9)) != 0 {
            SmcInterrupt::WriteProtected
        } else {
            SmcInterrupt::Unknown
        }
    }
}

/// Context passed to interrupt handler
///
/// Safe to use in ISR; only allows non-blocking operations
pub struct SmcIsrContext {
    pub controller_id: SmcController,
    pub interrupt: SmcInterrupt,
}
