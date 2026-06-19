// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use ast1060_pac as device;
use core::marker::PhantomData;

use super::types::{
    Bank, BankDevice, Direction, Error, InitialLevel, InterruptMode, InterruptTrigger,
    SgpiomPinConfig,
};

pub struct Sgpiom {
    sgpiom: *const device::sgpiom::RegisterBlock,
    /// Prevent `Send` and `Sync`.
    ///
    /// MMIO register blocks must not be transferred across threads or
    /// shared by reference due to potential side effects and lack of
    /// synchronization guarantees.
    _not_send_sync: PhantomData<*const ()>,
}

impl Sgpiom {
    /// Create an SGPIOM instance from a raw register-block pointer.
    ///
    /// # Safety
    ///
    /// - `sgpiom` must be a valid, non-null pointer to the AST1060 SGPIOM register block.
    /// - The pointed register block must remain valid for the lifetime of this `Sgpiom`.
    /// - Caller must enforce global ownership so concurrent mutable access does not occur.
    pub const unsafe fn new(sgpiom: *const device::sgpiom::RegisterBlock) -> Self {
        Self { sgpiom, _not_send_sync: PhantomData }
    }

    /// Create an instance pointing to the global AST1060 SGPIOM register block.
    ///
    /// # Safety
    ///
    /// Caller must ensure access to the singleton SGPIOM is coordinated.
    pub unsafe fn new_global() -> Self {
        // SAFETY: Caller upholds the singleton access contract.
        unsafe { Self::new(device::Sgpiom::ptr()) }
    }

    #[inline]
    fn regs(&self) -> &device::sgpiom::RegisterBlock {
        // SAFETY: `Sgpiom` construction is `unsafe`, so caller upholds pointer validity,
        // non-nullness, and aliasing/ownership requirements.
        unsafe { &*self.sgpiom }
    }

    /// Configures SGPIOM global settings.
    ///
    /// `ngpios` is total SGPIO count across banks.
    pub fn configure_global(&self, ngpios: u16, clock_div: u16) -> Result<(), Error> {
        if ngpios == 0 {
            return Err(Error::InvalidNgpios);
        }

        let numbers = ((ngpios as u32 + 7) / 8) as u8;

        self.regs().gpio554().modify(|_, w| {
            w.enbl_of_serial_gpio().set_bit();
            // SAFETY: writing the datasheet-defined numbers and clock-division fields.
            unsafe { w.numbers_of_serial_gpiopins().bits(numbers) };
            unsafe { w.serial_gpioclk_division().bits(clock_div) };
            w
        });
        Ok(())
    }

    /// Read the raw 32-bit Data Value register for a bank (sampled serial input).
    #[must_use]
    pub fn port_get_raw(&self, bank: Bank) -> u32 {
        match bank {
            Bank::Ad => self.regs().gpio500().read().bits(),
            Bank::Eh => self.regs().gpio51c().read().bits(),
            Bank::Il => self.regs().gpio538().read().bits(),
            Bank::Mp => self.regs().gpio590().read().bits(),
        }
    }

    /// Read the output-latch readback register for a bank.
    ///
    /// Use this for read-modify-write of outputs. `port_get_raw` returns the
    /// sampled serial input state, not the last driven value.
    #[must_use]
    pub fn read_output_latch(&self, bank: Bank) -> u32 {
        match bank {
            Bank::Ad => self.regs().gpio570().read().bits(),
            Bank::Eh => self.regs().gpio574().read().bits(),
            Bank::Il => self.regs().gpio578().read().bits(),
            // SAFETY: `self.sgpiom` is a valid RegisterBlock pointer. +0x7c is the
            // MP write-latch readback register inside the 0x100-byte SGPIOM register file.
            Bank::Mp => unsafe {
                let base = self.sgpiom.cast::<u8>();
                core::ptr::read_volatile(base.add(0x7c).cast::<u32>())
            },
        }
    }

    pub fn port_set_masked_raw(&self, bank: Bank, mask: u32, value: u32) {
        let current = self.read_output_latch(bank);
        let next = (current & !mask) | (value & mask);
        self.port_write_raw(bank, next);
    }

    pub fn port_set_bits_raw(&self, bank: Bank, mask: u32) {
        self.port_set_masked_raw(bank, mask, mask);
    }

    pub fn port_clear_bits_raw(&self, bank: Bank, mask: u32) {
        self.port_set_masked_raw(bank, mask, 0);
    }

    pub fn port_toggle_bits(&self, bank: Bank, mask: u32) {
        let current = self.read_output_latch(bank);
        self.port_write_raw(bank, current ^ mask);
    }

    pub fn pin_set_raw(&self, dev: &BankDevice, pin: u8, high: bool) -> Result<(), Error> {
        dev.validate_pin(pin)?;
        let bit = 1u32 << pin;
        if high {
            self.port_set_bits_raw(dev.bank, bit);
        } else {
            self.port_clear_bits_raw(dev.bank, bit);
        }
        Ok(())
    }

    pub fn configure_pin(
        &self,
        dev: &BankDevice,
        pin: u8,
        cfg: SgpiomPinConfig,
    ) -> Result<(), Error> {
        dev.validate_pin(pin)?;

        if cfg.pull_up || cfg.pull_down {
            return Err(Error::UnsupportedFlags);
        }

        if cfg.direction == Direction::Output {
            if let Some(initial) = cfg.initial {
                self.pin_set_raw(dev, pin, initial == InitialLevel::High)?;
            }
        }

        // SGPIOM direction is hardware managed in this design; no extra register write needed.
        Ok(())
    }

    pub fn configure_interrupt(
        &self,
        dev: &BankDevice,
        pin: u8,
        mode: InterruptMode,
        trig: InterruptTrigger,
    ) -> Result<(), Error> {
        dev.validate_pin(pin)?;

        let bit = 1u32 << pin;
        let int_type = match mode {
            InterruptMode::Disabled => 0u8,
            InterruptMode::Level => match trig {
                InterruptTrigger::Low => 2,
                InterruptTrigger::High => 3,
                InterruptTrigger::Both => return Err(Error::UnsupportedFlags),
            },
            InterruptMode::Edge => match trig {
                InterruptTrigger::Low => 0,
                InterruptTrigger::High => 1,
                InterruptTrigger::Both => 4,
            },
        };

        match mode {
            InterruptMode::Disabled => {
                let en = self.int_en_read(dev.bank) & !bit;
                self.int_en_write(dev.bank, en);
            }
            _ => {
                let en = self.int_en_read(dev.bank) | bit;
                self.int_en_write(dev.bank, en);

                let mut s0 = self.int_sens_read(dev.bank, 0) & !bit;
                let mut s1 = self.int_sens_read(dev.bank, 1) & !bit;
                let mut s2 = self.int_sens_read(dev.bank, 2) & !bit;

                if (int_type & 0x1) != 0 {
                    s0 |= bit;
                }
                if (int_type & 0x2) != 0 {
                    s1 |= bit;
                }
                if (int_type & 0x4) != 0 {
                    s2 |= bit;
                }

                self.int_sens_write(dev.bank, 0, s0);
                self.int_sens_write(dev.bank, 1, s1);
                self.int_sens_write(dev.bank, 2, s2);
            }
        }

        Ok(())
    }

    /// Read the latched interrupt status register for a bank.
    #[must_use]
    pub fn interrupt_status(&self, bank: Bank) -> u32 {
        match bank {
            Bank::Ad => self.regs().gpio514().read().bits(),
            Bank::Eh => self.regs().gpio530().read().bits(),
            Bank::Il => self.regs().gpio54c().read().bits(),
            Bank::Mp => self.regs().gpio5a4().read().bits(),
        }
    }

    /// Acknowledge (clear) interrupt status bits for a bank.
    pub fn clear_interrupt_status(&self, bank: Bank, mask: u32) {
        match bank {
            Bank::Ad => self.regs().gpio514().write(|w| unsafe { w.bits(mask) }),
            Bank::Eh => self.regs().gpio530().write(|w| unsafe { w.bits(mask) }),
            Bank::Il => self.regs().gpio54c().write(|w| unsafe { w.bits(mask) }),
            Bank::Mp => self.regs().gpio5a4().write(|w| unsafe { w.bits(mask) }),
        };
    }

    pub fn passthrough_masked(&self, bank: Bank, mask: u32) {
        let sampled = self.port_get_raw(bank);
        self.port_set_masked_raw(bank, mask, sampled);
    }

    fn port_write_raw(&self, bank: Bank, value: u32) {
        match bank {
            Bank::Ad => self.regs().gpio500().write(|w| unsafe { w.bits(value) }),
            Bank::Eh => self.regs().gpio51c().write(|w| unsafe { w.bits(value) }),
            Bank::Il => self.regs().gpio538().write(|w| unsafe { w.bits(value) }),
            Bank::Mp => self.regs().gpio590().write(|w| unsafe { w.bits(value) }),
        };
    }

    fn int_en_read(&self, bank: Bank) -> u32 {
        match bank {
            Bank::Ad => self.regs().gpio504().read().bits(),
            Bank::Eh => self.regs().gpio520().read().bits(),
            Bank::Il => self.regs().gpio53c().read().bits(),
            Bank::Mp => self.regs().gpio594().read().bits(),
        }
    }

    fn int_en_write(&self, bank: Bank, value: u32) {
        match bank {
            Bank::Ad => self.regs().gpio504().write(|w| unsafe { w.bits(value) }),
            Bank::Eh => self.regs().gpio520().write(|w| unsafe { w.bits(value) }),
            Bank::Il => self.regs().gpio53c().write(|w| unsafe { w.bits(value) }),
            Bank::Mp => self.regs().gpio594().write(|w| unsafe { w.bits(value) }),
        };
    }

    fn int_sens_read(&self, bank: Bank, index: u8) -> u32 {
        match (bank, index) {
            (Bank::Ad, 0) => self.regs().gpio508().read().bits(),
            (Bank::Ad, 1) => self.regs().gpio50c().read().bits(),
            (Bank::Ad, 2) => self.regs().gpio510().read().bits(),
            (Bank::Eh, 0) => self.regs().gpio524().read().bits(),
            (Bank::Eh, 1) => self.regs().gpio528().read().bits(),
            (Bank::Eh, 2) => self.regs().gpio52c().read().bits(),
            (Bank::Il, 0) => self.regs().gpio540().read().bits(),
            (Bank::Il, 1) => self.regs().gpio544().read().bits(),
            (Bank::Il, 2) => self.regs().gpio548().read().bits(),
            (Bank::Mp, 0) => self.regs().gpio598().read().bits(),
            (Bank::Mp, 1) => self.regs().gpio59c().read().bits(),
            (Bank::Mp, 2) => self.regs().gpio5a0().read().bits(),
            _ => 0,
        }
    }

    fn int_sens_write(&self, bank: Bank, index: u8, value: u32) {
        match (bank, index) {
            (Bank::Ad, 0) => {
                self.regs().gpio508().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Ad, 1) => {
                self.regs().gpio50c().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Ad, 2) => {
                self.regs().gpio510().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Eh, 0) => {
                self.regs().gpio524().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Eh, 1) => {
                self.regs().gpio528().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Eh, 2) => {
                self.regs().gpio52c().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Il, 0) => {
                self.regs().gpio540().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Il, 1) => {
                self.regs().gpio544().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Il, 2) => {
                self.regs().gpio548().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Mp, 0) => {
                self.regs().gpio598().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Mp, 1) => {
                self.regs().gpio59c().write(|w| unsafe { w.bits(value) });
            }
            (Bank::Mp, 2) => {
                self.regs().gpio5a0().write(|w| unsafe { w.bits(value) });
            }
            _ => {}
        };
    }
}
