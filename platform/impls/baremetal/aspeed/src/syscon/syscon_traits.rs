//! OpenPRoT HAL trait implementations for the AST1060 System Controller
//!
//! This module implements the OpenPRoT HAL blocking traits for system control,
//! providing a clean interface that maps to the underlying hardware implementation.

use super::syscon::{SysCon, Error as SysConError, ClockId, ResetId, ClockConfig};
use core::time::Duration;
use openprot_hal_blocking::system_control::{ClockControl, ResetControl, ErrorType, Error as HalError, ErrorKind};

/// Implementation of HAL Error trait for our SysCon error type
impl HalError for SysConError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::ClockAlreadyEnabled => ErrorKind::ClockAlreadyEnabled,
            Self::ClockAlreadyDisabled => ErrorKind::ClockAlreadyDisabled,
            Self::InvalidClockId => ErrorKind::ClockNotFound,
            Self::InvalidClockFrequency => ErrorKind::InvalidClockFrequency,
            Self::ClockConfigurationFailed => ErrorKind::ClockConfigurationFailed,
            Self::InvalidResetId => ErrorKind::InvalidResetId,
            Self::HardwareFailure => ErrorKind::HardwareFailure,
            Self::PermissionDenied => ErrorKind::PermissionDenied,
            Self::InvalidClkSource => ErrorKind::PermissionDenied,
            Self::Timeout => ErrorKind::Timeout,
        }
    }
}

/// Implementation of ErrorType trait for HAL compatibility
impl ErrorType for SysCon {
    type Error = SysConError;
}

/// Implementation of ClockControl trait for OpenPRoT HAL
impl ClockControl for SysCon {
    type ClockId = ClockId;
    type ClockConfig = ClockConfig;

    fn enable(&mut self, clock_id: &Self::ClockId) -> Result<(), Self::Error> {
        self.enable_clock(*clock_id as u8)
    }

    fn disable(&mut self, clock_id: &Self::ClockId) -> Result<(), Self::Error> {
        self.disable_clock(*clock_id as u8)
    }

    fn set_frequency(
        &mut self,
        clock_id: &Self::ClockId,
        frequency_hz: u64,
    ) -> Result<(), Self::Error> {
        self.set_frequency(*clock_id, frequency_hz)
    }

    fn get_frequency(&self, clock_id: &Self::ClockId) -> Result<u64, Self::Error> {
        self.get_frequency(*clock_id)
    }

    fn configure(
        &mut self,
        clock_id: &Self::ClockId,
        config: Self::ClockConfig,
    ) -> Result<(), Self::Error> {
        self.configure_clock(*clock_id, &config)
    }

    fn get_config(&self, clock_id: &Self::ClockId) -> Result<Self::ClockConfig, Self::Error> {
        self.get_clock_config(*clock_id)
    }
}

/// Implementation of ResetControl trait for OpenPRoT HAL
impl ResetControl for SysCon {
    type ResetId = ResetId;

    fn reset_assert(&mut self, reset_id: &Self::ResetId) -> Result<(), Self::Error> {
        self.reset_assert(*reset_id as u8)
    }

    fn reset_deassert(&mut self, reset_id: &Self::ResetId) -> Result<(), Self::Error> {
        self.reset_deassert(*reset_id as u8)
    }

    fn reset_pulse(
        &mut self,
        reset_id: &Self::ResetId,
        duration: Duration,
    ) -> Result<(), Self::Error> {
        self.reset_pulse(*reset_id as u8, duration)
    }

    fn reset_is_asserted(&self, reset_id: &Self::ResetId) -> Result<bool, Self::Error> {
        self.reset_is_asserted(*reset_id as u8)
    }
}
