// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Caliptra board policy for the OTP service: authorization, SVN fuse
//! encoding, and fuse geometry.

use hal_otp_driver::OtpOffset;
use otp_api::{FieldId, RegionId, REGION_SVN, REGION_VENDOR_HASHES_MANUF};
use otp_server::{AccessPolicy, CallerId, FieldMap, SvnCodec};

/// SoC anti-rollback SVN (thermometer-coded in the SVN partition).
pub const FIELD_SOC_MANIFEST_SVN: FieldId = FieldId(0x0003);
/// Vendor public-key hash in the manufacturing vendor-hashes partition.
pub const FIELD_VENDOR_PK_HASH_MANUF: FieldId = FieldId(0x0004);

/// Caller granted provisioning rights (raw program and floor advance). The
/// runtime assigns this id to the provisioning channel.
pub const PROVISIONER: CallerId = CallerId(1);

/// Caliptra fuse geometry: resolves a wire [`FieldId`] to `(region, offset,
/// len)`. Offsets and lengths mirror the Caliptra SS fuse map.
pub struct CaliptraFieldMap;

impl FieldMap for CaliptraFieldMap {
    fn locate(&self, field: FieldId) -> Option<(RegionId, OtpOffset, usize)> {
        Some(match field.0 {
            // soc_manifest_svn: 128-bit thermometer bitmap at SVN+20.
            0x0003 => (RegionId(REGION_SVN), OtpOffset::new(20), 16),
            // vendor pk hash (48 bytes) at the manuf vendor-hashes partition base.
            0x0004 => (RegionId(REGION_VENDOR_HASHES_MANUF), OtpOffset::new(0), 48),
            _ => return None,
        })
    }
}

/// Only the provisioning caller may program raw bytes or advance a floor; reads
/// are open to any caller (region-level read gating can tighten this later).
pub struct CaliptraAccessPolicy;

impl AccessPolicy for CaliptraAccessPolicy {
    fn can_read(&self, _caller: CallerId, _region: RegionId) -> bool {
        true
    }
    fn can_program(&self, caller: CallerId) -> bool {
        caller == PROVISIONER
    }
    fn can_commit_svn(&self, caller: CallerId) -> bool {
        caller == PROVISIONER
    }
}

/// SVN codec matching the Caliptra fuse representation: the stored value is a
/// 128-bit thermometer bitmap (lowest `svn` bits set, little-endian), and the
/// SVN magnitude is its population count. The wire candidate is a
/// little-endian `u32` SVN value.
pub struct CaliptraSvnCodec;

impl SvnCodec for CaliptraSvnCodec {
    fn is_monotonic_advance(&self, _field: FieldId, current: &[u8], candidate: &[u8]) -> bool {
        candidate_svn(candidate) > current_svn(current)
    }

    fn encode(&self, _field: FieldId, candidate: &[u8], out: &mut [u8]) -> usize {
        let bitmap = svn_to_bitmap(candidate_svn(candidate));
        let n = out.len().min(bitmap.len());
        out[..n].copy_from_slice(&bitmap[..n]);
        for slot in out.iter_mut().skip(n) {
            *slot = 0;
        }
        out.len()
    }
}

/// Decode the little-endian `u32` SVN value carried on the wire.
fn candidate_svn(bytes: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    let n = bytes.len().min(4);
    buf[..n].copy_from_slice(&bytes[..n]);
    u32::from_le_bytes(buf)
}

/// Recover the SVN magnitude from stored thermometer bytes.
fn current_svn(bitmap: &[u8]) -> u32 {
    bitmap.iter().map(|b| b.count_ones()).sum()
}

/// Thermometer-encode an SVN as a 128-bit little-endian bitmap with the lowest
/// `svn` bits set (saturating at 128). Matches the Caliptra fuse layout.
fn svn_to_bitmap(svn: u32) -> [u8; 16] {
    let n = svn.min(128);
    let val: u128 = if n == 0 {
        0
    } else if n == 128 {
        u128::MAX
    } else {
        (1u128 << n) - 1
    };
    val.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_map_resolves_known_fields() {
        let m = CaliptraFieldMap;
        assert_eq!(
            m.locate(FIELD_SOC_MANIFEST_SVN),
            Some((RegionId(REGION_SVN), OtpOffset::new(20), 16))
        );
        assert_eq!(
            m.locate(FIELD_VENDOR_PK_HASH_MANUF),
            Some((RegionId(REGION_VENDOR_HASHES_MANUF), OtpOffset::new(0), 48))
        );
        assert_eq!(m.locate(FieldId(0xABCD)), None);
    }

    #[test]
    fn svn_bitmap_matches_population_count() {
        assert_eq!(current_svn(&svn_to_bitmap(0)), 0);
        assert_eq!(current_svn(&svn_to_bitmap(5)), 5);
        assert_eq!(current_svn(&svn_to_bitmap(128)), 128);
        assert_eq!(current_svn(&svn_to_bitmap(200)), 128);
    }

    #[test]
    fn monotonic_advance_only_forward() {
        let c = CaliptraSvnCodec;
        let cur = svn_to_bitmap(3);
        assert!(c.is_monotonic_advance(FIELD_SOC_MANIFEST_SVN, &cur, &4u32.to_le_bytes()));
        assert!(!c.is_monotonic_advance(FIELD_SOC_MANIFEST_SVN, &cur, &3u32.to_le_bytes()));
        assert!(!c.is_monotonic_advance(FIELD_SOC_MANIFEST_SVN, &cur, &2u32.to_le_bytes()));
    }

    #[test]
    fn encode_produces_thermometer_bitmap() {
        let c = CaliptraSvnCodec;
        let mut out = [0u8; 16];
        let n = c.encode(FIELD_SOC_MANIFEST_SVN, &7u32.to_le_bytes(), &mut out);
        assert_eq!(n, 16);
        assert_eq!(out, svn_to_bitmap(7));
    }

    #[test]
    fn access_policy_gates_privileged_ops() {
        let p = CaliptraAccessPolicy;
        assert!(p.can_read(CallerId(99), RegionId(REGION_SVN)));
        assert!(p.can_program(PROVISIONER));
        assert!(!p.can_program(CallerId(99)));
        assert!(p.can_commit_svn(PROVISIONER));
        assert!(!p.can_commit_svn(CallerId(99)));
    }
}
