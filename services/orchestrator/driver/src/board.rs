// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! What the board supplies to the driver: traits and wiring data only.
//! Boards (or test mocks) implement these.

use openprot_orchestrator_sm::{BootFailureKind, ComponentId, ComponentKind};

pub use orchestrator_capabilities::{BootControl, BootWatch};
use orchestrator_capabilities::{Svn, SvnFloor};

/// Access to one component's active firmware image, however it is reached —
/// interposed flash, a PLDM/MCTP transfer, a RAM copy in tests.
pub trait ImageSource {
    /// The error type reported by this source.
    type Error: core::error::Error;

    /// Makes the image readable (claim the flash, open the transfer).
    /// Idempotent; a later `open` re-stages the image.
    fn open(&mut self) -> Result<(), Self::Error>;

    /// Image length in bytes. Only meaningful after a successful `open`;
    /// sources that learn the size during `open` cache it and report the
    /// cached value here.
    fn size(&self) -> Result<usize, Self::Error>;

    /// Reads `buf.len()` bytes starting at byte `offset` of the image.
    fn read_at(&mut self, offset: usize, buf: &mut [u8]) -> Result<(), Self::Error>;
}

impl<S: ImageSource> ImageSource for &mut S {
    type Error = S::Error;

    #[inline(always)]
    fn open(&mut self) -> Result<(), Self::Error> {
        (**self).open()
    }

    #[inline(always)]
    fn size(&self) -> Result<usize, Self::Error> {
        (**self).size()
    }

    #[inline(always)]
    fn read_at(&mut self, offset: usize, buf: &mut [u8]) -> Result<(), Self::Error> {
        (**self).read_at(offset, buf)
    }
}

/// Judges a component's firmware image; board wiring decides what counts
/// as authenticated.
pub trait Verifier {
    /// The error type reported by this verifier.
    type Error: core::error::Error;

    /// Judges `id`'s image, reading it from `image`.
    ///
    /// # Errors
    ///
    /// Only when the check could not be performed (crypto fault, missing
    /// key, unreadable source). A checked-and-bad image is
    /// `Ok(Verdict::Rejected)` — an actuation fault must not forge a
    /// verdict.
    fn verify(
        &mut self,
        id: ComponentId,
        image: &mut impl ImageSource,
    ) -> Result<Verdict, Self::Error>;
}

impl<V: Verifier> Verifier for &mut V {
    type Error = V::Error;

    #[inline(always)]
    fn verify(
        &mut self,
        id: ComponentId,
        image: &mut impl ImageSource,
    ) -> Result<Verdict, Self::Error> {
        (**self).verify(id, image)
    }
}

/// Judges a component's firmware image without making the caller wait.
///
/// The split mirrors `util_transport::AsyncTransport`: `start` begins a check
/// and returns, `poll` reports whether it has finished, and `cancel` abandons
/// it. A board whose check is a request to a crypto service implements this
/// directly, so the orchestrator keeps serving timers and events while the
/// image is hashed. A board whose check is fast and local can wrap its
/// [`Verifier`] in [`Immediate`].
///
/// One check is outstanding at a time, and the driver cancels before starting
/// another, so an implementation never holds two.
pub trait AsyncVerifier {
    /// The error type reported by this verifier.
    type Error: core::error::Error;

    /// Begins judging `id`'s image. An implementation that reaches the image
    /// itself, such as a service that reads it from the device server, may
    /// ignore `image`.
    ///
    /// # Errors
    ///
    /// Only when the check could not be started. A checked-and-bad image is
    /// a [`Verdict::Rejected`] from `poll`, not an error here.
    fn start(
        &mut self,
        id: ComponentId,
        image: &mut impl ImageSource,
    ) -> Result<(), Self::Error>;

    /// Reports the outcome of the check started for `id`. Never waits.
    ///
    /// # Errors
    ///
    /// Only when the check could not be carried through (the service
    /// failed, the transport broke). That is a fault, not a verdict. An
    /// error ends the check.
    fn poll(&mut self, id: ComponentId) -> Result<VerifyStatus, Self::Error>;

    /// Abandons the check started for `id`. Its answer, if one arrives
    /// later, is discarded.
    fn cancel(&mut self, id: ComponentId) -> Result<(), Self::Error>;
}

impl<V: AsyncVerifier> AsyncVerifier for &mut V {
    type Error = V::Error;

    #[inline(always)]
    fn start(
        &mut self,
        id: ComponentId,
        image: &mut impl ImageSource,
    ) -> Result<(), Self::Error> {
        (**self).start(id, image)
    }

    #[inline(always)]
    fn poll(&mut self, id: ComponentId) -> Result<VerifyStatus, Self::Error> {
        (**self).poll(id)
    }

    #[inline(always)]
    fn cancel(&mut self, id: ComponentId) -> Result<(), Self::Error> {
        (**self).cancel(id)
    }
}

/// Where an asynchronous check stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyStatus {
    /// Still running.
    Pending,
    /// Finished with a judgment.
    Done(Verdict),
    /// Ran past the implementation's deadline with no answer. Not a verdict:
    /// nothing was judged. The driver reports it as a failed verification, so
    /// the machine recovers the component instead of trusting or locking.
    TimedOut,
}

/// Runs a synchronous [`Verifier`] as an [`AsyncVerifier`]: the check happens
/// inside `start`, as it always did, and the first `poll` returns the
/// verdict. For boards whose check is fast and local, and for tests.
pub struct Immediate<V: Verifier> {
    inner: V,
    done: Option<(ComponentId, Verdict)>,
}

impl<V: Verifier> Immediate<V> {
    pub const fn new(inner: V) -> Self {
        Self { inner, done: None }
    }
}

impl<V: Verifier> AsyncVerifier for Immediate<V> {
    type Error = V::Error;

    fn start(
        &mut self,
        id: ComponentId,
        image: &mut impl ImageSource,
    ) -> Result<(), Self::Error> {
        self.done = None;
        let verdict = self.inner.verify(id, image)?;
        self.done = Some((id, verdict));
        Ok(())
    }

    fn poll(&mut self, id: ComponentId) -> Result<VerifyStatus, Self::Error> {
        Ok(match self.done {
            Some((done_id, verdict)) if done_id == id => {
                self.done = None;
                VerifyStatus::Done(verdict)
            }
            _ => VerifyStatus::Pending,
        })
    }

    fn cancel(&mut self, _id: ComponentId) -> Result<(), Self::Error> {
        self.done = None;
        Ok(())
    }
}

/// A [`Verifier`]'s judgment of one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Reported as `Event::VerificationPassed`. Carries the image's
    /// manifest SVN: verification is the only authenticated reading of the
    /// manifest, so this is the one value the anti-rollback commit
    /// ([`Effect::CommitSvnFloor`](openprot_orchestrator_sm::Effect)) may
    /// trust.
    Authenticated {
        /// The verified image's security version number.
        svn: Svn,
    },
    /// Reported as `Event::VerificationFailed`.
    Rejected,
}

/// One fact about the platform running degraded, carried outward to
/// management software. `#[non_exhaustive]`: a sink routes what it
/// recognises and ignores the rest, so a new report is not a trait break.
///
/// Payloads stay `Copy` and lifetime-free, like the effects these mirror. A
/// report names a component only where the effect it mirrors does.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    /// Held in reset and out of the trust chain, so the platform runs
    /// degraded. Reported once per component, as it is gated.
    Isolated(ComponentId),
    /// Recovery attempts exhausted and the platform halts. Reported before
    /// the lockdown latch, while there is still a platform to report from.
    RecoveryFailed(ComponentId),
    /// A boot walk failed at a named checkpoint. The `kind` tells whether
    /// this was silence (timed out), a retriable device fault, or a terminal
    /// device fault.
    BootFailed {
        id: ComponentId,
        checkpoint: &'static str,
        kind: BootFailureKind,
    },
    /// An update request declined because the platform was busy. Nothing
    /// staged, and the requester may ask again. Platform-wide, not
    /// per-component: the machine supervises one update at a time and
    /// `Event::UpdateRequest` names no component.
    UpdateDeferred,
    /// An update in flight superseded by recovery. Its staged image is
    /// discarded and no verdict for that request follows. Platform-wide for
    /// the same reason as [`Report::UpdateDeferred`].
    UpdateAborted,
}

/// Where the driver hands its [`Report`]s. What a report becomes, a log
/// entry, a management transport message, a fault line, is board wiring.
///
/// Infallible by design: a report names something that already happened, so
/// an undeliverable one costs information, not containment. An error channel
/// would put reports on the fail-closed path, letting the act of reporting a
/// contained failure escalate it.
pub trait ReportSink {
    /// Receives one report. A sink that cannot deliver immediately queues on
    /// its own side rather than stalling the effect batch.
    fn report(&mut self, report: Report);
}

/// Drops every report, for a board with no management side to tell. Losing
/// reports costs visibility only, so this is a wiring choice, not a stub.
impl ReportSink for () {
    #[inline(always)]
    fn report(&mut self, _report: Report) {}
}

impl<S: ReportSink> ReportSink for &mut S {
    #[inline(always)]
    fn report(&mut self, report: Report) {
        (**self).report(report)
    }
}

/// The set of platform capabilities one board composes into the
/// `PlatformDriver`, named by a marker type. A new seam adds an associated
/// type here and a field on [`Board`] — never another parameter.
pub trait BoardCapabilities {
    /// Image access for the managed components.
    type Image: ImageSource;
    /// Judges images for every component, without making the driver wait.
    /// A fast local [`Verifier`] is wrapped in [`Immediate`].
    type Verifier: AsyncVerifier;
    /// Reset actuation for the managed components.
    type BootControl: BootControl;
    /// Boot-checkpoint supervision for the managed components.
    type BootWatch: BootWatch;
    /// The anti-rollback floor of the managed components. The SVN number
    /// survives reset and power loss, otherwise a power cycle would
    /// re-admit images below the floor.
    type SvnFloor: SvnFloor;
    /// Where reports go. `()` for a board with no management side to tell.
    type ReportSink: ReportSink;
    // Later seams: Recovery, Staging.
}

/// Who keeps one component's anti-rollback floor. Spelled as its own type
/// so a board must state the choice; an eRoT floor and a device tracking
/// its own SVN are different wirings, not a present or absent value.
pub enum SvnFloorBinding<F: SvnFloor> {
    /// The eRoT holds the floor and `CommitSvnFloor` advances it.
    Erot(F),
    /// The component tracks its own SVN (iRoT, or a PLDM device
    /// committing internally). The eRoT keeps no second floor and its
    /// `CommitSvnFloor` is a no-op.
    SelfManaged,
}

/// Everything the board supplies, built once at bring-up and handed to
/// `PlatformDriver::new`. Fields are public: executors may need two parts at once
/// (disjoint borrows).
///
/// ```ignore
/// struct Ast1060Board;
/// impl BoardCapabilities for Ast1060Board {
///     type Image = SpiFlashImage;         // interposed flash, offsets from the slot layout
///     type Verifier = Immediate<ManifestVerifier>; // signature + SVN via the crypto engine
///     type BootControl = ExtrstGpio;      // per-component reset line
///     type BootWatch = CheckpointWalk;    // GPIO checkpoint walk over the boot window
///     type SvnFloor = OtpSvnFloor;        // fuse-backed anti-rollback floor
///     type ReportSink = MctpReports;      // reports out over the management transport
/// }
/// let board = Board::<Ast1060Board, 2> {
///     images: [bmc_image, cpld_image],
///     verifier,
///     boot_controls: [bmc_reset, cpld_reset],
///     boot_watches: [bmc_walk, cpld_walk],
///     component_kinds: [ComponentKind::Active, ComponentKind::Passive],
///     svn_floors: [SvnFloorBinding::Erot(bmc_floor), SvnFloorBinding::SelfManaged],
///     report_sink,
/// };
/// ```
pub struct Board<B: BoardCapabilities, const N: usize> {
    /// `images[i]` belongs to `ComponentId(i)` — device index = chain
    /// position = table declaration order.
    pub images: [B::Image; N],
    /// Judges images for every component.
    pub verifier: B::Verifier,
    /// `boot_controls[i]` actuates `ComponentId(i)`'s reset, same indexing
    /// as `images`.
    pub boot_controls: [B::BootControl; N],
    /// `boot_watches[i]` supervises `ComponentId(i)`'s boot walk, same
    /// indexing as `images`.
    pub boot_watches: [B::BootWatch; N],
    /// `component_kinds[i]` classifies `ComponentId(i)`: a completed walk becomes
    /// `ComponentReady` for `Active`, `Booted` for `Passive`. Comes from
    /// the same board table as the SM's chain, so both sides agree.
    pub component_kinds: [ComponentKind; N],
    /// `svn_floors[i]` says who keeps `ComponentId(i)`'s anti-rollback
    /// floor, same indexing as `images`.
    pub svn_floors: [SvnFloorBinding<B::SvnFloor>; N],
    /// Where the driver hands the SM's reports. One per platform, not one
    /// per component: two of the four reports name no component.
    pub report_sink: B::ReportSink,
    // Later seams add fields, e.g. recovery: [B::Recovery; N].
}
