// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

pub mod runtime;

use usart_api::backend::{BackendError, IrqMask, UsartBackend};
use usart_api::{
    PROTOCOL_VERSION, UsartConfigurePayload, UsartError, UsartOp, UsartParityWire,
    UsartRequestHeader, UsartResponseHeader,
};

pub const MAX_REQUEST_SIZE: usize = 512;
pub const MAX_RESPONSE_SIZE: usize = 512;

/// Outcome of a single dispatch call.
pub enum DispatchOutcome {
    /// Response is ready; caller should send `response[..len]` immediately.
    Respond(usize),
    /// No data was available yet.  The runtime stored the pending request and
    /// will respond once the RX interrupt fires.  Caller must NOT call
    /// `channel_respond` for this request.
    Queued,
}

pub fn dispatch_request<B: UsartBackend>(
    backend: &mut B,
    pending: &mut runtime::PendingIo,
    client_channel: u32,
    request: &[u8],
    response: &mut [u8],
) -> DispatchOutcome {
    if request.len() < UsartRequestHeader::SIZE {
        return DispatchOutcome::Respond(encode_error(response, UsartError::InvalidOperation));
    }

    let hdr_bytes = &request[..UsartRequestHeader::SIZE];
    let Some(hdr) = zerocopy::Ref::<_, UsartRequestHeader>::from_bytes(hdr_bytes).ok() else {
        return DispatchOutcome::Respond(encode_error(response, UsartError::InvalidOperation));
    };

    if hdr.protocol_version() != PROTOCOL_VERSION {
        return DispatchOutcome::Respond(encode_error(response, UsartError::UnsupportedVersion));
    }

    let op = match hdr.operation() {
        Ok(op) => op,
        Err(e) => return DispatchOutcome::Respond(encode_error(response, e)),
    };

    let payload_len = hdr.payload_length();
    if request.len() < UsartRequestHeader::SIZE + payload_len {
        return DispatchOutcome::Respond(encode_error(response, UsartError::InvalidOperation));
    }
    let payload = &request[UsartRequestHeader::SIZE..UsartRequestHeader::SIZE + payload_len];

    match op {
        UsartOp::Configure => {
            if payload_len != UsartConfigurePayload::SIZE {
                return DispatchOutcome::Respond(encode_error(response, UsartError::InvalidConfiguration));
            }

            let Some(cfg_payload) = zerocopy::Ref::<_, UsartConfigurePayload>::from_bytes(payload).ok() else {
                return DispatchOutcome::Respond(encode_error(response, UsartError::InvalidConfiguration));
            };

            let parity = match UsartParityWire::try_from(cfg_payload.parity) {
                Ok(UsartParityWire::None) => usart_api::backend::Parity::None,
                Ok(UsartParityWire::Even) => usart_api::backend::Parity::Even,
                Ok(UsartParityWire::Odd) => usart_api::backend::Parity::Odd,
                Err(e) => return DispatchOutcome::Respond(encode_error(response, e)),
            };

            if cfg_payload.stop_bits == 0 || cfg_payload.stop_bits > 2 {
                return DispatchOutcome::Respond(encode_error(response, UsartError::InvalidConfiguration));
            }

            let cfg = usart_api::backend::UsartConfig {
                baud_rate: cfg_payload.baud_rate_value(),
                parity,
                stop_bits: cfg_payload.stop_bits,
            };
            match backend.configure(cfg) {
                Ok(()) => DispatchOutcome::Respond(encode_success(response, &[])),
                Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
            }
        }
        UsartOp::Write => match backend.write(payload) {
            Ok(_) => DispatchOutcome::Respond(encode_success(response, &[])),
            Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
        },
        UsartOp::Read => {
            let req_len = hdr.arg0_value() as usize;
            let payload_offset = UsartResponseHeader::SIZE;
            let payload_capacity = response.len().saturating_sub(payload_offset);
            let read_buf_len = core::cmp::min(req_len, payload_capacity);

            match backend.read(&mut response[payload_offset..payload_offset + read_buf_len]) {
                Ok(n) => {
                    let hdr = UsartResponseHeader::success(n as u16);
                    response[..UsartResponseHeader::SIZE]
                        .copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
                    DispatchOutcome::Respond(UsartResponseHeader::SIZE + n)
                }
                Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
            }
        }
        UsartOp::TryRead => {
            let req_len = hdr.arg0_value() as usize;
            let payload_offset = UsartResponseHeader::SIZE;
            let payload_capacity = response.len().saturating_sub(payload_offset);
            let read_buf_len = core::cmp::min(req_len, payload_capacity);

            // Attempt immediate non-blocking drain of the RX FIFO.
            match backend.try_read(&mut response[payload_offset..payload_offset + read_buf_len]) {
                Ok(n) => {
                    // Data available right now — respond immediately.
                    let hdr = UsartResponseHeader::success(n as u16);
                    response[..UsartResponseHeader::SIZE]
                        .copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
                    DispatchOutcome::Respond(UsartResponseHeader::SIZE + n)
                }
                Err(BackendError::WouldBlock) => {
                    // No data yet.  Attempt to park the request; fail with
                    // Busy if another TryRead is already pending — UART RX
                    // is a single stream and cannot serve two concurrent
                    // consumers.
                    if !pending.park_read(client_channel, req_len) {
                        return DispatchOutcome::Respond(encode_error(
                            response,
                            UsartError::Busy,
                        ));
                    }

                    // If we cannot arm RX interrupts, do not leave a parked
                    // request behind; fail this transaction immediately.
                    if let Err(e) = backend.enable_interrupts(IrqMask::RX_DATA_AVAILABLE) {
                        let _ = pending.take();
                        return DispatchOutcome::Respond(encode_error(response, e.into()));
                    }
                    DispatchOutcome::Queued
                }
                Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
            }
        }
        UsartOp::Drain => {
            if !pending.park_drain(client_channel) {
                return DispatchOutcome::Respond(encode_error(response, UsartError::Busy));
            }

            if let Err(e) = backend.enable_interrupts(IrqMask::TX_IDLE) {
                let _ = pending.take();
                return DispatchOutcome::Respond(encode_error(response, e.into()));
            }
            DispatchOutcome::Queued
        }
        UsartOp::GetLineStatus => match backend.line_status() {
            Ok(lsr) => DispatchOutcome::Respond(encode_success(response, &[lsr.0])),
            Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
        },
        UsartOp::EnableInterrupts => {
            let mask = IrqMask::from_bits_truncate(hdr.arg0_value());
            match backend.enable_interrupts(mask) {
                Ok(()) => DispatchOutcome::Respond(encode_success(response, &[])),
                Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
            }
        }
        UsartOp::DisableInterrupts => {
            let mask = IrqMask::from_bits_truncate(hdr.arg0_value());
            match backend.disable_interrupts(mask) {
                Ok(()) => DispatchOutcome::Respond(encode_success(response, &[])),
                Err(e) => DispatchOutcome::Respond(encode_error(response, e.into())),
            }
        }
    }
}

fn encode_error(response: &mut [u8], error: UsartError) -> usize {
    let hdr = UsartResponseHeader::error(error);
    response[..UsartResponseHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
    UsartResponseHeader::SIZE
}

fn encode_success(response: &mut [u8], payload: &[u8]) -> usize {
    let hdr = UsartResponseHeader::success(payload.len() as u16);
    response[..UsartResponseHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
    response[UsartResponseHeader::SIZE..UsartResponseHeader::SIZE + payload.len()]
        .copy_from_slice(payload);
    UsartResponseHeader::SIZE + payload.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use usart_api::backend::{LineStatus, Parity, UsartConfig};

    struct MockBackend {
        try_read_result: Result<usize, BackendError>,
        enable_irq_result: Result<(), BackendError>,
    }

    impl Default for MockBackend {
        fn default() -> Self {
            Self {
                try_read_result: Ok(0),
                enable_irq_result: Ok(()),
            }
        }
    }

    impl UsartBackend for MockBackend {
        fn configure(&mut self, _config: UsartConfig) -> Result<(), BackendError> {
            Ok(())
        }

        fn write(&mut self, data: &[u8]) -> Result<usize, BackendError> {
            Ok(data.len())
        }

        fn read(&mut self, _out: &mut [u8]) -> Result<usize, BackendError> {
            Err(BackendError::WouldBlock)
        }

        fn try_read(&mut self, _out: &mut [u8]) -> Result<usize, BackendError> {
            self.try_read_result
        }

        fn line_status(&self) -> Result<LineStatus, BackendError> {
            Ok(LineStatus(0))
        }

        fn enable_interrupts(&mut self, _mask: IrqMask) -> Result<(), BackendError> {
            self.enable_irq_result
        }

        fn disable_interrupts(&mut self, _mask: IrqMask) -> Result<(), BackendError> {
            Ok(())
        }
    }

    fn parse_status(resp: &[u8]) -> Option<UsartError> {
        zerocopy::Ref::<_, UsartResponseHeader>::from_bytes(&resp[..UsartResponseHeader::SIZE])
            .ok()
            .map(|hdr| hdr.error_code())
    }

    #[test]
    fn try_read_returns_busy_when_already_pending() {
        let mut backend = MockBackend {
            try_read_result: Err(BackendError::WouldBlock),
            enable_irq_result: Ok(()),
        };
        let mut pending = runtime::PendingIo::new();
        let _ = pending.park_read(99, 16);

        let hdr = UsartRequestHeader::new(UsartOp::TryRead, 16, 0, 0);
        let mut req = [0u8; UsartRequestHeader::SIZE];
        req.copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
        let mut resp = [0u8; MAX_RESPONSE_SIZE];

        let outcome = dispatch_request(&mut backend, &mut pending, 42, &req, &mut resp);
        assert!(matches!(outcome, DispatchOutcome::Respond(_)));
        if let DispatchOutcome::Respond(n) = outcome {
            assert_eq!(n, UsartResponseHeader::SIZE);
            assert_eq!(parse_status(&resp), Some(UsartError::Busy));
        }
    }

    #[test]
    fn try_read_enable_interrupt_failure_does_not_leave_pending() {
        let mut backend = MockBackend {
            try_read_result: Err(BackendError::WouldBlock),
            enable_irq_result: Err(BackendError::InternalError),
        };
        let mut pending = runtime::PendingIo::new();

        let hdr = UsartRequestHeader::new(UsartOp::TryRead, 16, 0, 0);
        let mut req = [0u8; UsartRequestHeader::SIZE];
        req.copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
        let mut resp = [0u8; MAX_RESPONSE_SIZE];

        let outcome = dispatch_request(&mut backend, &mut pending, 42, &req, &mut resp);
        assert!(matches!(outcome, DispatchOutcome::Respond(_)));
        if let DispatchOutcome::Respond(n) = outcome {
            assert_eq!(n, UsartResponseHeader::SIZE);
            assert_eq!(parse_status(&resp), Some(UsartError::InternalError));
            assert!(!pending.is_pending());
        }
    }

    #[test]
    fn configure_path_still_succeeds() {
        let mut backend = MockBackend::default();
        let mut pending = runtime::PendingIo::new();
        let _ = UsartConfig { baud_rate: 115200, parity: Parity::None, stop_bits: 1 };

        let cfg = UsartConfigurePayload::new(115200, UsartParityWire::None, 1);
        let hdr = UsartRequestHeader::new(
            UsartOp::Configure,
            0,
            0,
            UsartConfigurePayload::SIZE as u16,
        );
        let mut req = [0u8; UsartRequestHeader::SIZE + UsartConfigurePayload::SIZE];
        req[..UsartRequestHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
        req[UsartRequestHeader::SIZE..UsartRequestHeader::SIZE + UsartConfigurePayload::SIZE]
            .copy_from_slice(zerocopy::IntoBytes::as_bytes(&cfg));
        let mut resp = [0u8; MAX_RESPONSE_SIZE];

        let outcome = dispatch_request(&mut backend, &mut pending, 7, &req, &mut resp);
        assert!(matches!(outcome, DispatchOutcome::Respond(_)));
        if let DispatchOutcome::Respond(n) = outcome {
            assert_eq!(n, UsartResponseHeader::SIZE);
            assert_eq!(parse_status(&resp), Some(UsartError::Success));
        }
    }

    #[test]
    fn version_mismatch_is_rejected() {
        let mut backend = MockBackend::default();
        let mut pending = runtime::PendingIo::new();

        let mut hdr = UsartRequestHeader::new(UsartOp::Read, 1, 0, 0);
        hdr.flags = 1;
        let mut req = [0u8; UsartRequestHeader::SIZE];
        req.copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
        let mut resp = [0u8; MAX_RESPONSE_SIZE];

        let outcome = dispatch_request(&mut backend, &mut pending, 5, &req, &mut resp);
        assert!(matches!(outcome, DispatchOutcome::Respond(_)));
        if let DispatchOutcome::Respond(n) = outcome {
            assert_eq!(n, UsartResponseHeader::SIZE);
            assert_eq!(parse_status(&resp), Some(UsartError::UnsupportedVersion));
        }
    }

    #[test]
    fn drain_queues_request() {
        let mut backend = MockBackend::default();
        let mut pending = runtime::PendingIo::new();

        let hdr = UsartRequestHeader::new(UsartOp::Drain, 0, 0, 0);
        let mut req = [0u8; UsartRequestHeader::SIZE];
        req.copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
        let mut resp = [0u8; MAX_RESPONSE_SIZE];

        let outcome = dispatch_request(&mut backend, &mut pending, 7, &req, &mut resp);
        assert!(matches!(outcome, DispatchOutcome::Queued));
        assert!(pending.is_pending());
    }
}
