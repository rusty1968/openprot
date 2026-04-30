// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use embedded_io::{Error, ErrorKind, ErrorType, Write};
use openprot_mctp_transport_serial::{SerialError, SerialPort};
use usart_client::{ClientError, UsartClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcSerialError {
    Io,
    Server,
    InvalidResponse,
    BufferTooSmall,
}

impl Error for IpcSerialError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

/// IPC serial backend for AST10x0 userspace USART driver.
pub struct Ast10x0IpcSerial {
    client: UsartClient,
}

impl Ast10x0IpcSerial {
    pub const fn new(handle: u32) -> Self {
        Self {
            client: UsartClient::new(handle),
        }
    }

    pub const fn from_client(client: UsartClient) -> Self {
        Self { client }
    }
}

impl ErrorType for Ast10x0IpcSerial {
    type Error = IpcSerialError;
}

impl Write for Ast10x0IpcSerial {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.client.write(buf).map_err(|e| match e {
            ClientError::IpcError(_) => IpcSerialError::Io,
            ClientError::ServerError(_) => IpcSerialError::Server,
            ClientError::InvalidResponse => IpcSerialError::InvalidResponse,
            ClientError::BufferTooSmall => IpcSerialError::BufferTooSmall,
        })
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl SerialPort for Ast10x0IpcSerial {
    fn configure(&mut self, baud_rate: u32) -> Result<(), SerialError> {
        self.client.configure(baud_rate).map_err(|e| match e {
            ClientError::IpcError(_) => SerialError::Io,
            ClientError::ServerError(_) => SerialError::Invalid,
            ClientError::InvalidResponse => SerialError::Invalid,
            ClientError::BufferTooSmall => SerialError::Invalid,
        })
    }
}
