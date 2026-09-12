// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Emulator smoke test for the VeeR OTP controller driver.
//!
//! Runs on the Caliptra MCU emulator: constructs an [`OtpController`] over the
//! OTP controller's MMIO base and exercises the DAI read path both directly and
//! through the blocking OTP HAL trait, then reports PASS/FAIL on the console and
//! exits.

#![no_std]
#![no_main]

use caliptra_ss_registers::fuses;
use caliptra_ss_registers::otp_ctrl::OTP_CTRL_ADDR;
use entry::exit;
use hal_otp_driver::{OtpRead, OtpProgramBytes, OtpReadBytes};
use otp_api::{OtpOp, OtpRequestHeader, OtpResponseHeader, OtpWireError, REGION_SVN};
use otp_backend::{
    CaliptraAccessPolicy, CaliptraFieldMap, CaliptraOtpBackend, CaliptraSvnCodec,
    FIELD_SOC_MANIFEST_SVN, PROVISIONER,
};
use otp_server::{dispatch, CallerId};
use target_common::{declare_target, TargetInterface};
use veer_peripherals::otp::{OtpController, OtpOffset, Partition};
use zerocopy::{FromBytes, IntoBytes};
use {codegen as _, console_backend as _};

/// Did the service answer with a success header?
fn resp_success(resp: &[u8]) -> bool {
    OtpResponseHeader::ref_from_bytes(&resp[..OtpResponseHeader::SIZE])
        .map(|h| h.is_success())
        .unwrap_or(false)
}

/// The error code the service answered with, if any.
fn resp_error(resp: &[u8]) -> Option<OtpWireError> {
    OtpResponseHeader::ref_from_bytes(&resp[..OtpResponseHeader::SIZE])
        .ok()
        .and_then(|h| h.error_code())
}

/// Population count of a byte slice — the SVN magnitude of a thermometer field.
fn popcount(bytes: &[u8]) -> u32 {
    bytes.iter().map(|b| b.count_ones()).sum()
}

/// Advance the anti-rollback floor to `svn` under `caller`, returning the
/// service response length.
fn commit_svn(
    backend: &mut CaliptraOtpBackend,
    policy: &CaliptraAccessPolicy,
    codec: &CaliptraSvnCodec,
    field_map: &CaliptraFieldMap,
    caller: CallerId,
    svn: u32,
    resp: &mut [u8],
) -> usize {
    let hdr = OtpRequestHeader::new(OtpOp::CommitSvnFloor, 0, FIELD_SOC_MANIFEST_SVN.0, 4, 0);
    let mut req = [0u8; OtpRequestHeader::SIZE + 4];
    req[..OtpRequestHeader::SIZE].copy_from_slice(hdr.as_bytes());
    req[OtpRequestHeader::SIZE..].copy_from_slice(&svn.to_le_bytes());
    dispatch(backend, policy, codec, field_map, caller, &req, resp)
}

/// Read the anti-rollback floor field and return its decoded SVN magnitude.
fn read_floor(
    backend: &mut CaliptraOtpBackend,
    policy: &CaliptraAccessPolicy,
    codec: &CaliptraSvnCodec,
    field_map: &CaliptraFieldMap,
    resp: &mut [u8],
) -> u32 {
    let hdr = OtpRequestHeader::new(OtpOp::ReadField, 0, FIELD_SOC_MANIFEST_SVN.0, 0, 0);
    let mut req = [0u8; OtpRequestHeader::SIZE];
    req.copy_from_slice(hdr.as_bytes());
    let n = dispatch(backend, policy, codec, field_map, CallerId(0), &req, resp);
    popcount(&resp[OtpResponseHeader::SIZE..n])
}

pub struct Target {}

impl TargetInterface for Target {
    const NAME: &'static str = "Caliptra MCU OTP Emulator Test";

    fn main() -> ! {
        // riscv does not run ctors automatically; needed for console/log init.
        unsafe { target_common::run_ctors() };

        // SAFETY: `OTP_CTRL_ADDR` is the OTP controller MMIO base in the
        // Caliptra Subsystem address map, and the kernel has exclusive access
        // to it here.
        let otp = unsafe { OtpController::from_addr(OTP_CTRL_ADDR as usize) };

        // Direct DAI read of the first fuse word.
        let raw = match otp.read_word(0) {
            Ok(v) => v,
            Err(_) => {
                pw_log::info!("[otp-test] DAI read_word(0) failed");
                pw_log::info!("FAIL: 1");
                exit(1);
            }
        };
        pw_log::info!("[otp-test] DAI read_word(0) = {}", raw as u32);

        // The same access through the blocking OTP HAL trait must agree.
        let region = Partition::new(0, 64);
        match OtpRead::read(&otp, region, OtpOffset::new(0)) {
            Ok(v) if v == raw => pw_log::info!("[otp-test] HAL read matches raw read"),
            Ok(_) => {
                pw_log::info!("[otp-test] HAL read mismatch");
                pw_log::info!("FAIL: 1");
                exit(1);
            }
            Err(_) => {
                pw_log::info!("[otp-test] HAL read failed");
                pw_log::info!("FAIL: 1");
                exit(1);
            }
        }

        // End-to-end: the OTP service dispatch over the Caliptra backend must
        // return the same bytes as a direct DAI read of the SVN partition.
        let svn_word = match otp.read_word(fuses::SVN_PARTITION_BYTE_OFFSET) {
            Ok(v) => v,
            Err(_) => {
                pw_log::info!("[otp-test] SVN partition read failed");
                pw_log::info!("FAIL: 1");
                exit(1);
            }
        };

        // SAFETY: same MMIO base as above; single-threaded, exclusive access.
        let mut backend =
            CaliptraOtpBackend::new(unsafe { OtpController::from_addr(OTP_CTRL_ADDR as usize) });
        let policy = CaliptraAccessPolicy;
        let codec = CaliptraSvnCodec;
        let field_map = CaliptraFieldMap;

        let hdr = OtpRequestHeader::new(OtpOp::ReadBytes, REGION_SVN, 0, 4, 0);
        let mut request = [0u8; OtpRequestHeader::SIZE];
        request.copy_from_slice(hdr.as_bytes());
        let mut response = [0u8; 64];
        let n = dispatch(
            &mut backend,
            &policy,
            &codec,
            &field_map,
            CallerId(0),
            &request,
            &mut response,
        );

        let resp_hdr = match OtpResponseHeader::ref_from_bytes(&response[..OtpResponseHeader::SIZE]) {
            Ok(h) => h,
            Err(_) => {
                pw_log::info!("[otp-test] response header decode failed");
                pw_log::info!("FAIL: 1");
                exit(1);
            }
        };
        if !resp_hdr.is_success()
            || resp_hdr.payload_length() != 4
            || n != OtpResponseHeader::SIZE + 4
        {
            pw_log::info!("[otp-test] service ReadBytes returned error");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        let payload = &response[OtpResponseHeader::SIZE..OtpResponseHeader::SIZE + 4];
        if payload != svn_word.to_le_bytes() {
            pw_log::info!("[otp-test] service payload does not match direct read");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        pw_log::info!("[otp-test] service ReadBytes matches direct DAI read");

        // Program raw bytes as the provisioner, then read them back.
        let value = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let phdr = OtpRequestHeader::new(OtpOp::ProgramBytes, REGION_SVN, 0, value.len() as u16, 4);
        let mut preq = [0u8; OtpRequestHeader::SIZE + 4];
        preq[..OtpRequestHeader::SIZE].copy_from_slice(phdr.as_bytes());
        preq[OtpRequestHeader::SIZE..].copy_from_slice(&value);
        let mut presp = [0u8; 64];
        dispatch(&mut backend, &policy, &codec, &field_map, PROVISIONER, &preq, &mut presp);
        if !resp_success(&presp) {
            pw_log::info!("[otp-test] ProgramBytes (authorized) rejected");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        let rbhdr = OtpRequestHeader::new(OtpOp::ReadBytes, REGION_SVN, 0, 4, 4);
        let mut rbreq = [0u8; OtpRequestHeader::SIZE];
        rbreq.copy_from_slice(rbhdr.as_bytes());
        let mut rbresp = [0u8; 64];
        dispatch(&mut backend, &policy, &codec, &field_map, CallerId(0), &rbreq, &mut rbresp);
        if !resp_success(&rbresp)
            || rbresp[OtpResponseHeader::SIZE..OtpResponseHeader::SIZE + 4] != value
        {
            pw_log::info!("[otp-test] ProgramBytes read-back mismatch");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        pw_log::info!("[otp-test] ProgramBytes writes and reads back");

        // A non-provisioner may not program raw bytes.
        let uhdr = OtpRequestHeader::new(OtpOp::ProgramBytes, REGION_SVN, 0, value.len() as u16, 8);
        let mut ureq = [0u8; OtpRequestHeader::SIZE + 4];
        ureq[..OtpRequestHeader::SIZE].copy_from_slice(uhdr.as_bytes());
        ureq[OtpRequestHeader::SIZE..].copy_from_slice(&value);
        let mut uresp = [0u8; 64];
        dispatch(&mut backend, &policy, &codec, &field_map, CallerId(0), &ureq, &mut uresp);
        if resp_error(&uresp) != Some(OtpWireError::NotAuthorized) {
            pw_log::info!("[otp-test] unauthorized ProgramBytes not denied");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        pw_log::info!("[otp-test] unauthorized ProgramBytes denied");

        // Anti-rollback floor: advance, reject a regression, advance again.
        let mut fresp = [0u8; 64];
        commit_svn(&mut backend, &policy, &codec, &field_map, PROVISIONER, 3, &mut fresp);
        if !resp_success(&fresp)
            || read_floor(&mut backend, &policy, &codec, &field_map, &mut fresp) != 3
        {
            pw_log::info!("[otp-test] CommitSvnFloor initial advance failed");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        // A lower value is an idempotent no-op; the floor must not regress.
        commit_svn(&mut backend, &policy, &codec, &field_map, PROVISIONER, 2, &mut fresp);
        if !resp_success(&fresp)
            || read_floor(&mut backend, &policy, &codec, &field_map, &mut fresp) != 3
        {
            pw_log::info!("[otp-test] CommitSvnFloor regressed the floor");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        // A higher value advances it.
        let mut raw = [0u8; 16];
        let _ = backend.read_bytes(otp_api::RegionId(REGION_SVN), OtpOffset::new(20), &mut raw);
        let is_adv =
            otp_server::SvnCodec::is_monotonic_advance(&codec, FIELD_SOC_MANIFEST_SVN, &raw, &5u32.to_le_bytes());
        pw_log::info!("[otp-test] pre-advance5 field[0]={} is_adv={}", raw[0] as u32, is_adv as u32);
        // Probe raw OR-rewrite semantics on a blank word (offset 8).
        let _ = backend.program_bytes(otp_api::RegionId(REGION_SVN), OtpOffset::new(8), &[0x07, 0, 0, 0]);
        let _ = backend.program_bytes(otp_api::RegionId(REGION_SVN), OtpOffset::new(8), &[0x1F, 0, 0, 0]);
        let mut probe = [0u8; 4];
        let _ = backend.read_bytes(otp_api::RegionId(REGION_SVN), OtpOffset::new(8), &mut probe);
        pw_log::info!("[otp-test] rewrite probe offset8[0]={}", probe[0] as u32);
        commit_svn(&mut backend, &policy, &codec, &field_map, PROVISIONER, 5, &mut fresp);
        let adv_ok = resp_success(&fresp);
        let floor = read_floor(&mut backend, &policy, &codec, &field_map, &mut fresp);
        pw_log::info!("[otp-test] advance5: ok={} floor={}", adv_ok as u32, floor as u32);
        if !adv_ok || floor != 5 {
            pw_log::info!("[otp-test] CommitSvnFloor second advance failed");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        // A non-provisioner may not advance the floor.
        commit_svn(&mut backend, &policy, &codec, &field_map, CallerId(0), 9, &mut fresp);
        if resp_error(&fresp) != Some(OtpWireError::NotAuthorized) {
            pw_log::info!("[otp-test] unauthorized CommitSvnFloor not denied");
            pw_log::info!("FAIL: 1");
            exit(1);
        }
        pw_log::info!("[otp-test] CommitSvnFloor advances monotonically");

        pw_log::info!("PASS");
        exit(0);
    }
}

declare_target!(Target);
