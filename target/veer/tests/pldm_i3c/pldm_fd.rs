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
//! evaluation: `download_fw_data` takes one chunk per call, `verify`/`apply`
//! report done immediately, and `get_xfer_size` is capped small so each
//! `RequestFirmwareData` chunk response stays a single MCTP fragment (inbound
//! multi-fragment is not yet supported by the i3c server).

#![no_main]
#![no_std]

use core::cell::{Cell, RefCell};

use openprot_mctp_client_ipc::IpcMctpClient;
use openprot_pldm_service::firmware_device::{
    FdEvent, FdEventSink, FirmwareDevice, RunTerminusResult, FD_MAX_MSG,
};
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

/// Update Agent EID this FD serves (the host).
const UA_EID: u8 = 0x0a;
/// Small firmware image so a full download is a handful of single-fragment
/// chunks.
const IMAGE_SIZE: usize = 512;
/// Cap the negotiated transfer size so each `RequestFirmwareData` chunk
/// response fits one MCTP fragment (4 i3c + 4 MCTP + 1 type + PLDM hdr + data +
/// PEC must stay under the 250-byte i3c frame / 241-byte MTU).
const FD_XFER_CAP: usize = 180;
/// Idle responder poll timeout (ms); 0 would block indefinitely.
const IDLE_TIMEOUT_MILLIS: u32 = 100;
/// Bound each FD-initiated request's wait for the UA response.
const REQUESTER_TIMEOUT_MILLIS: u32 = 2000;
/// Steps to keep running after the stop event so the last response is read
/// before the process exits (a small grace against the staging/exit race).
const GRACE_STEPS: u32 = 20;

/// Stops the FD loop once the update lifecycle reaches its (Phase A) endpoint.
///
/// Phase A drives only `RequestUpdate`, so `UpdateRequested` is the endpoint.
/// (Phase B/C will move the stop to after activation.)
struct StopOnUpdateRequested<'a> {
    done: &'a Cell<bool>,
}

impl FdEventSink for StopOnUpdateRequested<'_> {
    fn notify(&mut self, event: FdEvent) {
        if matches!(event, FdEvent::UpdateRequested) {
            self.done.set(true);
        }
    }
}

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

    let mut fd = FirmwareDevice::init(
        &fd_ops,
        &PLDM_PROTOCOL_CAPABILITIES,
        responder_transport,
        requester_transport,
    );

    pw_log::info!("pldm fd: waiting for update agent");

    let mut buf = [0u8; FD_MAX_MSG];
    let done = Cell::new(false);
    let mut sink = StopOnUpdateRequested { done: &done };

    // `run_until` returns `StoppedByError(Mctp(timeout))` on every idle poll
    // timeout (respond_once maps a listener recv-timeout to that), so it is not
    // a fatal error while idle — it means "no command this window." Loop over
    // it, keeping the listener re-armed, until we accept a RequestUpdate; then
    // stay a short grace so the host reads the response, and exit 0.
    let mut grace = GRACE_STEPS;
    loop {
        match fd.run_until(
            UA_EID,
            &mut buf,
            IDLE_TIMEOUT_MILLIS,
            REQUESTER_TIMEOUT_MILLIS,
            &mut sink,
            || true,
        ) {
            RunTerminusResult::StoppedByError(PldmServiceError::Mctp(e)) if e.is_timeout() => {}
            RunTerminusResult::StoppedByError(_) => {
                pw_log::error!("pldm fd: run_until stopped by error");
                let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
                #[expect(clippy::empty_loop)]
                loop {}
            }
            RunTerminusResult::Completed => {}
        }
        if done.get() {
            if grace == 0 {
                break;
            }
            grace -= 1;
        }
    }

    pw_log::info!("pldm fd: update requested, exiting");
    let _ = syscall::debug_shutdown(Ok(()));
    #[expect(clippy::empty_loop)]
    loop {}
}
