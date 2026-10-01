// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! PLDM Firmware Device (FD) app.
//!
//! Runs the real `openprot-pldm-service` `FirmwareDevice::run_terminus` state
//! machine over MCTP-over-I3C: two `MctpPldmTransport`s (responder + requester)
//! backed by `IpcMctpClient`s to the `mctp_server`, which carries the traffic
//! over the `i3c_server`. The host plays the Update Agent.
//!
//! `MockFdOps` is a bounded-slice implementation per the callback-shape
//! evaluation: `download_fw_data` takes one chunk per call and `verify`/`apply`
//! report done immediately. Unlike the pldm_i3c correctness test, `get_xfer_size`
//! here allows a chunk larger than one MCTP fragment: each `RequestFirmwareData`
//! response spans a few inbound fragments, which the i3c server's inbound ring
//! queues and the FD's MCTP stack reassembles — fewer, bigger download
//! round-trips (the throughput lever).

#![no_main]
#![no_std]

use core::cell::{Cell, RefCell};

use openprot_mctp_client_ipc::IpcMctpClient;
use openprot_pldm_service::firmware_device::{FirmwareDevice, RunTerminusResult, FD_MAX_MSG};
use openprot_pldm_service::{MctpPldmTransport, PldmServiceError};
use pldm_common::message::firmware_update::apply_complete::ApplyResult;
use pldm_common::message::firmware_update::get_fw_params::FirmwareParameters;
use pldm_common::message::firmware_update::get_status::ProgressPercent;
use pldm_common::message::firmware_update::transfer_complete::TransferResult;
use pldm_common::message::firmware_update::verify_complete::VerifyResult;
use pldm_common::protocol::firmware_update::{ComponentResponseCode, Descriptor};
use pldm_common::util::fw_component::FirmwareComponent;
use pldm_fd_codegen::handle;
use pldm_interface::config::PLDM_PROTOCOL_CAPABILITIES;
use pldm_interface::firmware_device::fd_ops::{ComponentOperation, FdOps, FdOpsError};
use userspace::process_entry;
use userspace::syscall;

/// This FD's own MCTP EID; must match the mctp_server's `OWN_EID` underneath it.
const FD_EID: u8 = 8;
/// Update Agent EID this FD serves (the host).
const UA_EID: u8 = 0x0a;
/// Larger than the pldm_i3c correctness image (512) so the download phase is
/// long enough for a stable throughput number, matching how caliptra-mcu-sw
/// measures its PLDM transfer speed.
const IMAGE_SIZE: usize = 4096;
/// Per-chunk transfer cap, well past the single-fragment MTU (241 B): each
/// `RequestFirmwareData` response spans ~5 inbound MCTP fragments, which the
/// inbound ring (`RX_RING = 6`) queues and the FD's MCTP stack reassembles into
/// a ~965 B message (within the 1023 B MCTP ceiling) — fewer, bigger download
/// round-trips.
///
/// This needs two things raised together, which this branch does: the
/// `pldm-common` `MAX_TRANSFER_SIZE` cap (512 → 960, via the MODULE.bazel
/// crate patch — it was a hardcoded placeholder; caliptra-mcu-sw derives the
/// same ~1019 B from the MCTP message size) and the i3c server's `RX_RING`
/// (4 → 6, so the 5-fragment burst fits).
const FD_XFER_CAP: usize = 960;
/// How long the responder waits for a UA command while idle. Large, like the
/// AST10x0 reference FD: `run_terminus` is called once and blocks here between
/// commands, rather than being re-entered in a tight poll loop (that churn of
/// timed-out recvs is what crashed the earlier version).
const IDLE_TIMEOUT_MILLIS: u32 = 15_000;
/// Bound each FD-initiated request's wait for the UA response.
const REQUESTER_TIMEOUT_MILLIS: u32 = 5_000;

struct MockFdOps {
    component_accepted: Cell<bool>,
    download_bytes_received: Cell<usize>,
    downloaded_image: RefCell<[u8; IMAGE_SIZE]>,
    verified: Cell<bool>,
    applied: Cell<bool>,
}

impl MockFdOps {
    const fn new() -> Self {
        Self {
            component_accepted: Cell::new(false),
            download_bytes_received: Cell::new(0),
            downloaded_image: RefCell::new([0u8; IMAGE_SIZE]),
            verified: Cell::new(false),
            applied: Cell::new(false),
        }
    }
}

impl FdOps for MockFdOps {
    fn get_device_identifiers(&self, _ids: &mut [Descriptor]) -> Result<usize, FdOpsError> {
        Ok(0)
    }

    fn get_firmware_parms(&self, params: &mut FirmwareParameters) -> Result<(), FdOpsError> {
        *params = FirmwareParameters::default();
        Ok(())
    }

    fn get_xfer_size(&self, ua_transfer_size: usize) -> Result<usize, FdOpsError> {
        Ok(ua_transfer_size.min(FD_XFER_CAP))
    }

    fn handle_component(
        &self,
        _component: &FirmwareComponent,
        _fw_params: &FirmwareParameters,
        _op: ComponentOperation,
    ) -> Result<ComponentResponseCode, FdOpsError> {
        self.component_accepted.set(true);
        Ok(ComponentResponseCode::CompCanBeUpdated)
    }

    fn query_download_offset_and_length(
        &self,
        _component: &FirmwareComponent,
    ) -> Result<(usize, usize), FdOpsError> {
        let offset = self.download_bytes_received.get();
        let length = IMAGE_SIZE
            .checked_sub(offset)
            .ok_or(FdOpsError::FwDownloadError)?
            .min(FD_XFER_CAP);
        Ok((offset, length))
    }

    fn download_fw_data(
        &self,
        offset: usize,
        data: &[u8],
        _component: &FirmwareComponent,
    ) -> Result<TransferResult, FdOpsError> {
        let end = offset
            .checked_add(data.len())
            .ok_or(FdOpsError::FwDownloadError)?;
        let mut image = self.downloaded_image.borrow_mut();
        image
            .get_mut(offset..end)
            .ok_or(FdOpsError::FwDownloadError)?
            .copy_from_slice(data);
        self.download_bytes_received
            .set(end.max(self.download_bytes_received.get()));
        Ok(TransferResult::TransferSuccess)
    }

    fn is_download_complete(&self, _component: &FirmwareComponent) -> bool {
        self.download_bytes_received.get() >= IMAGE_SIZE
    }

    fn query_download_progress(
        &self,
        _component: &FirmwareComponent,
        progress_percent: &mut ProgressPercent,
    ) -> Result<(), FdOpsError> {
        let pct = (self.download_bytes_received.get() * 100 / IMAGE_SIZE) as u8;
        progress_percent
            .set_value(pct.min(100))
            .map_err(|_| FdOpsError::FwDownloadError)?;
        Ok(())
    }

    fn verify(
        &self,
        _component: &FirmwareComponent,
        _progress_percent: &mut ProgressPercent,
    ) -> Result<VerifyResult, FdOpsError> {
        // Leave progress at default (treated as done) so VerifyComplete is
        // issued immediately — a bounded single-slice verify.
        self.verified.set(true);
        Ok(VerifyResult::VerifySuccess)
    }

    fn apply(
        &self,
        _component: &FirmwareComponent,
        _progress_percent: &mut ProgressPercent,
    ) -> Result<ApplyResult, FdOpsError> {
        self.applied.set(true);
        Ok(ApplyResult::ApplySuccess)
    }

    fn activate(&self, _self_contained: u8, _estimated_time: &mut u16) -> Result<u8, FdOpsError> {
        Ok(0)
    }

    fn cancel_update_component(&self, _component: &FirmwareComponent) -> Result<(), FdOpsError> {
        Ok(())
    }
}

#[process_entry("pldm_fd")]
fn entry() {
    // Earliest possible log: if this appears but "waiting for update agent" does
    // not, the crash is in FirmwareDevice/transport construction below (likely
    // stack). If this never appears, the crash is before entry() runs.
    pw_log::info!("pldm fd: starting");

    let fd_ops = MockFdOps::new();
    let responder_transport = MctpPldmTransport::new(IpcMctpClient::new(handle::MCTP));
    let requester_transport = MctpPldmTransport::new(IpcMctpClient::new(handle::MCTP));

    // Set this FD's MCTP EID before init, as the AST10x0 reference FD does.
    if responder_transport.stack().set_eid(FD_EID).is_err() {
        pw_log::error!("pldm fd: set_eid failed");
        let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
        #[expect(clippy::empty_loop)]
        loop {}
    }

    let mut fd = FirmwareDevice::init(
        &fd_ops,
        &PLDM_PROTOCOL_CAPABILITIES,
        responder_transport,
        requester_transport,
    );

    pw_log::info!("pldm fd: waiting for update agent");

    let mut buf = [0u8; FD_MAX_MSG];

    // Drive the whole update in a single `run_terminus`, exactly like the
    // AST10x0 reference FD (target/ast10x0/tests/pldm/firmware_update/fd_main.rs):
    // it services UA->FD commands and autonomously issues the FD-initiated
    // download/verify/apply requests from this one thread, blocking on the
    // responder for up to `IDLE_TIMEOUT_MILLIS` between commands. It returns
    // `Completed` when the device goes back to Idle (after ActivateFirmware) or
    // a `Mctp` timeout when the UA stops talking. Phase B's host UA stops after
    // ApplyComplete without sending ActivateFirmware, so the terminal idle
    // timeout is expected; success here is "the image was applied".
    match fd.run_terminus(
        UA_EID,
        &mut buf,
        IDLE_TIMEOUT_MILLIS,
        REQUESTER_TIMEOUT_MILLIS,
        &mut (),
    ) {
        RunTerminusResult::Completed => {
            pw_log::info!("pldm fd: run_terminus completed");
        }
        RunTerminusResult::StoppedByError(PldmServiceError::Mctp(e)) if e.is_timeout() => {
            pw_log::info!("pldm fd: idle timeout (no further UA command)");
        }
        RunTerminusResult::StoppedByError(_) => {
            pw_log::error!("pldm fd: run_terminus stopped on error");
        }
    }

    if fd_ops.applied.get() {
        pw_log::info!("pldm fd: update applied, exiting");
        let _ = syscall::debug_shutdown(Ok(()));
    } else {
        pw_log::error!("pldm fd: update did not complete");
        let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
    }
    #[expect(clippy::empty_loop)]
    loop {}
}
