// Licensed under the Apache-2.0 license

//! # Non-blocking I2C Hardware Abstraction Traits
//!
//! This module defines non-blocking traits for I2C hardware abstraction, specifically
//! for polling-based and interrupt-driven slave operations that don't block the caller.
//!
//! These traits complement the blocking traits in `openprot-hal-blocking` by providing
//! non-blocking alternatives suitable for async code, main loops, and interrupt handlers.

use embedded_hal::i2c::{AddressMode, SevenBitAddress};
use openprot_hal_blocking::i2c_hardware::slave::{
    I2cSEvent, I2cSlaveBuffer, I2cSlaveCore, I2cSlaveInterrupts,
};

/// Non-blocking slave event handling (async/polling pattern)
///
/// This trait provides non-blocking event operations suitable for async code,
/// main loops, or interrupt-driven architectures. All operations return
/// immediately without blocking the caller.
pub trait I2cSlaveEventPolling<A: AddressMode = SevenBitAddress>: I2cSlaveInterrupts<A> {
    /// Check for pending slave events without blocking
    ///
    /// Returns the next available slave event if one is pending, or None
    /// if no events are waiting. This is useful for polling-based event
    /// handling or in main loops that need to be non-blocking.
    fn poll_slave_events(&mut self) -> Result<Option<I2cSEvent>, Self::Error>;

    /// Handle a specific slave event (called from ISR or event loop)
    ///
    /// Processes a slave event and performs any necessary hardware actions.
    /// This method encapsulates the event-specific logic and can be called
    /// from interrupt handlers or main event loops. Always returns immediately.
    fn handle_slave_event(&mut self, event: I2cSEvent) -> Result<(), Self::Error>;

    /// Non-blocking check if a specific event is pending
    ///
    /// Returns true if the specified event is currently pending, false otherwise.
    /// Useful for checking specific conditions without consuming the event.
    fn is_event_pending(&self, event: I2cSEvent) -> Result<bool, Self::Error>;
}

/// Complete non-blocking slave implementation
///
/// This trait represents a full non-blocking slave implementation that supports
/// all non-blocking slave operations. Perfect for interrupt-driven or
/// polling-based implementations that cannot afford to block.
pub trait I2cSlaveNonBlocking<A: AddressMode = SevenBitAddress>:
    I2cSlaveCore<A> + I2cSlaveBuffer<A> + I2cSlaveEventPolling<A>
{
}

/// Blanket implementation: any type implementing core + buffer + polling events gets non-blocking slave
impl<T, A: AddressMode> I2cSlaveNonBlocking<A> for T where
    T: I2cSlaveCore<A> + I2cSlaveBuffer<A> + I2cSlaveEventPolling<A>
{
}
