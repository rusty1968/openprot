// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidPin,
    InvalidNgpios,
    UnsupportedFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialLevel {
    Low,
    High,
}

/// Low-level per-pin configuration used by [`crate::sgpiom::register_block::Sgpiom::configure_pin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SgpiomPinConfig {
    pub direction: Direction,
    pub initial: Option<InitialLevel>,
    pub pull_up: bool,
    pub pull_down: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptMode {
    Disabled,
    Level,
    Edge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptTrigger {
    Low,
    High,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Bank {
    Ad = 0,
    Eh = 1,
    Il = 2,
    Mp = 3,
}

impl Bank {
    #[inline]
    pub const fn from_pin_offset(pin_offset: u8) -> Option<Self> {
        match pin_offset >> 5 {
            0 => Some(Self::Ad),
            1 => Some(Self::Eh),
            2 => Some(Self::Il),
            3 => Some(Self::Mp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankDevice {
    pub bank: Bank,
    pub pin_offset: u8,
    pub ngpios: u8,
}

impl BankDevice {
    #[inline]
    pub const fn new(bank: Bank, pin_offset: u8, ngpios: u8) -> Self {
        Self {
            bank,
            pin_offset,
            ngpios,
        }
    }

    #[inline]
    pub fn validate_pin(&self, pin: u8) -> Result<(), Error> {
        if pin < self.ngpios && pin < 32 {
            Ok(())
        } else {
            Err(Error::InvalidPin)
        }
    }
}
