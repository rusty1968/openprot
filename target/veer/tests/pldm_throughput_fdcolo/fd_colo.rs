// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! The PLDM FirmwareDevice, the MCTP server and the I3C driver in ONE process.
//!
//! `pldm_throughput_colo` already merges the i3c and mctp servers (removing the
//! `mctp <-> i3c` IPC boundary) but keeps the FD in its own process, talking to
//! the MCTP server over a kernel channel. This variant removes that boundary too:
//! the FD's MCTP client is an in-process [`MctpClient`] that calls the `Server`
//! directly. The IPC round trip (about 21 ms on the FPGA for the cheapest op)
//! and the server-to-FD wake disappear; nothing else in the FD changes, it is the
//! same `FirmwareDevice::run_terminus` over `MctpPldmTransport`.
//!
//! One thread does everything, so the client's blocking calls drive the event
//! loop themselves: `recv` / `recv_any` check the server, and while nothing has
//! arrived they wait on the I3C interrupt (with the caller's timeout as the
//! deadline), service it (drain frames into the MCTP router) and check again.
//!
//! Tradeoff, as for the other colocated variants: the process boundary was
//! fault and privilege isolation between the FD, the MCTP stack and the I3C
//! driver. This is an experiment to price that isolation on the full PLDM
//! download, not a recommendation to merge.

#![no_main]
#![no_std]

use core::cell::{Cell, RefCell};

use caliptra_i3c_target::CaliptraI3cTarget;
use fd_colo_codegen::{handle, signals};
use i3c_api::{Transport, TransportError};
use i3c_client::I3cClient;
use i3c_server::{dispatch, Inbound, Server as I3cServer};
use openprot_hal_blocking::i3c_hardware::{I3cTarget, TargetEvent};
use openprot_mctp_api::{Handle, MctpClient, MctpError, RecvMetadata, ResponseCode};
use openprot_mctp_server::Server as MctpServer;
use openprot_mctp_transport_i3c::{I3cSender, MctpI3cReceiver};
use openprot_pldm_service::firmware_device::{FirmwareDevice, RunTerminusResult, FD_MAX_MSG};
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
use userspace::process_entry;
use userspace::syscall::{self, Signals};
use userspace::time::{Clock, Duration, Instant, SystemClock};

/// This FD's own MCTP EID, also the colocated MCTP server's EID.
const FD_EID: u8 = 8;
/// Update Agent EID this FD serves (the host).
const UA_EID: u8 = 0x0a;
/// Component size assumed until `UpdateComponent` supplies the real one. The
/// pldm_i3c host test serves an image of this size.
const DEFAULT_IMAGE_SIZE: usize = 4096;
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

/// Device UUID the Update Agent's package targets (same as the native FD).
const DEVICE_UUID: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

struct MockFdOps {
    component_accepted: Cell<bool>,
    /// Size of the component being downloaded, from `UpdateComponent`.
    image_size: Cell<usize>,
    download_bytes_received: Cell<usize>,
    verified: Cell<bool>,
    applied: Cell<bool>,
}

impl MockFdOps {
    const fn new() -> Self {
        Self {
            component_accepted: Cell::new(false),
            image_size: Cell::new(DEFAULT_IMAGE_SIZE),
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
        let active_version = PldmFirmwareVersion::new(0x1234_5678, &active_string, Some("20250210"));
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
        *params = FirmwareParameters::new(
            capabilities,
            1,
            &active_string,
            &pending_string,
            &[entry],
        );
        Ok(())
    }

    fn get_xfer_size(&self, ua_transfer_size: usize) -> Result<usize, FdOpsError> {
        Ok(ua_transfer_size.min(FD_XFER_CAP))
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
            .min(FD_XFER_CAP);
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

const REMOTE_I3C_ADDR: u8 = 0x0a;
const PEC: bool = true;
const I3C_RX_MAX: usize = i3c_api::MAX_PAYLOAD;

/// In-process i3c transport: `dispatch` straight against the shared i3c server.
struct ColoTransport<'a> {
    srv: &'a RefCell<I3cSrv>,
}

impl Transport for ColoTransport<'_> {
    fn transact(
        &mut self,
        req: &[u8],
        resp: &mut [u8],
    ) -> core::result::Result<usize, TransportError> {
        let mut srv = self.srv.borrow_mut();
        Ok(dispatch(&mut srv, req, resp))
    }
}

type I3cSrv = I3cServer<CaliptraI3cTarget>;
type MctpSrv<'a> = MctpServer<I3cSender<ColoTransport<'a>>, 16>;

/// Everything the in-process MCTP client needs: the MCTP router, the i3c
/// driver state, the frame-decoding path and the time base.
struct Colo<'a> {
    i3c: &'a RefCell<I3cSrv>,
    mctp: &'a RefCell<MctpSrv<'a>>,
    rx_client: RefCell<I3cClient<ColoTransport<'a>>>,
    receiver: RefCell<MctpI3cReceiver>,
    rx_buf: RefCell<[u8; I3C_RX_MAX]>,
    epoch: Instant,
}

impl Colo<'_> {
    /// Whole milliseconds since the epoch the MCTP server's clock started at.
    fn millis(&self) -> u64 {
        (SystemClock::now() - self.epoch).as_millis() as u64
    }

    /// A kernel instant by which `millis()` has reached `millis` (one
    /// millisecond late on purpose, as `mctp_server_runtime` does, so a wake on
    /// the deadline never reads the clock one short).
    fn instant_at(&self, millis: u64) -> Instant {
        self.epoch
            .checked_add_duration(Duration::from_millis(millis.saturating_add(1)))
            .unwrap_or(Instant::MAX)
    }

    /// Bring the router's clock up to now. Needed before a packet is fed to it
    /// or a message is sent, so what they create is dated now.
    fn advance(&self) {
        self.mctp.borrow_mut().advance(self.millis());
    }

    /// Handle the i3c interrupt: drain the controller's frames into the MCTP
    /// router, then acknowledge it (the wake source is level-triggered).
    fn service_irq(&self) {
        self.advance();
        let evt = self.i3c.borrow_mut().target.on_interrupt();
        match evt {
            Ok(TargetEvent::InboundReady) => loop {
                match self.i3c.borrow_mut().latch_inbound() {
                    Ok(Inbound::Latched) => continue,
                    Ok(Inbound::DroppedFull) => {
                        pw_log::error!("fd colo: i3c inbound ring full; frame dropped");
                        break;
                    }
                    Ok(Inbound::Empty) => break,
                    Err(_) => {
                        pw_log::error!("fd colo: i3c read_frame failed");
                        break;
                    }
                }
            },
            Ok(TargetEvent::ResponseRead) => self.i3c.borrow_mut().notify_response_read(),
            _ => {}
        }
        let _ = syscall::interrupt_ack(handle::I3C_IRQ, signals::I3C);

        let mut rx_client = self.rx_client.borrow_mut();
        let mut receiver = self.receiver.borrow_mut();
        let mut buf = self.rx_buf.borrow_mut();
        while let Ok(Some(n)) = rx_client.recv(&mut *buf) {
            let decoded = buf
                .get(..n)
                .map(|frame| receiver.decode(frame, || rx_client.dynamic_address().ok().flatten()));
            if let Some(Ok(pkt)) = decoded {
                let _ = self.mctp.borrow_mut().inbound(pkt);
            } else {
                pw_log::error!("fd colo: i3c frame decode failed");
            }
        }
    }

    /// Block until the i3c interrupt fires or `deadline` passes, servicing the
    /// interrupt if it fired. The caller re-checks its own condition.
    fn wait_irq(&self, deadline: Instant) {
        // A wait group is readable when any of its sources is pending (the same mask
        // `util_service` waits with).
        match syscall::object_wait(handle::WG, Signals::READABLE | signals::I3C, deadline) {
            Ok(_) => self.service_irq(),
            Err(pw_status::Error::DeadlineExceeded) => {}
            Err(_) => pw_log::error!("fd colo: wait failed"),
        }
    }

    /// Run `attempt` against the MCTP server until it yields a value or
    /// `timeout_millis` (0 = forever) passes, servicing the i3c interrupt
    /// whenever there is nothing yet.
    fn block_until<T>(
        &self,
        timeout_millis: u32,
        mut attempt: impl FnMut(&mut MctpSrv<'_>) -> Option<T>,
    ) -> Result<T, MctpError> {
        let start = self.millis();
        loop {
            self.advance();
            if let Some(value) = attempt(&mut self.mctp.borrow_mut()) {
                return Ok(value);
            }
            let deadline = if timeout_millis == 0 {
                Instant::MAX
            } else {
                let due = start.saturating_add(u64::from(timeout_millis));
                if self.millis() >= due {
                    return Err(MctpError::from_code(ResponseCode::TimedOut));
                }
                self.instant_at(due)
            };
            self.wait_irq(deadline);
        }
    }
}

/// The FD's MCTP client: direct calls on the in-process server. Blocking calls
/// run the event loop (see [`Colo::block_until`]).
#[derive(Clone, Copy)]
struct ColoClient<'c, 'a> {
    colo: &'c Colo<'a>,
}

impl MctpClient for ColoClient<'_, '_> {
    fn req(&self, eid: u8) -> Result<Handle, MctpError> {
        self.colo.mctp.borrow_mut().req(eid)
    }

    fn listener(&self, msg_type: u8) -> Result<Handle, MctpError> {
        self.colo.mctp.borrow_mut().listener(msg_type)
    }

    fn get_eid(&self) -> u8 {
        self.colo.mctp.borrow().get_eid()
    }

    fn set_eid(&self, eid: u8) -> Result<(), MctpError> {
        self.colo.mctp.borrow_mut().set_eid(eid)
    }

    fn recv(
        &self,
        handle: Handle,
        timeout_millis: u32,
        buf: &mut [u8],
    ) -> Result<RecvMetadata, MctpError> {
        let meta = self
            .colo
            .block_until(timeout_millis, |m| m.try_recv(handle, buf))?;
        if meta.payload_size > buf.len() {
            // The server consumed a message the caller's buffer cannot hold.
            return Err(MctpError::from_code(ResponseCode::NoSpace));
        }
        Ok(meta)
    }

    fn recv_any(
        &self,
        handles: &[Handle],
        timeout_millis: u32,
        buf: &mut [u8],
    ) -> Result<(Handle, RecvMetadata), MctpError> {
        let (fired, meta) = self
            .colo
            .block_until(timeout_millis, |m| m.try_recv_any(handles, buf))?;
        if meta.payload_size > buf.len() {
            return Err(MctpError::from_code(ResponseCode::NoSpace));
        }
        Ok((fired, meta))
    }

    fn send(
        &self,
        handle: Option<Handle>,
        msg_type: u8,
        eid: Option<u8>,
        tag: Option<u8>,
        integrity_check: bool,
        buf: &[u8],
    ) -> Result<u8, MctpError> {
        self.colo.advance();
        self.colo
            .mctp
            .borrow_mut()
            .send(handle, msg_type, eid, tag, integrity_check, buf)
    }

    fn drop_handle(&self, handle: Handle) {
        let _ = self.colo.mctp.borrow_mut().unbind(handle);
    }
}

#[process_entry("fd_colo")]
fn entry() {
    pw_log::info!("pldm fd + mctp + i3c colocated: starting");

    // SAFETY: this process exclusively owns the I3C peripheral, mapped as a
    // device region by system.json5; Caliptra ROM already initialized the core.
    let target = unsafe { CaliptraI3cTarget::new() };
    // The channel handle is unused (no IPC); the IRQ handle is real.
    let i3c = RefCell::new(I3cSrv::new(0, handle::I3C_IRQ, target));
    let _ = i3c.borrow_mut().target.enable();

    let epoch = SystemClock::now();
    let sender = I3cSender::new(
        I3cClient::new(ColoTransport { srv: &i3c }),
        REMOTE_I3C_ADDR,
        PEC,
    );
    let mctp = RefCell::new(MctpServer::<_, 16>::new(mctp::Eid(FD_EID), 0, sender));

    if syscall::wait_group_add(handle::WG, handle::I3C_IRQ, signals::I3C, 0).is_err() {
        pw_log::error!("fd colo: wait_group_add failed");
        let _ = syscall::debug_shutdown(Err(pw_status::Error::Internal));
        #[expect(clippy::empty_loop)]
        loop {}
    }

    let colo = Colo {
        i3c: &i3c,
        mctp: &mctp,
        rx_client: RefCell::new(I3cClient::new(ColoTransport { srv: &i3c })),
        receiver: RefCell::new(MctpI3cReceiver::new(PEC)),
        rx_buf: RefCell::new([0u8; I3C_RX_MAX]),
        epoch,
    };

    let fd_ops = MockFdOps::new();
    let responder_transport = MctpPldmTransport::new(ColoClient { colo: &colo });
    let requester_transport = MctpPldmTransport::new(ColoClient { colo: &colo });

    // The server already has this EID (set at construction); the FD sets it
    // again, as the AST10x0 reference FD does, through the same client API.
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

    // Same single `run_terminus` as the separate-process FD: it services UA->FD
    // commands and issues the FD-initiated download/verify/apply requests from
    // this one thread; the idle timeout after ApplyComplete is expected.
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
