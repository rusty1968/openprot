// Licensed under the Apache-2.0 license

use usart_api::backend::{BackendError, LineStatus, UsartBackend, UsartConfig};
use usart_api::{UsartOp, UsartRequestHeader, UsartResponseHeader};

#[derive(Default)]
struct MockBackend;

impl UsartBackend for MockBackend {
    fn configure(&mut self, _config: UsartConfig) -> Result<(), BackendError> {
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> Result<usize, BackendError> {
        Ok(data.len())
    }

    fn read(&mut self, out: &mut [u8]) -> Result<usize, BackendError> {
        if out.is_empty() {
            return Ok(0);
        }
        out[0] = b'A';
        Ok(1)
    }

    fn line_status(&self) -> Result<LineStatus, BackendError> {
        Ok(LineStatus(0x20))
    }

    fn enable_interrupts(&mut self, _mask: u16) -> Result<(), BackendError> {
        Ok(())
    }

    fn disable_interrupts(&mut self, _mask: u16) -> Result<(), BackendError> {
        Ok(())
    }
}

#[test]
fn read_dispatch_smoke() {
    let mut backend = MockBackend;
    let mut req = [0u8; 32];
    let mut resp = [0u8; 32];

    let hdr = UsartRequestHeader::new(UsartOp::Read, 1, 0, 0);
    req[..UsartRequestHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));

    let n = usart_server::dispatch_request(
        &mut backend,
        &req[..UsartRequestHeader::SIZE],
        &mut resp,
    );

    assert!(n >= UsartResponseHeader::SIZE + 1);
    let hdr = zerocopy::Ref::<_, UsartResponseHeader>::from_bytes(
        &resp[..UsartResponseHeader::SIZE],
    )
    .unwrap();
    assert!(hdr.is_success());
    assert_eq!(resp[UsartResponseHeader::SIZE], b'A');
}
