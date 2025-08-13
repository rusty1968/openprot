//! Base System Controller (SCU) implementation for AST1060
//!
//! This module provides the core system control functionality, copied and adapted
//! from aspeed-rust implementation. The OpenPRoT HAL trait implementations are
//! in a separate module to keep concerns separated.

use ast1060_pac::Scu;
use core::time::Duration;

pub const HPLL_FREQ: u32 = 800_000_000;

pub const fn mhz(mhz: u32) -> u32 {
    mhz * 1_000_000
}

const ASPEED_CLK_GRP_1_OFFSET: u8 = 32;
const ASPEED_CLK_GRP_2_OFFSET: u8 = 64;
const ASPEED_RESET_GRP_1_OFFSET: u8 = 32;

const ASPEED_I3C_CLOCK_DIVIDER_MAX: u8 = 15;
const ASPEED_HCLK_CLOCK_DIVIDER_MAX: u8 = 7;

const I3C_CLK_SRC_480MHZ: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClockId {
    ClkSRAM = 0,
    ClkSOC = 1,
    ClkLPC = 5,
    ClkUART1 = 8,
    ClkUART2 = 9,
    ClkUART3 = 10,
    ClkUART4 = 11,
    ClkUART5 = 12,
    ClkHACE = 13,
    ClkMII = 19,
    ClkRSA = 24,
    ClkI2C = 25,
    ClkSPI1 = 30,
    ClkSPI2 = 31,
    ClkI3C0 = 32,
    ClkI3C1 = 33,
    ClkI3C2 = 34,
    ClkI3C3 = 35,
    ClkUSB = 36,
    ClkHCLK = 64,
    ClkPCLK = 65,
    ClkAPB = 66,
    ClkAHB = 67,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ResetId {
    RstSRAM = 0,
    RstSOC = 1,
    RstLPC = 5,
    RstUART1 = 8,
    RstUART2 = 9,
    RstUART3 = 10,
    RstUART4 = 11,
    RstUART5 = 12,
    RstHACE = 13,
    RstMII = 19,
    RstRSA = 24,
    RstI2C = 25,
    RstSPI1 = 30,
    RstSPI2 = 31,
    RstI3C0 = 32,
    RstI3C1 = 33,
    RstI3C2 = 34,
    RstI3C3 = 35,
    RstUSB = 36,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I3CClkSource {
    I3CHPLL = 0,
    I3C480MHZ = 1,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HCLKSource {
    HCLK = 0,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    ClockAlreadyEnabled,
    ClockAlreadyDisabled,
    InvalidClockId,
    InvalidClockFrequency,
    ClockConfigurationFailed,
    InvalidResetId,
    HardwareFailure,
    PermissionDenied,
    InvalidClkSource,
    Timeout,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ClockConfig {
    pub frequency_hz: u64,
    pub clk_source_sel: u8,
}

pub struct SysCon {
    scu: Scu,
}

impl SysCon {
    pub fn new(scu: Scu) -> Self {
        Self { scu }
    }
    
    /// Clock Stop Control Clear
    /// `clock_bit`: clock enable bit position
    ///
    pub fn enable_clock(&mut self, clock_bit: u8) -> Result<(), Error> {
        let mut bit_pos = clock_bit;
        if bit_pos >= ASPEED_CLK_GRP_2_OFFSET {
            return Ok(());
        }
        if bit_pos >= ASPEED_CLK_GRP_1_OFFSET {
            bit_pos -= ASPEED_CLK_GRP_1_OFFSET;
            if self.scu.scu090().read().bits() & (1 << bit_pos) == (1 << bit_pos) {
                self.scu.scu094().write(|w| unsafe { w.bits(1 << bit_pos) });
            } else {
                return Err(Error::ClockAlreadyEnabled);
            }
        } else if self.scu.scu080().read().bits() & (1 << bit_pos) == (1 << bit_pos) {
            self.scu.scu084().write(|w| unsafe { w.bits(1 << bit_pos) });
        } else {
            return Err(Error::ClockAlreadyEnabled);
        }
        Ok(())
    }

    pub fn disable_clock(&mut self, clock_bit: u8) -> Result<(), Error> {
        let mut bit_pos = clock_bit;
        if bit_pos >= ASPEED_CLK_GRP_2_OFFSET {
            return Ok(());
        }
        if bit_pos >= ASPEED_CLK_GRP_1_OFFSET {
            bit_pos -= ASPEED_CLK_GRP_1_OFFSET;
            if self.scu.scu090().read().bits() & (1 << bit_pos) == (1 << bit_pos) {
                return Err(Error::ClockAlreadyDisabled);
            }
            self.scu.scu090().write(|w| unsafe { w.bits(1 << bit_pos) });
        } else if self.scu.scu080().read().bits() & (1 << bit_pos) == (1 << bit_pos) {
            return Err(Error::ClockAlreadyDisabled);
        } else {
            self.scu.scu080().write(|w| unsafe { w.bits(1 << bit_pos) });
        }
        Ok(())
    }

    pub fn set_frequency(&mut self, clock_id: ClockId, frequency_hz: u64) -> Result<(), Error> {
        let src: u32;
        let clk_div: u32;
        let freq = u32::try_from(frequency_hz).map_err(|_| Error::InvalidClockFrequency)?;
        match clock_id {
            ClockId::ClkI3C0 | ClockId::ClkI3C1 | ClockId::ClkI3C2 | ClockId::ClkI3C3 => {
                if self.scu.scu310().read().i3cclk_source_sel().bit() == I3C_CLK_SRC_480MHZ {
                    src = mhz(480);
                } else {
                    src = HPLL_FREQ;
                }
                clk_div = src / freq;
                if clk_div <= u32::from(ASPEED_I3C_CLOCK_DIVIDER_MAX) {
                    let divider =
                        u8::try_from(clk_div).map_err(|_| Error::InvalidClockFrequency)?;
                    self.scu
                        .scu310()
                        .modify(|_, w| unsafe { w.i3cclk_divider_sel().bits(divider) });
                    Ok(())
                } else {
                    Err(Error::InvalidClockFrequency)
                }
            }
            ClockId::ClkHCLK => {
                src = HPLL_FREQ;
                clk_div = src / freq;
                if clk_div <= u32::from(ASPEED_HCLK_CLOCK_DIVIDER_MAX) {
                    let divider =
                        u8::try_from(clk_div).map_err(|_| Error::InvalidClockFrequency)?;
                    self.scu
                        .scu314()
                        .modify(|_, w| unsafe { w.hclkdivider_sel().bits(divider) });
                    Ok(())
                } else {
                    Err(Error::InvalidClockFrequency)
                }
            }
            _ => Err(Error::PermissionDenied),
        }
    }

    pub fn get_frequency(&self, clock_id: ClockId) -> Result<u64, Error> {
        let freq: u32 = match clock_id {
            ClockId::ClkI3C0 | ClockId::ClkI3C1 | ClockId::ClkI3C2 | ClockId::ClkI3C3 => {
                let src: u32 = if self.scu.scu310().read().i3cclk_source_sel().bit() == I3C_CLK_SRC_480MHZ {
                    mhz(480)
                } else {
                    HPLL_FREQ
                };
                let divider = self.scu.scu310().read().i3cclk_divider_sel().bits();
                src / u32::from(divider)
            }
            ClockId::ClkHCLK => {
                let divider = self.scu.scu314().read().hclkdivider_sel().bits();
                HPLL_FREQ / u32::from(divider)
            }
            ClockId::ClkPCLK => {
                let hclk_divider = self.scu.scu314().read().hclkdivider_sel().bits();
                HPLL_FREQ / u32::from(hclk_divider) / 2
            }
            _ => return Err(Error::PermissionDenied),
        };
        Ok(u64::from(freq))
    }

    pub fn configure_clock(&mut self, clock_id: ClockId, config: &ClockConfig) -> Result<(), Error> {
        match clock_id {
            ClockId::ClkI3C0 | ClockId::ClkI3C1 | ClockId::ClkI3C2 | ClockId::ClkI3C3 => {
                if config.clk_source_sel > I3CClkSource::I3C480MHZ as u8 {
                    return Err(Error::InvalidClkSource);
                }
                self.scu.scu310().modify(|_, w| {
                    w.i3cclk_source_sel()
                        .bit(config.clk_source_sel != I3CClkSource::I3CHPLL as u8)
                });

                self.set_frequency(clock_id, config.frequency_hz)
            }

            ClockId::ClkHCLK => {
                if config.clk_source_sel > HCLKSource::HCLK as u8 {
                    return Err(Error::InvalidClkSource);
                }
                self.scu
                    .scu314()
                    .modify(|_, w| unsafe { w.hclkdivider_sel().bits(config.clk_source_sel) });

                self.set_frequency(clock_id, config.frequency_hz)
            }

            _ => Err(Error::PermissionDenied),
        }
    }

    pub fn get_clock_config(&self, clock_id: ClockId) -> Result<ClockConfig, Error> {
        let clk_source_sel: u8 = match clock_id {
            ClockId::ClkI3C0 | ClockId::ClkI3C1 | ClockId::ClkI3C2 | ClockId::ClkI3C3 => {
                if self.scu.scu310().read().i3cclk_source_sel().bit() {
                    I3CClkSource::I3C480MHZ as u8
                } else {
                    I3CClkSource::I3CHPLL as u8
                }
            }

            ClockId::ClkHCLK | ClockId::ClkPCLK => 0,

            _ => {
                return Err(Error::PermissionDenied);
            }
        };
        let frequency_hz = self.get_frequency(clock_id)?;
        let config = ClockConfig {
            frequency_hz,
            clk_source_sel,
        };
        Ok(config)
    }

    pub fn reset_assert(&mut self, reset_id: u8) -> Result<(), Error> {
        let mut bit_pos = reset_id;

        if bit_pos >= ASPEED_RESET_GRP_1_OFFSET + 32 {
            return Err(Error::InvalidResetId);
        }

        let reg_value: u32 = if bit_pos >= ASPEED_RESET_GRP_1_OFFSET {
            bit_pos -= ASPEED_RESET_GRP_1_OFFSET;
            self.scu.scu050().write(|w| unsafe { w.bits(1 << bit_pos) });
            self.scu.scu050().read().bits()
        } else {
            self.scu.scu040().write(|w| unsafe { w.bits(1 << bit_pos) });
            self.scu.scu040().read().bits()
        };

        if reg_value & (1 << bit_pos) != (1 << bit_pos) {
            return Err(Error::HardwareFailure);
        }
        Ok(())
    }

    pub fn reset_deassert(&mut self, reset_id: u8) -> Result<(), Error> {
        let mut bit_pos = reset_id;
        if bit_pos >= ASPEED_RESET_GRP_1_OFFSET + 32 {
            return Err(Error::InvalidResetId);
        }

        let reg_value: u32 = if bit_pos >= ASPEED_RESET_GRP_1_OFFSET {
            bit_pos -= ASPEED_RESET_GRP_1_OFFSET;
            self.scu.scu054().write(|w| unsafe { w.bits(1 << bit_pos) });
            self.scu.scu054().read().bits()
        } else {
            self.scu.scu044().write(|w| unsafe { w.bits(1 << bit_pos) });
            self.scu.scu044().read().bits()
        };

        if reg_value & (1 << bit_pos) != (1 << bit_pos) {
            return Err(Error::HardwareFailure);
        }
        Ok(())
    }

    pub fn reset_pulse(&mut self, reset_id: u8, _duration: Duration) -> Result<(), Error> {
        let bit_pos: u8 = reset_id;
        if bit_pos >= ASPEED_RESET_GRP_1_OFFSET + 32 {
            return Err(Error::InvalidResetId);
        }
        
        // For now, just do assert then immediate deassert
        // In a real implementation, you'd want to implement a proper delay
        // mechanism that doesn't require dynamic polymorphism
        self.reset_assert(reset_id)?;
        
        // Add a simple busy wait (not ideal but removes the dependency)
        for _ in 0..1000 {
            core::hint::spin_loop();
        }
        
        self.reset_deassert(reset_id)
    }

    pub fn reset_is_asserted(&self, reset_id: u8) -> Result<bool, Error> {
        let mut bit_pos: u8 = reset_id;

        if bit_pos >= ASPEED_RESET_GRP_1_OFFSET + 32 {
            return Err(Error::InvalidResetId);
        }

        let reg_value: u32 = if bit_pos >= ASPEED_RESET_GRP_1_OFFSET + 32 {
            return Err(Error::InvalidResetId);
        } else if bit_pos >= ASPEED_RESET_GRP_1_OFFSET {
            bit_pos -= ASPEED_RESET_GRP_1_OFFSET;
            self.scu.scu050().read().bits()
        } else {
            self.scu.scu040().read().bits()
        };

        if reg_value & (1 << bit_pos) == (1 << bit_pos) {
            return Ok(true);
        }
        Ok(false)
    }

    /// Convenience method to initialize HACE peripheral
    /// Enables clock and deasserts reset
    pub fn init_hace(&mut self) -> Result<(), Error> {
        // Enable HACE clock
        self.enable_clock(ClockId::ClkHACE as u8)?;
        
        // Deassert HACE reset
        self.reset_deassert(ResetId::RstHACE as u8)?;
        
        Ok(())
    }

    /// Convenience method to initialize RSA peripheral
    /// Enables clock and deasserts reset
    pub fn init_rsa(&mut self) -> Result<(), Error> {
        // Enable RSA clock
        self.enable_clock(ClockId::ClkRSA as u8)?;
        
        // Deassert RSA reset
        self.reset_deassert(ResetId::RstRSA as u8)?;
        
        Ok(())
    }
}
