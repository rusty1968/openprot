// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Server side of the OTP userspace service.
//!
//! [`dispatch`] is a pure function (no `userspace`/IPC) generic over any HAL
//! OTP device, so it is host-testable with a mock device and a mock policy.
//! The Pigweed wait/respond loop belongs in a separate kernel-tagged
//! `otp-server-runtime` crate.
//!
//! The server is the single choke point for OTP policy the wire cannot carry:
//!
//! - **Authorization** — [`AccessPolicy`] decides which caller may read a
//!   region and who may program or advance the anti-rollback floor. Denials
//!   answer `NotAuthorized`; they never leak fuse contents.
//! - **Anti-rollback monotonicity** — `CommitSvnFloor` reads the current floor
//!   and only ever advances it.
//! - **Fuse encoding** — [`SvnCodec`] owns the fuse representation (one-hot,
//!   majority-vote, …) and the ordering used by the monotonic guard.
//! - **Fuse geometry** — [`FieldMap`] resolves a wire [`FieldId`] to this
//!   SoC's `(region, offset, len)`, so no fuse map is baked into the wire.
//!
//! Fail-closed: malformed, unsupported, denied, and hardware-error requests all
//! answer with an error header. The dispatch never panics.

#![no_std]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use hal_otp_driver::{Error, ErrorKind, OtpOffset, OtpProgramBytes, OtpReadBytes};
use otp_api::{
    FieldId, OtpOp, OtpRequestHeader, OtpResponseHeader, OtpWireError, RegionId, MAX_FIELD,
    MAX_PAYLOAD_SIZE,
};

/// One request/response buffer size.
pub const MAX_BUF_SIZE: usize = 512;

/// Identity of the caller, derived by the runtime from the channel a request
/// arrived on. Trust is anchored to the channel, not to anything on the wire.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CallerId(pub u32);

/// Board-supplied authorization, enforced on every request.
pub trait AccessPolicy {
    /// May `caller` read from `region`?
    fn can_read(&self, caller: CallerId, region: RegionId) -> bool;
    /// May `caller` program raw bytes?
    fn can_program(&self, caller: CallerId) -> bool;
    /// May `caller` advance an anti-rollback floor?
    fn can_commit_svn(&self, caller: CallerId) -> bool;
}

/// Board-supplied SVN semantics, kept server-side so the wire carries neither
/// the fuse encoding nor the value width. The candidate travels the wire as an
/// opaque byte payload whose meaning both the board and its clients agree on.
pub trait SvnCodec {
    /// Is `candidate` (the inline wire value) strictly newer than the value
    /// currently held in `current` (raw fuse bytes)? The board owns both the
    /// encoding and the ordering. A non-advance is an idempotent no-op.
    fn is_monotonic_advance(&self, field: FieldId, current: &[u8], candidate: &[u8]) -> bool;
    /// Encode `candidate` into `out` (the field's raw representation); returns
    /// the number of bytes written.
    fn encode(&self, field: FieldId, candidate: &[u8], out: &mut [u8]) -> usize;
}

/// Board-supplied fuse geometry. Resolves a wire [`FieldId`] to this SoC's
/// `(region, offset, len)`. Returns `None` for an id this board does not
/// carry, which the server answers as `Unsupported`.
pub trait FieldMap {
    fn locate(&self, field: FieldId) -> Option<(RegionId, OtpOffset, usize)>;
}

/// Map the HAL error taxonomy onto the wire status code.
fn wire_from_kind(kind: ErrorKind) -> OtpWireError {
    match kind {
        ErrorKind::InvalidAddress => OtpWireError::InvalidAddress,
        ErrorKind::AlignmentError => OtpWireError::Alignment,
        ErrorKind::RegionProtected => OtpWireError::RegionProtected,
        ErrorKind::Hardware => OtpWireError::Hardware,
        ErrorKind::Timeout => OtpWireError::Timeout,
        ErrorKind::Unsupported => OtpWireError::Unsupported,
        // `ErrorKind` is non-exhaustive; treat future kinds as internal.
        _ => OtpWireError::Internal,
    }
}

fn encode_error(response: &mut [u8], err: OtpWireError) -> usize {
    let hdr = OtpResponseHeader::error(err);
    response[..OtpResponseHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
    OtpResponseHeader::SIZE
}

/// Success header with `payload_len` bytes already placed at
/// `response[OtpResponseHeader::SIZE..]` by the caller (0 for ack-only).
fn encode_ok(response: &mut [u8], payload_len: usize) -> usize {
    let hdr = OtpResponseHeader::success(payload_len as u16);
    response[..OtpResponseHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&hdr));
    OtpResponseHeader::SIZE + payload_len
}

/// Decode one wire request, authorize it, execute it against `otp`, and encode
/// the response. Returns the number of bytes written (always `>=
/// OtpResponseHeader::SIZE`). Pure: no IPC, no globals. Never panics on
/// malformed input — it answers with an error header instead.
pub fn dispatch<D, P, C, M>(
    otp: &mut D,
    policy: &P,
    codec: &C,
    field_map: &M,
    caller: CallerId,
    request: &[u8],
    response: &mut [u8],
) -> usize
where
    D: OtpReadBytes<Region = RegionId> + OtpProgramBytes<Region = RegionId>,
    P: AccessPolicy,
    C: SvnCodec,
    M: FieldMap,
{
    if request.len() < OtpRequestHeader::SIZE {
        return encode_error(response, OtpWireError::Internal);
    }
    let Ok(hdr) =
        zerocopy::Ref::<_, OtpRequestHeader>::from_bytes(&request[..OtpRequestHeader::SIZE])
    else {
        return encode_error(response, OtpWireError::Internal);
    };

    match hdr.op() {
        Ok(OtpOp::ReadBytes) => {
            read_bytes(otp, policy, caller, hdr.region(), hdr.offset(), hdr.len(), response)
        }
        Ok(OtpOp::ReadField) => {
            let Some((region, offset, len)) = field_map.locate(hdr.field()) else {
                return encode_error(response, OtpWireError::Unsupported);
            };
            read_bytes(otp, policy, caller, region, offset, len, response)
        }
        Ok(OtpOp::ProgramBytes) => {
            program_bytes(otp, policy, caller, &hdr, request, response)
        }
        Ok(OtpOp::CommitSvnFloor) => {
            commit_svn(otp, policy, codec, field_map, caller, &hdr, request, response)
        }
        // `OtpOp` is non-exhaustive; reject any future opcode we do not handle.
        Ok(_) => encode_error(response, OtpWireError::Unsupported),
        Err(e) => encode_error(response, e),
    }
}

fn read_bytes<D, P>(
    otp: &D,
    policy: &P,
    caller: CallerId,
    region: RegionId,
    offset: hal_otp_driver::OtpOffset,
    len: usize,
    response: &mut [u8],
) -> usize
where
    D: OtpReadBytes<Region = RegionId>,
    P: AccessPolicy,
{
    if !policy.can_read(caller, region) {
        return encode_error(response, OtpWireError::NotAuthorized);
    }
    if len > MAX_PAYLOAD_SIZE || OtpResponseHeader::SIZE + len > response.len() {
        return encode_error(response, OtpWireError::InvalidAddress);
    }
    let out = &mut response[OtpResponseHeader::SIZE..OtpResponseHeader::SIZE + len];
    match otp.read_bytes(region, offset, out) {
        Ok(()) => encode_ok(response, len),
        Err(e) => encode_error(response, wire_from_kind(e.kind())),
    }
}

fn program_bytes<D, P>(
    otp: &mut D,
    policy: &P,
    caller: CallerId,
    hdr: &OtpRequestHeader,
    request: &[u8],
    response: &mut [u8],
) -> usize
where
    D: OtpProgramBytes<Region = RegionId>,
    P: AccessPolicy,
{
    if !policy.can_program(caller) {
        return encode_error(response, OtpWireError::NotAuthorized);
    }
    let len = hdr.len();
    if len > MAX_PAYLOAD_SIZE || request.len() < OtpRequestHeader::SIZE + len {
        return encode_error(response, OtpWireError::InvalidAddress);
    }
    let data = &request[OtpRequestHeader::SIZE..OtpRequestHeader::SIZE + len];
    match otp.program_bytes(hdr.region(), hdr.offset(), data) {
        Ok(()) => encode_ok(response, 0),
        Err(e) => encode_error(response, wire_from_kind(e.kind())),
    }
}

fn commit_svn<D, P, C, M>(
    otp: &mut D,
    policy: &P,
    codec: &C,
    field_map: &M,
    caller: CallerId,
    hdr: &OtpRequestHeader,
    request: &[u8],
    response: &mut [u8],
) -> usize
where
    D: OtpReadBytes<Region = RegionId> + OtpProgramBytes<Region = RegionId>,
    P: AccessPolicy,
    C: SvnCodec,
    M: FieldMap,
{
    if !policy.can_commit_svn(caller) {
        return encode_error(response, OtpWireError::NotAuthorized);
    }
    let field = hdr.field();
    let Some((region, offset, len)) = field_map.locate(field) else {
        return encode_error(response, OtpWireError::Unsupported);
    };
    if len > MAX_FIELD {
        return encode_error(response, OtpWireError::Internal);
    }
    let cand_len = hdr.len();
    if cand_len > MAX_PAYLOAD_SIZE || request.len() < OtpRequestHeader::SIZE + cand_len {
        return encode_error(response, OtpWireError::InvalidAddress);
    }
    let candidate = &request[OtpRequestHeader::SIZE..OtpRequestHeader::SIZE + cand_len];

    let mut cur = [0u8; MAX_FIELD];
    if let Err(e) = otp.read_bytes(region, offset, &mut cur[..len]) {
        return encode_error(response, wire_from_kind(e.kind()));
    }
    // Monotonic guard: never regress the floor. An already-satisfied request is
    // an idempotent no-op, not an error.
    if !codec.is_monotonic_advance(field, &cur[..len], candidate) {
        return encode_ok(response, 0);
    }

    let mut enc = [0u8; MAX_FIELD];
    let n = codec.encode(field, candidate, &mut enc[..len]);
    if n > len {
        return encode_error(response, OtpWireError::Internal);
    }
    match otp.program_bytes(region, offset, &enc[..n]) {
        Ok(()) => encode_ok(response, 0),
        Err(e) => encode_error(response, wire_from_kind(e.kind())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hal_otp_driver::{ErrorType, OtpOffset};
    use otp_api::{REGION_SVN, REGION_VENDOR_HASHES_MANUF};
    use zerocopy::FromBytes;

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    struct MockError;
    impl hal_otp_driver::Error for MockError {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Hardware
        }
    }

    /// One fuse partition of raw bytes; RegionId(0) is the SVN partition.
    struct MockOtp {
        svn: [u8; 64],
    }

    impl MockOtp {
        fn new() -> Self {
            Self { svn: [0u8; 64] }
        }
    }

    impl ErrorType for MockOtp {
        type Error = MockError;
    }

    impl OtpReadBytes for MockOtp {
        type Region = RegionId;
        fn read_bytes(
            &self,
            region: RegionId,
            offset: OtpOffset,
            buf: &mut [u8],
        ) -> Result<(), MockError> {
            if region.0 != REGION_SVN {
                return Err(MockError);
            }
            let start = offset.bytes();
            let end = start.checked_add(buf.len()).ok_or(MockError)?;
            let src = self.svn.get(start..end).ok_or(MockError)?;
            buf.copy_from_slice(src);
            Ok(())
        }
    }

    impl OtpProgramBytes for MockOtp {
        fn program_bytes(
            &mut self,
            region: RegionId,
            offset: OtpOffset,
            data: &[u8],
        ) -> Result<(), MockError> {
            if region.0 != REGION_SVN {
                return Err(MockError);
            }
            let start = offset.bytes();
            let end = start.checked_add(data.len()).ok_or(MockError)?;
            let dst = self.svn.get_mut(start..end).ok_or(MockError)?;
            dst.copy_from_slice(data);
            Ok(())
        }
    }

    /// Little-endian u32 in the first 4 bytes of the field; the wire candidate
    /// is the same little-endian value.
    struct LeCodec;
    impl SvnCodec for LeCodec {
        fn is_monotonic_advance(&self, _field: FieldId, current: &[u8], candidate: &[u8]) -> bool {
            le_u32(candidate) > le_u32(current)
        }
        fn encode(&self, _field: FieldId, candidate: &[u8], out: &mut [u8]) -> usize {
            let bytes = le_u32(candidate).to_le_bytes();
            let n = out.len().min(4);
            out[..n].copy_from_slice(&bytes[..n]);
            for slot in out.iter_mut().skip(4) {
                *slot = 0;
            }
            out.len()
        }
    }

    fn le_u32(b: &[u8]) -> u32 {
        let mut buf = [0u8; 4];
        let n = b.len().min(4);
        buf[..n].copy_from_slice(&b[..n]);
        u32::from_le_bytes(buf)
    }

    /// Reads allowed on the SVN region for everyone; program/commit only for
    /// the provisioning caller (id 1).
    struct Policy;
    const PROVISIONER: CallerId = CallerId(1);
    const APP: CallerId = CallerId(2);
    impl AccessPolicy for Policy {
        fn can_read(&self, _caller: CallerId, region: RegionId) -> bool {
            region.0 == REGION_SVN
        }
        fn can_program(&self, caller: CallerId) -> bool {
            caller == PROVISIONER
        }
        fn can_commit_svn(&self, caller: CallerId) -> bool {
            caller == PROVISIONER
        }
    }

    // Board-private field id assignment; the wire crate ascribes no meaning.
    const RUNTIME_SVN: FieldId = FieldId(0x0002);
    const VENDOR_PK_HASH0: FieldId = FieldId(0x0004);

    /// Test fuse map mirroring the Caliptra layout.
    struct Fields;
    impl FieldMap for Fields {
        fn locate(&self, field: FieldId) -> Option<(RegionId, OtpOffset, usize)> {
            Some(match field {
                FieldId(0x0001) => (RegionId(REGION_SVN), OtpOffset::new(0), 4),
                FieldId(0x0002) => (RegionId(REGION_SVN), OtpOffset::new(4), 16),
                FieldId(0x0003) => (RegionId(REGION_SVN), OtpOffset::new(20), 16),
                FieldId(0x0004) => (RegionId(REGION_VENDOR_HASHES_MANUF), OtpOffset::new(0), 48),
                _ => return None,
            })
        }
    }

    fn read_field_req(field: FieldId) -> [u8; OtpRequestHeader::SIZE] {
        let h = OtpRequestHeader::new(OtpOp::ReadField, REGION_SVN, field.0, 0, 0);
        let mut buf = [0u8; OtpRequestHeader::SIZE];
        buf.copy_from_slice(zerocopy::IntoBytes::as_bytes(&h));
        buf
    }

    /// Build a `CommitSvnFloor` request with an inline `value` payload into
    /// `out`; returns the total request length.
    fn commit_req(field: FieldId, value: &[u8], out: &mut [u8]) -> usize {
        let h = OtpRequestHeader::new(
            OtpOp::CommitSvnFloor,
            REGION_SVN,
            field.0,
            value.len() as u16,
            0,
        );
        out[..OtpRequestHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&h));
        out[OtpRequestHeader::SIZE..OtpRequestHeader::SIZE + value.len()].copy_from_slice(value);
        OtpRequestHeader::SIZE + value.len()
    }

    #[test]
    fn read_field_returns_bytes() {
        let mut otp = MockOtp::new();
        otp.svn[4..8].copy_from_slice(&5u32.to_le_bytes()); // RuntimeSvn field
        let mut resp = [0u8; MAX_BUF_SIZE];
        let req = read_field_req(RUNTIME_SVN);
        let n = dispatch(&mut otp, &Policy, &LeCodec, &Fields, APP, &req, &mut resp);

        let hdr = OtpResponseHeader::ref_from_bytes(&resp[..OtpResponseHeader::SIZE]).unwrap();
        assert!(hdr.is_success());
        assert_eq!(hdr.payload_length(), 16);
        let mut val = [0u8; 4];
        val.copy_from_slice(&resp[OtpResponseHeader::SIZE..OtpResponseHeader::SIZE + 4]);
        assert_eq!(u32::from_le_bytes(val), 5);
        assert!(n > OtpResponseHeader::SIZE);
    }

    #[test]
    fn read_denied_region_is_not_authorized() {
        let mut otp = MockOtp::new();
        let mut resp = [0u8; MAX_BUF_SIZE];
        // VendorPkHash0 lives in a region the policy does not allow.
        let req = read_field_req(VENDOR_PK_HASH0);
        dispatch(&mut otp, &Policy, &LeCodec, &Fields, APP, &req, &mut resp);

        let hdr = OtpResponseHeader::ref_from_bytes(&resp[..OtpResponseHeader::SIZE]).unwrap();
        assert_eq!(hdr.error_code(), Some(OtpWireError::NotAuthorized));
        let _ = (REGION_VENDOR_HASHES_MANUF, MockError.kind());
    }

    #[test]
    fn commit_svn_advances_then_refuses_regress() {
        let mut otp = MockOtp::new();
        let mut resp = [0u8; MAX_BUF_SIZE];
        let mut req = [0u8; MAX_BUF_SIZE];

        // Advance floor to 7.
        let n = commit_req(RUNTIME_SVN, &7u32.to_le_bytes(), &mut req);
        dispatch(&mut otp, &Policy, &LeCodec, &Fields, PROVISIONER, &req[..n], &mut resp);
        let hdr = OtpResponseHeader::ref_from_bytes(&resp[..OtpResponseHeader::SIZE]).unwrap();
        assert!(hdr.is_success());
        assert_eq!(u32::from_le_bytes(otp.svn[4..8].try_into().unwrap()), 7);

        // A lower value is an idempotent no-op; the floor stays at 7.
        let n = commit_req(RUNTIME_SVN, &3u32.to_le_bytes(), &mut req);
        dispatch(&mut otp, &Policy, &LeCodec, &Fields, PROVISIONER, &req[..n], &mut resp);
        let hdr = OtpResponseHeader::ref_from_bytes(&resp[..OtpResponseHeader::SIZE]).unwrap();
        assert!(hdr.is_success());
        assert_eq!(u32::from_le_bytes(otp.svn[4..8].try_into().unwrap()), 7);
    }

    #[test]
    fn commit_svn_requires_authorization() {
        let mut otp = MockOtp::new();
        let mut resp = [0u8; MAX_BUF_SIZE];
        let mut req = [0u8; MAX_BUF_SIZE];
        let n = commit_req(RUNTIME_SVN, &9u32.to_le_bytes(), &mut req);
        dispatch(&mut otp, &Policy, &LeCodec, &Fields, APP, &req[..n], &mut resp);

        let hdr = OtpResponseHeader::ref_from_bytes(&resp[..OtpResponseHeader::SIZE]).unwrap();
        assert_eq!(hdr.error_code(), Some(OtpWireError::NotAuthorized));
        assert_eq!(u32::from_le_bytes(otp.svn[4..8].try_into().unwrap()), 0);
    }

    #[test]
    fn malformed_request_answers_error() {
        let mut otp = MockOtp::new();
        let mut resp = [0u8; MAX_BUF_SIZE];
        let n = dispatch(&mut otp, &Policy, &LeCodec, &Fields, APP, &[0u8; 3], &mut resp);
        let hdr = OtpResponseHeader::ref_from_bytes(&resp[..OtpResponseHeader::SIZE]).unwrap();
        assert!(!hdr.is_success());
        assert_eq!(n, OtpResponseHeader::SIZE);
    }
}
