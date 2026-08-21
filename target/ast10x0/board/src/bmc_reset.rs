// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Board-portable BMC reset control.
//!
//! The two active-low BMC reset lines (`BMC_SRST`, `BMC_EXTRST`) are the seam
//! between the RoT holding the host in reset and the external-flash bus being
//! safe to drive. Boards route these lines through different peripherals:
//!
//! * AST1060 "prot": SGPIO master outputs (bank A–D bits 8/9).
//! * AST1060/AST2700 "dcscm": native GPIO `GPIO_M5` / `GPIO_H2`.
//!
//! Both are expressed through `embedded-hal` [`StatefulOutputPin`], so the
//! sequencer ([`BmcReset`]) and the access gate ([`BmcResetGate`]) are generic
//! over the pin type and each board just supplies concrete pins.

use ast1060_pac as device;
use ast10x0_peripherals::gpio::{gpioh, gpiom, GpioExt, Output, PushPull};
use core::convert::Infallible;
use embedded_hal::digital::{ErrorType, OutputPin, StatefulOutputPin};
use hal_flash::BusAccessGate;
use util_error::{self as error, ErrorCode};

/// SGPIO master A–D output-latch bit for the "prot" `BMC_SRST` line.
const PROT_SRST_MASK: u32 = 1 << 8;
/// SGPIO master A–D output-latch bit for the "prot" `BMC_EXTRST` line.
const PROT_EXTRST_MASK: u32 = 1 << 9;

/// Settle time between reset-line transitions.
const RESET_SETTLE_US: u32 = 10_000;

/// A single SGPIO master output (bank A–D) exposed as a [`StatefulOutputPin`].
///
/// `set_*` drive the output latch via `gpio500`; `is_set_*` read it back through
/// `gpio570`, so the readback reflects what the RoT holds, not an input level.
/// Construction has no register side effects, so it is safe for a read-only
/// observer to build one without disturbing the line.
pub struct SgpioOutputPin {
    mask: u32,
}

impl SgpioOutputPin {
    /// Bind to the SGPIO master A–D output-latch bit(s) in `mask`.
    ///
    /// # Safety
    ///
    /// Access to the SGPIO register block must be coordinated for the lifetime
    /// of this value.
    pub const unsafe fn new(mask: u32) -> Self {
        Self { mask }
    }
}

impl ErrorType for SgpioOutputPin {
    type Error = Infallible;
}

impl OutputPin for SgpioOutputPin {
    fn set_high(&mut self) -> Result<(), Self::Error> {
        let sgpio = unsafe { &*device::Sgpiom::ptr() };
        let latch = sgpio.gpio570().read().bits();
        sgpio
            .gpio500()
            .write(|w| unsafe { w.bits(latch | self.mask) });
        Ok(())
    }

    fn set_low(&mut self) -> Result<(), Self::Error> {
        let sgpio = unsafe { &*device::Sgpiom::ptr() };
        let latch = sgpio.gpio570().read().bits();
        sgpio
            .gpio500()
            .write(|w| unsafe { w.bits(latch & !self.mask) });
        Ok(())
    }
}

impl StatefulOutputPin for SgpioOutputPin {
    fn is_set_high(&mut self) -> Result<bool, Self::Error> {
        let sgpio = unsafe { &*device::Sgpiom::ptr() };
        Ok(sgpio.gpio570().read().bits() & self.mask == self.mask)
    }

    fn is_set_low(&mut self) -> Result<bool, Self::Error> {
        self.is_set_high().map(|v| !v)
    }
}

/// Enable the SGPIO master pins and serial-shift controller.
///
/// Required before driving the "prot" reset lines; the readback path used by
/// the gate does not depend on it.
fn configure_sgpio_master() {
    let scu = unsafe { &*device::Scu::ptr() };
    scu.scu41c().modify(|_, w| {
        w.enbl_sgpiomaster_ckfn_pin()
            .set_bit()
            .enbl_sgpiomaster_ldfn_pin()
            .set_bit()
            .enbl_sgpiomaster_dofn_pin()
            .set_bit()
            .enbl_sgpiomaster_difn_pin()
            .set_bit()
    });

    let sgpio = unsafe { &*device::Sgpiom::ptr() };
    sgpio.gpio554().modify(|_, w| unsafe {
        w.enbl_of_serial_gpio()
            .set_bit()
            .numbers_of_serial_gpiopins()
            .bits(16)
            .serial_gpioclk_division()
            .bits(24)
    });
}

/// Sequencer that asserts/releases the active-low BMC reset lines.
///
/// On assert `EXTRST` is driven low first; on release `SRST` is driven high
/// first, each followed by a settle delay and confirmed by reading the lines
/// back. Owns both reset pins, so it is the actuator half; the observer half is
/// [`BmcResetGate`].
pub struct BmcReset<S, E> {
    srst: S,
    extrst: E,
}

impl<S, E> BmcReset<S, E>
where
    S: StatefulOutputPin,
    E: StatefulOutputPin,
{
    /// Build a sequencer from the two reset pins.
    pub const fn new(srst: S, extrst: E) -> Self {
        Self { srst, extrst }
    }

    /// Assert both resets (drive low). Returns whether both read back asserted.
    #[must_use]
    pub fn assert(&mut self) -> bool {
        let _ = self.extrst.set_low();
        crate::delay_us(RESET_SETTLE_US);
        let _ = self.srst.set_low();
        crate::delay_us(RESET_SETTLE_US);
        self.is_asserted()
    }

    /// Release both resets (drive high). Returns whether both read back released.
    #[must_use]
    pub fn release(&mut self) -> bool {
        let _ = self.srst.set_high();
        crate::delay_us(RESET_SETTLE_US);
        let _ = self.extrst.set_high();
        crate::delay_us(RESET_SETTLE_US);
        self.is_released()
    }

    /// Both reset lines currently held low.
    #[must_use]
    pub fn is_asserted(&mut self) -> bool {
        self.srst.is_set_low().unwrap_or(false) && self.extrst.is_set_low().unwrap_or(false)
    }

    /// Both reset lines currently released high.
    #[must_use]
    pub fn is_released(&mut self) -> bool {
        self.srst.is_set_high().unwrap_or(false) && self.extrst.is_set_high().unwrap_or(false)
    }
}

impl BmcReset<SgpioOutputPin, SgpioOutputPin> {
    /// AST1060 "prot" sequencer: SGPIO master A–D bits 8/9.
    #[must_use]
    pub fn prot() -> Self {
        configure_sgpio_master();
        Self::new(unsafe { SgpioOutputPin::new(PROT_SRST_MASK) }, unsafe {
            SgpioOutputPin::new(PROT_EXTRST_MASK)
        })
    }
}

impl BmcReset<gpiom::PM5<Output<PushPull>>, gpioh::PH2<Output<PushPull>>> {
    /// "dcscm" sequencer: native `GPIO_M5` (SRST) and `GPIO_H2` (EXTRST).
    ///
    /// Configures both pins as push-pull outputs (their released state).
    #[must_use]
    pub fn dcscm() -> Self {
        let srst = unsafe { gpiom::GPIOM::new_global() }
            .split()
            .pm5
            .into_push_pull_output();
        let extrst = unsafe { gpioh::GPIOH::new_global() }
            .split()
            .ph2
            .into_push_pull_output();
        Self::new(srst, extrst)
    }
}

/// External-flash access gate backed by the BMC reset outputs.
///
/// Reports the gate open only while the RoT holds both BMC resets low, so the
/// external-flash server refuses operations whenever the host could be driving
/// its own bus. This is the observer half: it only reads the lines. The
/// orchestrator owns asserting them via [`BmcReset`].
pub struct BmcResetGate<S, E> {
    srst: S,
    extrst: E,
}

impl<S, E> BmcResetGate<S, E> {
    /// Gate observing the two reset lines.
    pub const fn new(srst: S, extrst: E) -> Self {
        Self { srst, extrst }
    }
}

impl BmcResetGate<SgpioOutputPin, SgpioOutputPin> {
    /// Gate for the AST1060 "prot" board routing.
    #[must_use]
    pub fn prot() -> Self {
        Self::new(unsafe { SgpioOutputPin::new(PROT_SRST_MASK) }, unsafe {
            SgpioOutputPin::new(PROT_EXTRST_MASK)
        })
    }
}

impl BmcResetGate<gpiom::PM5<Output<PushPull>>, gpioh::PH2<Output<PushPull>>> {
    /// Gate for the "dcscm" board routing (native `GPIO_M5` / `GPIO_H2`).
    ///
    /// Observes the pins without re-driving them; the actuator configures them.
    #[must_use]
    pub fn dcscm() -> Self {
        Self::new(unsafe { gpiom::PM5::<Output<PushPull>>::steal() }, unsafe {
            gpioh::PH2::<Output<PushPull>>::steal()
        })
    }
}

impl<S, E> BusAccessGate for BmcResetGate<S, E>
where
    S: StatefulOutputPin,
    E: StatefulOutputPin,
{
    type Error = ErrorCode;

    fn ensure_open(&mut self) -> Result<(), ErrorCode> {
        let srst_low = self.srst.is_set_low().unwrap_or(false);
        let extrst_low = self.extrst.is_set_low().unwrap_or(false);
        if srst_low && extrst_low {
            Ok(())
        } else {
            Err(error::FLASH_AST10X0_GATE_CLOSED)
        }
    }
}

/// Assert or release the "prot" board BMC reset outputs.
///
/// Convenience wrapper over [`BmcReset::prot`] for the SGPIO bring-up path.
/// Returns whether the requested state was confirmed by readback.
#[must_use]
pub fn set_bmc_resets(asserted: bool) -> bool {
    let mut reset = BmcReset::prot();
    if asserted {
        reset.assert()
    } else {
        reset.release()
    }
}
