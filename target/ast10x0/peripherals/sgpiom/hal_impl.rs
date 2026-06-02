// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use openprot_hal_blocking::gpio_port::{
    ActivePolarity, GpioBankPassthrough, GpioError, GpioErrorKind, GpioErrorType, GpioPort,
    PinConfig, PinDirection, PinMask,
};

use super::register_block::Sgpiom;
use super::types::{BankDevice, Error};

/// 32-bit pin mask for a single SGPIOM bank.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct SgpiomMask(pub u32);

impl PinMask for SgpiomMask {
    fn empty() -> Self {
        Self(0)
    }

    fn all() -> Self {
        Self(0xFFFF_FFFF)
    }

    fn is_empty(&self) -> bool {
        self.0 == 0
    }

    fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn intersection(&self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    fn toggle(&self) -> Self {
        Self(!self.0)
    }
}

impl GpioError for Error {
    fn kind(&self) -> GpioErrorKind {
        match self {
            Error::InvalidPin => GpioErrorKind::InvalidPin,
            Error::InvalidNgpios => GpioErrorKind::UnsupportedConfiguration,
            Error::UnsupportedFlags => GpioErrorKind::UnsupportedConfiguration,
        }
    }
}

/// A single SGPIOM bank exposed as a HAL [`GpioPort`].
///
/// Combines the shared [`Sgpiom`] register block with a [`BankDevice`] descriptor.
/// Use [`SgpiomBankPort::new`] to construct; the same safety contract as [`Sgpiom::new`]
/// applies.
pub struct SgpiomBankPort {
    pub(super) sgpiom: Sgpiom,
    pub(super) dev: BankDevice,
}

impl SgpiomBankPort {
    /// Create a bank port from an existing `Sgpiom` instance and a `BankDevice` descriptor.
    ///
    /// # Safety
    ///
    /// Same contract as [`Sgpiom::new`]: the register block pointer must be valid, non-null,
    /// and access must be externally coordinated for the lifetime of this value.
    pub const unsafe fn new(sgpiom: Sgpiom, dev: BankDevice) -> Self {
        Self { sgpiom, dev }
    }
}

impl GpioErrorType for SgpiomBankPort {
    type Error = Error;
}

impl GpioPort for SgpiomBankPort {
    type Config = PinConfig;
    type Mask = SgpiomMask;

    fn configure(&mut self, pins: Self::Mask, config: Self::Config) -> Result<(), Self::Error> {
        // Reject pins outside this bank's ngpios window.
        let valid_mask: u32 = if self.dev.ngpios >= 32 {
            u32::MAX
        } else {
            (1u32 << self.dev.ngpios) - 1
        };
        if (pins.0 & !valid_mask) != 0 {
            return Err(Error::InvalidPin);
        }

        if config.direction == PinDirection::Output {
            if let Some(logical_high) = config.initial_output {
                // Map logical level through active polarity to physical drive level.
                let drive_high = match config.polarity {
                    ActivePolarity::ActiveHigh => logical_high,
                    ActivePolarity::ActiveLow => !logical_high,
                };
                if drive_high {
                    self.sgpiom.port_set_bits_raw(self.dev.bank, pins.0);
                } else {
                    self.sgpiom.port_clear_bits_raw(self.dev.bank, pins.0);
                }
            }
        }

        // SGPIOM direction is hardware-managed; no direction register write is required.
        Ok(())
    }

    fn set_reset(
        &mut self,
        set_mask: Self::Mask,
        reset_mask: Self::Mask,
    ) -> Result<(), Self::Error> {
        // Atomically apply both masks: set wins over reset for overlapping bits.
        self.sgpiom
            .port_set_masked_raw(self.dev.bank, set_mask.0 | reset_mask.0, set_mask.0);
        Ok(())
    }

    fn read_input(&self) -> Result<Self::Mask, Self::Error> {
        Ok(SgpiomMask(self.sgpiom.port_get_raw(self.dev.bank)))
    }

    fn toggle(&mut self, pins: Self::Mask) -> Result<(), Self::Error> {
        self.sgpiom.port_toggle_bits(self.dev.bank, pins.0);
        Ok(())
    }
}

impl GpioBankPassthrough for SgpiomBankPort {
    /// Sample current inputs for `mask` pins and write them to the output latch (one-shot).
    fn set_passthrough_mask(&mut self, mask: Self::Mask) -> Result<(), Self::Error> {
        self.sgpiom.passthrough_masked(self.dev.bank, mask.0);
        Ok(())
    }

    /// No persistent passthrough hardware state exists; this is a no-op.
    fn clear_passthrough(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
