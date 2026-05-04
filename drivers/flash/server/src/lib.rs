// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

pub mod runtime;

use flash_api::backend::{BackendError, FlashBackend, IrqMask};
use flash_api::{FlashError, FlashOp, FlashRequestHeader, FlashResponseHeader};

/// Per-request scratch buffers sized for `MAX_PAYLOAD_SIZE` plus headers
/// and a little slack.  Concrete platform binaries pick their own sizes
/// when they call `runtime::run`; these are the defaults exposed for
/// convenience.
pub const MAX_REQUEST_SIZE: usize = 512;
pub const MAX_RESPONSE_SIZE: usize = 512;

/// Outcome of a single dispatch call.
pub enum DispatchOutcome {
    Respond(usize),
    Queued,
}

pub fn dispatch_request<B: FlashBackend>(
    backend: &mut B,
    key: B::RouteKey,
    pending: &mut runtime::PendingRequest<B::RouteKey>,
    client_channel: u32,
    request: &[u8],
    response: &mut [u8],
) -> DispatchOutcome {
    if request.len() < FlashRequestHeader::SIZE {
        return DispatchOutcome::Respond(encode_error(response, FlashError::InvalidOperation));
    }

    let hdr_bytes = &request[..FlashRequestHeader::SIZE];
    let Some(hdr) = zerocopy::Ref::<_, FlashRequestHeader>::from_bytes(hdr_bytes).ok() else {
        return DispatchOutcome::Respond(encode_error(response, FlashError::InvalidOperation));
    };

    let op = match hdr.operation() {
        Ok(op) => op,
        Err(e) => return DispatchOutcome::Respond(encode_error(response, e)),
    };

    let payload_len = hdr.payload_length();
    if request.len() < FlashRequestHeader::SIZE + payload_len {
        return DispatchOutcome::Respond(encode_error(response, FlashError::InvalidOperation));
    }
    let payload = &request[FlashRequestHeader::SIZE..FlashRequestHeader::SIZE + payload_len];

    let address = hdr.address_value();
    let length = hdr.length_value();

    match op {
        FlashOp::Exists => match backend.exists(key) {
            Ok(present) => DispatchOutcome::Respond(encode_value(response, u32::from(present), 0)),
            Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
        },

        FlashOp::GetCapacity => {
            let info = backend.info(key);
            DispatchOutcome::Respond(encode_value(response, info.capacity, 0))
        }

        FlashOp::GetChunkSize => {
            let info = backend.info(key);
            DispatchOutcome::Respond(encode_value(response, info.chunk_size, 0))
        }

        FlashOp::Read => {
            let payload_offset = FlashResponseHeader::SIZE;
            let payload_capacity = response.len().saturating_sub(payload_offset);
            let read_buf_len =
                core::cmp::min(length as usize, payload_capacity).min(u16::MAX as usize);

            match backend.read(
                key,
                address,
                &mut response[payload_offset..payload_offset + read_buf_len],
            ) {
                Ok(n) => {
                    let hdr = FlashResponseHeader::success(n as u32, n as u16);
                    response[..FlashResponseHeader::SIZE]
                        .copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
                    DispatchOutcome::Respond(FlashResponseHeader::SIZE + n)
                }
                Err(BackendError::WouldBlock) => {
                    if !pending.park(client_channel, key, request) {
                        return DispatchOutcome::Respond(encode_error(response, FlashError::Busy));
                    }
                    let _ = backend.enable_interrupts(IrqMask::OPERATION_COMPLETE);
                    DispatchOutcome::Queued
                }
                Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
            }
        }

        FlashOp::Write => {
            if (length as usize) != payload_len {
                return DispatchOutcome::Respond(encode_error(
                    response,
                    FlashError::InvalidLength,
                ));
            }
            match backend.write(key, address, payload) {
                Ok(n) => DispatchOutcome::Respond(encode_value(response, n as u32, 0)),
                Err(BackendError::WouldBlock) => {
                    if !pending.park(client_channel, key, request) {
                        return DispatchOutcome::Respond(encode_error(response, FlashError::Busy));
                    }
                    let _ = backend.enable_interrupts(IrqMask::OPERATION_COMPLETE);
                    DispatchOutcome::Queued
                }
                Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
            }
        }

        FlashOp::Erase => match backend.erase(key, address, length) {
            Ok(()) => DispatchOutcome::Respond(encode_value(response, length, 0)),
            Err(BackendError::WouldBlock) => {
                if !pending.park(client_channel, key, request) {
                    return DispatchOutcome::Respond(encode_error(response, FlashError::Busy));
                }
                let _ = backend.enable_interrupts(IrqMask::OPERATION_COMPLETE);
                DispatchOutcome::Queued
            }
            Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
        },
    }
}

fn encode_error(response: &mut [u8], error: FlashError) -> usize {
    let hdr = FlashResponseHeader::error(error);
    response[..FlashResponseHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
    FlashResponseHeader::SIZE
}

fn encode_value(response: &mut [u8], value: u32, payload_len: u16) -> usize {
    let hdr = FlashResponseHeader::success(value, payload_len);
    response[..FlashResponseHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
    FlashResponseHeader::SIZE + payload_len as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use flash_api::backend::{BackendError, FlashBackend, FlashInfo};
    use zerocopy::IntoBytes;

    #[derive(Default)]
    struct MockBackend {
        exists: bool,
        last_key: Option<()>,
        last_address: Option<u32>,
    }

    impl FlashBackend for MockBackend {
        type RouteKey = ();

        fn info(&self, _key: ()) -> FlashInfo {
            FlashInfo {
                capacity: 0,
                chunk_size: 0,
                erase_size: 0,
            }
        }

        fn exists(&mut self, key: ()) -> Result<bool, BackendError> {
            self.last_key = Some(key);
            Ok(self.exists)
        }

        fn read(
            &mut self,
            _key: (),
            _address: u32,
            _out: &mut [u8],
        ) -> Result<usize, BackendError> {
            Err(BackendError::InvalidOperation)
        }

        fn write(
            &mut self,
            _key: (),
            _address: u32,
            _data: &[u8],
        ) -> Result<usize, BackendError> {
            Err(BackendError::InvalidOperation)
        }

        fn erase(
            &mut self,
            _key: (),
            _address: u32,
            _length: u32,
        ) -> Result<(), BackendError> {
            Err(BackendError::InvalidOperation)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
    struct KeyedKey(u8);

    #[derive(Default)]
    struct KeyedBackend {
        last_call: Option<(KeyedKey, &'static str, u32)>,
    }

    impl FlashBackend for KeyedBackend {
        type RouteKey = KeyedKey;

        fn info(&self, _key: KeyedKey) -> FlashInfo {
            FlashInfo {
                capacity: 0,
                chunk_size: 0,
                erase_size: 0,
            }
        }

        fn exists(&mut self, _key: KeyedKey) -> Result<bool, BackendError> {
            Ok(true)
        }

        fn read(
            &mut self,
            key: KeyedKey,
            address: u32,
            _out: &mut [u8],
        ) -> Result<usize, BackendError> {
            self.last_call = Some((key, "read", address));
            Ok(0)
        }

        fn write(
            &mut self,
            key: KeyedKey,
            address: u32,
            _data: &[u8],
        ) -> Result<usize, BackendError> {
            self.last_call = Some((key, "write", address));
            Ok(0)
        }

        fn erase(
            &mut self,
            key: KeyedKey,
            address: u32,
            _length: u32,
        ) -> Result<(), BackendError> {
            self.last_call = Some((key, "erase", address));
            Ok(())
        }
    }

    #[test]
    fn flash_exists_returns_1_when_backend_reports_present() {
        let mut backend = MockBackend {
            exists: true,
            ..MockBackend::default()
        };
        let mut pending = runtime::PendingRequest::<()>::new();
        let mut response = [0u8; MAX_RESPONSE_SIZE];

        let request_hdr = FlashRequestHeader::new(FlashOp::Exists, 0, 0, 0);
        let request = request_hdr.as_bytes();

        let outcome =
            dispatch_request(&mut backend, (), &mut pending, 1, request, &mut response);
        let DispatchOutcome::Respond(resp_len) = outcome else {
            panic!("exists should respond immediately");
        };

        assert_eq!(resp_len, FlashResponseHeader::SIZE);
        let resp_ref = zerocopy::Ref::<_, FlashResponseHeader>::from_bytes(
            &response[..FlashResponseHeader::SIZE],
        )
        .expect("response header should decode");
        let resp = *resp_ref;
        assert!(resp.is_success());
        assert_eq!(resp.payload_length(), 0);
        assert_eq!(resp.value_word(), 1);
    }

    #[test]
    fn flash_exists_returns_0_when_backend_reports_absent() {
        let mut backend = MockBackend {
            exists: false,
            ..MockBackend::default()
        };
        let mut pending = runtime::PendingRequest::<()>::new();
        let mut response = [0u8; MAX_RESPONSE_SIZE];

        let request_hdr = FlashRequestHeader::new(FlashOp::Exists, 0, 0, 0);
        let request = request_hdr.as_bytes();

        let outcome =
            dispatch_request(&mut backend, (), &mut pending, 1, request, &mut response);
        let DispatchOutcome::Respond(resp_len) = outcome else {
            panic!("exists should respond immediately");
        };

        assert_eq!(resp_len, FlashResponseHeader::SIZE);
        let resp_ref = zerocopy::Ref::<_, FlashResponseHeader>::from_bytes(
            &response[..FlashResponseHeader::SIZE],
        )
        .expect("response header should decode");
        let resp = *resp_ref;
        assert!(resp.is_success());
        assert_eq!(resp.payload_length(), 0);
        assert_eq!(resp.value_word(), 0);
    }

    #[test]
    fn dispatch_routes_read_to_correct_key() {
        let mut backend = KeyedBackend::default();
        let mut pending = runtime::PendingRequest::<KeyedKey>::new();
        let mut response = [0u8; MAX_RESPONSE_SIZE];

        let request_hdr = FlashRequestHeader::new(FlashOp::Read, 0x1000, 0, 0);
        let request = request_hdr.as_bytes();

        let _ = dispatch_request(
            &mut backend,
            KeyedKey(7),
            &mut pending,
            1,
            request,
            &mut response,
        );
        assert_eq!(backend.last_call, Some((KeyedKey(7), "read", 0x1000)));

        let _ = dispatch_request(
            &mut backend,
            KeyedKey(9),
            &mut pending,
            1,
            request,
            &mut response,
        );
        assert_eq!(backend.last_call, Some((KeyedKey(9), "read", 0x1000)));
    }

    #[test]
    fn pending_request_carries_key() {
        let mut pending = runtime::PendingRequest::<KeyedKey>::new();
        let request_hdr = FlashRequestHeader::new(FlashOp::Read, 0x2000, 0, 0);
        let request = request_hdr.as_bytes();

        assert!(pending.park(11, KeyedKey(3), request));

        let mut buf = [0u8; MAX_REQUEST_SIZE];
        let (channel, key, len) = pending.take_into(&mut buf).expect("take_into succeeds");
        assert_eq!(channel, 11);
        assert_eq!(key, KeyedKey(3));
        assert_eq!(len, request.len());
    }
}
