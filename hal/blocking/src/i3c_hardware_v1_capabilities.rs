// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! I3C target (device) mode hardware abstraction traits.
//!
//! These are the low-level, chip-implemented seam for an I3C device operating
//! in **target mode**: a peripheral that a controller addresses, reads, and
//! writes on the bus. They are the `FlashDriver`-level contract — synchronous,
//! in-process, and signal-free. The signal→drain reactor and any IPC client
//! API live above this seam in the service layer.
//!
//! Scope is target mode only; controller/primary-mode operations are out of
//! scope.
//!
//! ```text
//! I3cTargetErrorType (shared associated error)
//!   ├── I3cTargetIdentity   (dynamic address, infallible read)
//!   ├── I3cTargetTransfer   (inbound drain + staged response)
//!   ├── I3cTargetInterrupt  (arm/disarm the RX interrupt source)
//!   ├── I3cTargetIbi        (optional: raise an IBI)
//!   └── I3cTargetHotJoin    (optional: request bus admission)
//!
//! I3cTargetDevice = Identity + Transfer + Interrupt   (composed marker)
//! ```

/// A 7-bit I3C dynamic address assigned by the active controller.
///
/// Constructed only through validation, so an out-of-range or reserved value
/// can never reach the HAL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicAddress(u8);

impl DynamicAddress {
    /// The raw 7-bit value.
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

/// Error returned when a value is not a valid 7-bit dynamic address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidAddress;

impl TryFrom<u8> for DynamicAddress {
    type Error = InvalidAddress;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        // 0x00 is reserved; 0x7E/0x7F are reserved; the MSB must be clear.
        match value {
            0x01..=0x7D => Ok(DynamicAddress(value)),
            _ => Err(InvalidAddress),
        }
    }
}

/// The Mandatory Data Byte carried by every IBI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mdb(pub u8);

/// Shared associated error type for the I3C target traits.
///
/// A single error surface per implementor, named distinctly so it does not
/// collide with other protocols' error seams in the crate root.
pub trait I3cTargetErrorType {
    /// Hardware error surfaced by target operations.
    type Error: core::fmt::Debug;
}

/// Read-only target identity.
pub trait I3cTargetIdentity: I3cTargetErrorType {
    /// The dynamic address currently assigned by the active controller, or
    /// `None` if the device has not completed dynamic address assignment.
    ///
    /// A local state read: it cannot fail, so it returns `Option`, not
    /// `Result<Option<_>>`.
    fn dynamic_address(&self) -> Option<DynamicAddress>;
}

/// The private-transfer data path: inbound writes and staged responses.
///
/// RX and TX are the two halves of one data path and co-vary, so they live in
/// one trait rather than two.
pub trait I3cTargetTransfer: I3cTargetErrorType {
    /// Drain the next buffered inbound frame into `buf`.
    ///
    /// Returns the number of bytes written, or `None` when the RX queue is
    /// empty. The reactor calls this in response to an RX signal and drains
    /// until it returns `None`; it is not a poll.
    fn read_frame(&mut self, buf: &mut [u8]) -> Result<Option<usize>, Self::Error>;

    /// Stage the payload the controller receives on its next private read.
    fn write_response(&mut self, data: &[u8]) -> Result<(), Self::Error>;
}

/// Arm/disarm the RX interrupt source — the origin of the RX signal the
/// service reactor waits on.
pub trait I3cTargetInterrupt: I3cTargetErrorType {
    /// Arm the RX interrupt source.
    fn enable_rx_interrupt(&mut self) -> Result<(), Self::Error>;

    /// Disarm the RX interrupt source.
    fn disable_rx_interrupt(&mut self) -> Result<(), Self::Error>;
}

/// Optional capability: raise an In-Band Interrupt.
pub trait I3cTargetIbi: I3cTargetErrorType {
    /// Raise an IBI carrying `mdb` and an optional data `payload`.
    fn raise_ibi(&mut self, mdb: Mdb, payload: &[u8]) -> Result<(), Self::Error>;
}

/// Optional capability: request bus admission via Hot-Join.
///
/// Kept separate from [`I3cTargetIbi`]: Hot-Join carries no MDB/payload, is
/// only meaningful before addressing, and varies independently.
pub trait I3cTargetHotJoin: I3cTargetErrorType {
    /// Request bus admission via a Hot-Join IBI so the controller runs dynamic
    /// address assignment.
    fn raise_hot_join(&mut self) -> Result<(), Self::Error>;
}

/// Every functional target: identity + data path + interrupt control.
///
/// A composed marker, not a re-declaration of the parts, so its surface can
/// never drift from the traits it bundles. IBI and Hot-Join are deliberately
/// excluded — they are optional capabilities a consumer names explicitly.
pub trait I3cTargetDevice:
    I3cTargetIdentity + I3cTargetTransfer + I3cTargetInterrupt
{
}

impl<T> I3cTargetDevice for T where
    T: I3cTargetIdentity + I3cTargetTransfer + I3cTargetInterrupt
{
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_address_accepts_valid_range() {
        assert_eq!(DynamicAddress::try_from(0x01).map(|a| a.as_u8()), Ok(0x01));
        assert_eq!(DynamicAddress::try_from(0x42).map(|a| a.as_u8()), Ok(0x42));
        assert_eq!(DynamicAddress::try_from(0x7D).map(|a| a.as_u8()), Ok(0x7D));
    }

    #[test]
    fn dynamic_address_rejects_reserved() {
        assert_eq!(DynamicAddress::try_from(0x00), Err(InvalidAddress));
        assert_eq!(DynamicAddress::try_from(0x7E), Err(InvalidAddress));
        assert_eq!(DynamicAddress::try_from(0x7F), Err(InvalidAddress));
        assert_eq!(DynamicAddress::try_from(0x80), Err(InvalidAddress));
        assert_eq!(DynamicAddress::try_from(0xFF), Err(InvalidAddress));
    }

    // A minimal target that implements the mandatory capabilities, to prove
    // the composed `I3cTargetDevice` marker is granted by the blanket impl.
    #[derive(Default)]
    struct FakeTarget {
        addr: Option<u8>,
        rx: Option<usize>,
        rx_irq: bool,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct FakeError;

    impl I3cTargetErrorType for FakeTarget {
        type Error = FakeError;
    }

    impl I3cTargetIdentity for FakeTarget {
        fn dynamic_address(&self) -> Option<DynamicAddress> {
            self.addr.and_then(|a| DynamicAddress::try_from(a).ok())
        }
    }

    impl I3cTargetTransfer for FakeTarget {
        fn read_frame(&mut self, buf: &mut [u8]) -> Result<Option<usize>, Self::Error> {
            match self.rx.take() {
                Some(n) if n <= buf.len() => Ok(Some(n)),
                Some(_) => Err(FakeError),
                None => Ok(None),
            }
        }

        fn write_response(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl I3cTargetInterrupt for FakeTarget {
        fn enable_rx_interrupt(&mut self) -> Result<(), Self::Error> {
            self.rx_irq = true;
            Ok(())
        }

        fn disable_rx_interrupt(&mut self) -> Result<(), Self::Error> {
            self.rx_irq = false;
            Ok(())
        }
    }

    fn assert_is_device<T: I3cTargetDevice>() {}

    #[test]
    fn blanket_marker_is_granted() {
        assert_is_device::<FakeTarget>();
    }

    #[test]
    fn drain_returns_none_when_empty() {
        let mut t = FakeTarget {
            addr: Some(0x42),
            rx: None,
            rx_irq: false,
        };
        let mut buf = [0u8; 8];
        assert_eq!(t.read_frame(&mut buf), Ok(None));
        assert_eq!(t.dynamic_address(), DynamicAddress::try_from(0x42).ok());
    }
}
