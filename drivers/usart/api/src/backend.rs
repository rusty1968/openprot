// Licensed under the Apache-2.0 license

use crate::protocol::UsartError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendError {
    InvalidOperation,
    InvalidConfiguration,
    BufferTooSmall,
    Busy,
    Timeout,
    InternalError,
}

impl From<BackendError> for UsartError {
    fn from(value: BackendError) -> Self {
        match value {
            BackendError::InvalidOperation => UsartError::InvalidOperation,
            BackendError::InvalidConfiguration => UsartError::InvalidConfiguration,
            BackendError::BufferTooSmall => UsartError::BufferTooSmall,
            BackendError::Busy => UsartError::Busy,
            BackendError::Timeout => UsartError::Timeout,
            BackendError::InternalError => UsartError::InternalError,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsartConfig {
    pub baud_rate: u32,
    pub parity: Parity,
    pub stop_bits: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineStatus(pub u8);

pub trait UsartBackend {
    fn configure(&mut self, config: UsartConfig) -> Result<(), BackendError>;
    fn write(&mut self, data: &[u8]) -> Result<usize, BackendError>;
    fn read(&mut self, out: &mut [u8]) -> Result<usize, BackendError>;
    fn line_status(&self) -> Result<LineStatus, BackendError>;
    fn enable_interrupts(&mut self, mask: u16) -> Result<(), BackendError>;
    fn disable_interrupts(&mut self, mask: u16) -> Result<(), BackendError>;
}
