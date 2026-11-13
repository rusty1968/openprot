// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! The PLDM Firmware Device (FD) the VeeR PLDM test images run.
//!
//! Runs the real `openprot-pldm-service` `FirmwareDevice::run_terminus` state
//! machine over MCTP: a responder and a requester [`MctpClient`] (one per
//! direction, as `FirmwareDevice` wants). The images give it
//! `IpcMctpClient`s to an `mctp_server` process, or, in the all-in-one image, an
//! in-process client. The host plays the Update Agent. What the image differs in
//! is only which clients it passes and the [`FdConfig`].
//!
//! `MockFdOps` is a bounded-slice implementation per the callback-shape
//! evaluation: `download_fw_data` takes one chunk per call and `verify`/`apply`
//! report done immediately. `get_xfer_size` allows a chunk larger than one MCTP
//! fragment when [`FdConfig::xfer_cap`] does: each `RequestFirmwareData`
//! response then spans a few inbound fragments, which the i3c server's inbound
//! ring queues and the FD's MCTP stack reassembles, so fewer, bigger round trips.
//!
//! The device identity and firmware parameters match the caliptra-mcu-sw native
//! FD (`firmware_update/config.rs`), so the same Update Agent package
//! (`get_streaming_boot_pldm_fw_manifest`) accepts either stack. The component
//! size comes from `UpdateComponent`, and downloaded bytes are counted and
//! discarded, like the native `bench-skip-validation` path (no staging writes).

#![no_std]

use core::cell::Cell;

use openprot_mctp_api::MctpClient;
use openprot_pldm_service::firmware_device::{
    FdEvent, FdEventSink, FirmwareDevice, RunTerminusResult, FD_MAX_MSG,
};
use openprot_pldm_service::{MctpPldmTransport, PldmServiceError};
use pldm_common::message::firmware_update::apply_complete::ApplyResult;
use pldm_common::message::firmware_update::get_fw_params::FirmwareParameters;
use pldm_common::message::firmware_update::get_status::ProgressPercent;
use pldm_common::message::firmware_update::transfer_complete::TransferResult;
use pldm_common::message::firmware_update::verify_complete::VerifyResult;
use pldm_common::protocol::firmware_update::{
    ComponentActivationMethods, ComponentClassification, ComponentParameterEntry,
    ComponentResponseCode, Descriptor, DescriptorType, FirmwareDeviceCapability,
    PldmFirmwareString, PldmFirmwareVersion,
};
use pldm_common::util::fw_component::FirmwareComponent;
use pldm_interface::config::PLDM_PROTOCOL_CAPABILITIES;
use pldm_interface::firmware_device::fd_ops::{ComponentOperation, FdOps, FdOpsError};
use userspace::syscall;

/// This FD's own MCTP EID; must match the MCTP server's own EID underneath it.
pub const FD_EID: u8 = 8;
/// Update Agent EID this FD serves (the host).
pub const UA_EID: u8 = 0x0a;

/// Bound each FD-initiated request's wait for the UA response.
const REQUESTER_TIMEOUT_MILLIS: u32 = 5_000;

/// Device UUID the Update Agent's package targets (same as the native FD).
const DEVICE_UUID: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

/// Logs the line a host test waits for before it sends its first command
/// ("pldm fd: waiting for update agent") when the FD starts listening.
///
/// Not before: until the responder's listener is registered an Update Agent
/// command that arrives has nowhere to go and is dropped, and a fast transport
/// (an FD in the same process as the MCTP server) can deliver one in that
/// window. The log line is the signal the host acts on, so it has to mean the
/// listener exists.
struct ReadySink;

impl FdEventSink for ReadySink {
    fn notify(&mut self, event: FdEvent) {
        if matches!(event, FdEvent::Listening) {
            pw_log::info!("pldm fd: waiting for update agent");
        }
    }
}

/// What differs between the images that run this FD.
#[derive(Clone, Copy)]
pub struct FdConfig {
    /// Cap on the chunk size the FD asks for in each `RequestFirmwareData`.
    ///
    /// Up to 241 bytes the chunk response is one MCTP fragment. Larger spans
    /// several, which needs the i3c server's inbound ring (`RX_RING`) to hold a
    /// whole message's fragments and `pldm-common`'s `MAX_TRANSFER_SIZE` to allow
    /// it (raised to 960 by a crate patch; the reassembled message must stay
    /// within the MCTP stack's 1032 byte payload limit).
    pub xfer_cap: usize,
    /// Component size assumed until `UpdateComponent` supplies the real one.
    pub default_image_size: usize,
    /// How long the responder waits for a UA command while idle, in
    /// milliseconds. `run_terminus` is called once and blocks here between
    /// commands, like the AST10x0 reference FD, rather than being re-entered in a
    /// tight poll loop (that churn of timed-out receives crashed an earlier
    /// version). The host UA stops after `ActivateFirmware`, and the FD exits
    /// when this runs out with nothing more to do, so it is also how long a run
    /// takes to finish after the update.
    pub idle_timeout_millis: u32,
}

struct MockFdOps {
    xfer_cap: usize,
    component_accepted: Cell<bool>,
    /// Size of the component being downloaded, from `UpdateComponent`.
    image_size: Cell<usize>,
    download_bytes_received: Cell<usize>,
    verified: Cell<bool>,
    applied: Cell<bool>,
}

impl MockFdOps {
    const fn new(config: FdConfig) -> Self {
        Self {
            xfer_cap: config.xfer_cap,
            component_accepted: Cell::new(false),
            image_size: Cell::new(config.default_image_size),
            download_bytes_received: Cell::new(0),
            verified: Cell::new(false),
            applied: Cell::new(false),
        }
    }
}

impl FdOps for MockFdOps {
    fn get_device_identifiers(&self, ids: &mut [Descriptor]) -> Result<usize, FdOpsError> {
        let slot = ids.first_mut().ok_or(FdOpsError::DeviceIdentifiersError)?;
        *slot = Descriptor::new(DescriptorType::Uuid, &DEVICE_UUID)
            .map_err(|_| FdOpsError::DeviceIdentifiersError)?;
        Ok(1)
    }

    fn get_firmware_parms(&self, params: &mut FirmwareParameters) -> Result<(), FdOpsError> {
        // Same values as the native FD: one firmware component, identifier 0xffff,
        // active comparison stamp below the package's 0xffffffff.
        let err = |_| FdOpsError::FirmwareParametersError;
        let active_string = PldmFirmwareString::new("UTF-8", "soc-fw-1.0").map_err(err)?;
        let active_version =
            PldmFirmwareVersion::new(0x1234_5678, &active_string, Some("20250210"));
        let pending_string = PldmFirmwareString::new("UTF-8", "soc-fw-1.1").map_err(err)?;
        let pending_version =
            PldmFirmwareVersion::new(0x8765_4321, &pending_string, Some("20250213"));
        let activation_methods = ComponentActivationMethods(0x0001);
        let capabilities = FirmwareDeviceCapability(0x0010);
        let entry = ComponentParameterEntry::new(
            ComponentClassification::Firmware,
            0xffff,
            0,
            &active_version,
            &pending_version,
            activation_methods,
            capabilities,
        );
        *params =
            FirmwareParameters::new(capabilities, 1, &active_string, &pending_string, &[entry]);
        Ok(())
    }

    fn get_xfer_size(&self, ua_transfer_size: usize) -> Result<usize, FdOpsError> {
        Ok(ua_transfer_size.min(self.xfer_cap))
    }

    fn handle_component(
        &self,
        component: &FirmwareComponent,
        _fw_params: &FirmwareParameters,
        _op: ComponentOperation,
    ) -> Result<ComponentResponseCode, FdOpsError> {
        // PassComponentTable carries no size; UpdateComponent does.
        if let Some(size) = component.comp_image_size {
            self.image_size.set(size as usize);
        }
        self.component_accepted.set(true);
        Ok(ComponentResponseCode::CompCanBeUpdated)
    }

    fn query_download_offset_and_length(
        &self,
        _component: &FirmwareComponent,
    ) -> Result<(usize, usize), FdOpsError> {
        let offset = self.download_bytes_received.get();
        let length = self
            .image_size
            .get()
            .checked_sub(offset)
            .ok_or(FdOpsError::FwDownloadError)?
            .min(self.xfer_cap);
        Ok((offset, length))
    }

    fn download_fw_data(
        &self,
        offset: usize,
        data: &[u8],
        _component: &FirmwareComponent,
    ) -> Result<TransferResult, FdOpsError> {
        // Count and discard: no staging writes, like the native bench path.
        let end = offset
            .checked_add(data.len())
            .ok_or(FdOpsError::FwDownloadError)?;
        if end > self.image_size.get() {
            return Err(FdOpsError::FwDownloadError);
        }
        self.download_bytes_received
            .set(end.max(self.download_bytes_received.get()));
        Ok(TransferResult::TransferSuccess)
    }

    fn is_download_complete(&self, _component: &FirmwareComponent) -> bool {
        self.download_bytes_received.get() >= self.image_size.get()
    }

    fn query_download_progress(
        &self,
        _component: &FirmwareComponent,
        progress_percent: &mut ProgressPercent,
    ) -> Result<(), FdOpsError> {
        let size = self.image_size.get().max(1);
        let pct = (self.download_bytes_received.get() * 100 / size) as u8;
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

/// Run the FD to the end of one update, then shut the system down: PASS if the
/// image was applied, FAIL if not. Never returns.
pub fn run<Cr: MctpClient, Cq: MctpClient>(config: FdConfig, responder: Cr, requester: Cq) -> ! {
    // Earliest possible log: if this appears but "waiting for update agent" does
    // not, the crash is in FirmwareDevice/transport construction below (likely
    // stack). If this never appears, the crash is before the FD runs.
    pw_log::info!("pldm fd: starting");

    let fd_ops = MockFdOps::new(config);
    let responder_transport = MctpPldmTransport::new(responder);
    let requester_transport = MctpPldmTransport::new(requester);

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

    let mut buf = [0u8; FD_MAX_MSG];

    // Drive the whole update in a single `run_terminus`, exactly like the
    // AST10x0 reference FD (target/ast10x0/tests/pldm/firmware_update/fd_main.rs):
    // it services UA->FD commands and autonomously issues the FD-initiated
    // download/verify/apply requests from this one thread, waiting for the
    // response or a command at each step. It returns `Completed` when the device
    // goes back to Idle (after ActivateFirmware) or a `Mctp` timeout when the UA
    // stops talking. The host UA stops after ApplyComplete without sending
    // ActivateFirmware, so the terminal idle timeout is expected; success here is
    // "the image was applied".
    match fd.run_terminus(
        UA_EID,
        &mut buf,
        config.idle_timeout_millis,
        REQUESTER_TIMEOUT_MILLIS,
        &mut ReadySink,
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
