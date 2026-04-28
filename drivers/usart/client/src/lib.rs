// Licensed under the Apache-2.0 license

#![no_std]

use usart_api::{
    UsartError, UsartOp, UsartRequestHeader, UsartResponseHeader, MAX_PAYLOAD_SIZE,
};
use userspace::syscall;
use userspace::time::Instant;

const MAX_BUF_SIZE: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    IpcError(pw_status::Error),
    ServerError(UsartError),
    InvalidResponse,
    BufferTooSmall,
}

impl From<pw_status::Error> for ClientError {
    fn from(e: pw_status::Error) -> Self {
        Self::IpcError(e)
    }
}

pub struct UsartClient {
    handle: u32,
}

impl UsartClient {
    pub const fn new(handle: u32) -> Self {
        Self { handle }
    }

    pub fn configure(&self, baud_rate: u32) -> Result<(), ClientError> {
        let arg0 = (baud_rate & 0xffff) as u16;
        let arg1 = (baud_rate >> 16) as u16;
        self.call_no_payload(UsartOp::Configure, arg0, arg1)
    }

    pub fn write(&self, data: &[u8]) -> Result<usize, ClientError> {
        if data.len() > MAX_PAYLOAD_SIZE {
            return Err(ClientError::BufferTooSmall);
        }

        let mut req = [0u8; MAX_BUF_SIZE];
        let mut resp = [0u8; MAX_BUF_SIZE];

        let hdr = UsartRequestHeader::new(UsartOp::Write, 0, 0, data.len() as u16);
        req[..UsartRequestHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
        req[UsartRequestHeader::SIZE..UsartRequestHeader::SIZE + data.len()].copy_from_slice(data);

        let resp_len = syscall::channel_transact(
            self.handle,
            &req[..UsartRequestHeader::SIZE + data.len()],
            &mut resp,
            Instant::MAX,
        )?;

        parse_no_payload_response(&resp[..resp_len])?;
        Ok(data.len())
    }

    pub fn read(&self, out: &mut [u8]) -> Result<usize, ClientError> {
        if out.len() > MAX_PAYLOAD_SIZE {
            return Err(ClientError::BufferTooSmall);
        }

        let mut req = [0u8; MAX_BUF_SIZE];
        let mut resp = [0u8; MAX_BUF_SIZE];

        let hdr = UsartRequestHeader::new(UsartOp::Read, out.len() as u16, 0, 0);
        req[..UsartRequestHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));

        let resp_len = syscall::channel_transact(
            self.handle,
            &req[..UsartRequestHeader::SIZE],
            &mut resp,
            Instant::MAX,
        )?;

        parse_payload_response(&resp[..resp_len], out)
    }

    fn call_no_payload(&self, op: UsartOp, arg0: u16, arg1: u16) -> Result<(), ClientError> {
        let mut req = [0u8; MAX_BUF_SIZE];
        let mut resp = [0u8; MAX_BUF_SIZE];

        let hdr = UsartRequestHeader::new(op, arg0, arg1, 0);
        req[..UsartRequestHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));

        let resp_len = syscall::channel_transact(
            self.handle,
            &req[..UsartRequestHeader::SIZE],
            &mut resp,
            Instant::MAX,
        )?;

        parse_no_payload_response(&resp[..resp_len])
    }
}

fn parse_no_payload_response(resp: &[u8]) -> Result<(), ClientError> {
    if resp.len() < UsartResponseHeader::SIZE {
        return Err(ClientError::InvalidResponse);
    }

    let hdr_bytes = &resp[..UsartResponseHeader::SIZE];
    let Some(hdr) = zerocopy::Ref::<_, UsartResponseHeader>::from_bytes(hdr_bytes).ok() else {
        return Err(ClientError::InvalidResponse);
    };

    if hdr.is_success() {
        Ok(())
    } else {
        Err(ClientError::ServerError(hdr.error_code()))
    }
}

fn parse_payload_response(resp: &[u8], out: &mut [u8]) -> Result<usize, ClientError> {
    if resp.len() < UsartResponseHeader::SIZE {
        return Err(ClientError::InvalidResponse);
    }

    let hdr_bytes = &resp[..UsartResponseHeader::SIZE];
    let Some(hdr) = zerocopy::Ref::<_, UsartResponseHeader>::from_bytes(hdr_bytes).ok() else {
        return Err(ClientError::InvalidResponse);
    };

    if !hdr.is_success() {
        return Err(ClientError::ServerError(hdr.error_code()));
    }

    let len = hdr.payload_length();
    if len > out.len() || resp.len() < UsartResponseHeader::SIZE + len {
        return Err(ClientError::InvalidResponse);
    }

    out[..len].copy_from_slice(&resp[UsartResponseHeader::SIZE..UsartResponseHeader::SIZE + len]);
    Ok(len)
}
