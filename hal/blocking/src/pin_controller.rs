// Licensed under the Apache-2.0 license

//! OpenProt PinController trait implementation for ASPEED AST1060
//!
//! This module provides a bridge between OpenProt's generic PinController trait
//! and ASPEED's hardware-specific pinctrl system. It allows OpenProt peripherals
//! to configure pin multiplexing using a standardized interface while leveraging
//! ASPEED's comprehensive pin control capabilities.

#![allow(unused_imports)]

use crate::pinctrl::{self, PinctrlPin};
use heapless::FnvIndexMap;
use core::fmt;

/// Represents a physical pin on the target hardware
/// 
/// This is intentionally opaque to allow different platforms to represent
/// pins in the most efficient way (e.g., port+pin, linear index, etc.)
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct PhysicalPin(u32);

impl PhysicalPin {
    /// Create a new physical pin identifier
    /// 
    /// The internal representation is platform-specific
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
    
    /// Get the raw pin identifier
    pub const fn id(&self) -> u32 {
        self.0
    }
}

/// Pin function assignments that a physical pin can serve
/// 
/// This enum covers common peripheral functions found across embedded platforms.
/// Platforms can extend this with custom functions via the `Custom` variant.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PinFunction {
    /// General Purpose Input/Output
    Gpio,
    /// SPI controller (with instance number)
    Spi(u8),
    /// I2C controller (with instance number) 
    I2c(u8),
    /// UART controller (with instance number)
    Uart(u8),
    /// PWM output (with channel number)
    Pwm(u8),
    /// Timer input/output (with timer number)
    Timer(u8),
    /// ADC input (with channel number)
    Adc(u8),
    /// DAC output (with channel number)
    Dac(u8),
    /// Clock output
    ClockOut,
    /// External interrupt input
    ExternalInterrupt,
    /// Platform-specific function
    Custom(u32),
}

/// Pin-specific role within a peripheral function
/// 
/// For peripherals that use multiple pins, this specifies the role of each pin
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PinRole {
    /// Clock signal (SPI CLK, I2C SCL, etc.)
    Clock,
    /// Data signal (SPI MOSI/MISO, I2C SDA, UART TX/RX, etc.)
    Data,
    /// Chip select or enable signal
    ChipSelect,
    /// Reset or control signal
    Reset,
    /// Interrupt or status signal
    Interrupt,
    /// Power control
    Power,
    /// Generic signal for simple peripherals
    Signal,
}


/// Errors that can occur during pin control operations
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PinControlError {
    /// The specified pin does not exist on this platform
    InvalidPin(PhysicalPin),
    /// The requested function is not available on this pin
    UnsupportedFunction {
        pin: PhysicalPin,
        function: PinFunction,
    },
    /// Pin is already configured for a different function
    PinInUse {
        pin: PhysicalPin,
        current_function: PinFunction,
        requested_function: PinFunction,
    },
    /// Multiple pins are configured for conflicting functions
    ResourceConflict {
        pin1: PhysicalPin,
        pin2: PhysicalPin,
        shared_resource: &'static str,
    },
    /// Hardware-specific error occurred
    HardwareError(&'static str),
    /// Operation timed out
    Timeout,
    /// Invalid configuration parameters
    InvalidConfiguration,
    /// Operation not permitted in current state
    PermissionDenied,
}

impl fmt::Display for PinControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPin(pin) => write!(f, "Invalid pin: {:?}", pin),
            Self::UnsupportedFunction { pin, function } => {
                write!(f, "Function {:?} not supported on pin {:?}", function, pin)
            }
            Self::PinInUse { pin, current_function, requested_function } => {
                write!(f, "Pin {:?} in use for {:?}, cannot configure for {:?}", 
                       pin, current_function, requested_function)
            }
            Self::ResourceConflict { pin1, pin2, shared_resource } => {
                write!(f, "Resource conflict: pins {:?} and {:?} share {}", 
                       pin1, pin2, shared_resource)
            }
            Self::HardwareError(msg) => write!(f, "Hardware error: {}", msg),
            Self::Timeout => write!(f, "Operation timed out"),
            Self::InvalidConfiguration => write!(f, "Invalid configuration"),
            Self::PermissionDenied => write!(f, "Permission denied"),
        }
    }
}

/// Core trait for pin control and multiplexing
/// 
/// This trait provides the fundamental interface for configuring pin functions
/// across different hardware platforms. Implementations handle the platform-specific
/// details of pin multiplexing while exposing a consistent interface.
pub trait PinController {
    /// Configure a single pin for a specific function
    /// 
    /// This will change the pin's multiplexer settings to route the specified
    /// peripheral function to the physical pin.
    fn configure_pin(&mut self, config: PinConfig) -> Result<(), PinControlError>;
    
    /// Configure multiple pins atomically
    /// 
    /// This ensures that either all pins are configured successfully, or none are.
    /// This is important for peripherals that require multiple pins to function.
    fn configure_pins(&mut self, configs: &[PinConfig]) -> Result<(), PinControlError>;
    
    /// Check if a pin configuration would conflict with existing configurations
    /// 
    /// This allows validation before attempting to configure pins, which is useful
    /// for error handling and configuration planning.
    fn check_conflicts(&self, configs: &[PinConfig]) -> Result<(), PinControlError>;
    
    /// Get the current function of a pin
    /// 
    /// Returns `None` if the pin is not configured or is in a default state.
    fn get_pin_function(&self, pin: PhysicalPin) -> Option<PinFunction>;
    
    /// Reset a pin to its default/unconfigured state
    /// 
    /// This typically means returning the pin to GPIO mode or a platform-specific
    /// default function.
    fn reset_pin(&mut self, pin: PhysicalPin) -> Result<(), PinControlError>;
    
    /// Reset multiple pins to their default states
    fn reset_pins(&mut self, pins: &[PhysicalPin]) -> Result<(), PinControlError>;
    
    /// Get a list of all available pins on this platform
    fn available_pins(&self) -> &[PhysicalPin];
    
    /// Get supported functions for a specific pin
    /// 
    /// Returns a list of functions that this pin can be configured for.
    /// This is platform-specific as different pins may support different functions.
    fn supported_functions(&self, pin: PhysicalPin) -> Result<&[PinFunction], PinControlError>;
}

