// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! SPI monitor service control-plane API.
//!
//! This crate intentionally does not depend on target-specific peripheral
//! crates. It defines the transport-agnostic contract used by orchestrator,
//! client, and server layers.

/// Maximum number of SPI monitor instances on AST10x0 platforms.
pub const MONITOR_COUNT: usize = 4;

/// Stable monitor identifier used by service clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MonitorId {
    Spim0,
    Spim1,
    Spim2,
    Spim3,
}

impl MonitorId {
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Spim0 => 0,
            Self::Spim1 => 1,
            Self::Spim2 => 2,
            Self::Spim3 => 3,
        }
    }
}

impl TryFrom<usize> for MonitorId {
    type Error = ServiceError;

    fn try_from(value: usize) -> Result<Self> {
        match value {
            0 => Ok(Self::Spim0),
            1 => Ok(Self::Spim1),
            2 => Ok(Self::Spim2),
            3 => Ok(Self::Spim3),
            _ => Err(ServiceError::InvalidMonitor),
        }
    }
}

/// Direction selector for address privilege programming.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivilegeDirection {
    Read,
    Write,
}

/// Region policy entry in service-level policy payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionPolicy {
    pub start: u32,
    pub length: u32,
    pub direction: PrivilegeDirection,
}

/// Policy profile applied to one monitor instance.
#[derive(Clone, Debug)]
pub struct MonitorPolicy {
    pub allow_commands: [u8; 32],
    pub allow_command_count: usize,
    pub regions: [Option<RegionPolicy>; 32],
}

impl MonitorPolicy {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            allow_commands: [0; 32],
            allow_command_count: 0,
            regions: [None; 32],
        }
    }
}

/// Static-first policy payload applied at boot/handoff.
#[derive(Clone, Debug)]
pub struct PolicySet {
    /// Per-monitor policy profile. Index order follows `MonitorId`.
    pub per_monitor: [MonitorPolicy; MONITOR_COUNT],
    /// Monitor-enable bitmask. Bit N corresponds to monitor N.
    pub enabled_mask: u8,
}

impl PolicySet {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            per_monitor: [
                MonitorPolicy::empty(),
                MonitorPolicy::empty(),
                MonitorPolicy::empty(),
                MonitorPolicy::empty(),
            ],
            enabled_mask: 0,
        }
    }

    #[must_use]
    pub const fn is_enabled(&self, monitor: MonitorId) -> bool {
        (self.enabled_mask & (1u8 << monitor.index())) != 0
    }
}

/// Lock state observed from hardware policy tables.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockState {
    Unlocked,
    Locked,
}

/// Verify-readback field discriminator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyField {
    Control,
    AllowCommandSlot,
    AddressFilterSlot,
    LockStatus,
}

/// One mismatch returned by verify-readback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifyMismatch {
    pub monitor: MonitorId,
    pub field: VerifyField,
    pub slot: u8,
    pub expected: u32,
    pub actual: u32,
}

/// Lock-state snapshot exported to orchestrator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockStatus {
    pub monitor: MonitorId,
    pub state: LockState,
    pub raw: u32,
}

/// Parsed blocked-access event class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockedEventKind {
    BlockedCommand { opcode: u8 },
    BlockedWriteAddress { address: u32 },
    BlockedReadAddress { address: u32 },
    Unknown,
}

/// Blocked-access event exported by the monitor service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockedEvent {
    pub monitor: MonitorId,
    /// Monotonic service cursor for acknowledgement and resume.
    pub cursor: u32,
    /// Raw register payload (for forensics/audit retention).
    pub raw: u32,
    pub kind: BlockedEventKind,
}

/// Output metadata for paged blocked-event fetches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventPage {
    pub count: usize,
    pub next_cursor: u32,
    pub overflowed: bool,
}

/// Built-in update profile selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateProfileId {
    Runtime,
    FirmwareUpdate,
    Recovery,
    Vendor(u8),
}

/// Authorization material provided by the orchestrator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateWindowToken<'a> {
    pub profile_id: UpdateProfileId,
    pub nonce: u32,
    pub issued_at_seconds: u64,
    pub expires_at_seconds: u64,
    pub issuer_key_id: u8,
    pub signature: &'a [u8],
}

/// Handle that represents an active temporary policy window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateWindowSession {
    pub id: u32,
    pub profile_id: UpdateProfileId,
}

/// SPI monitor service-level errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    InvalidMonitor,
    InvalidCursor,
    InvalidProfile,
    InvalidToken,
    TokenExpired,
    NotAuthorized,
    UpdateWindowAlreadyOpen,
    UpdateWindowNotOpen,
    ReadbackMismatch,
    BufferTooSmall,
    Busy,
    HardwareFault,
    Internal,
}

/// Result alias for SPI monitor service operations.
pub type Result<T> = core::result::Result<T, ServiceError>;

/// Control-plane API for a SPI monitor service.
pub trait SpiMonitorService {
    /// Apply policy to all enabled monitor instances.
    fn init_apply(&mut self, policy_set: &PolicySet) -> Result<()>;

    /// Verify monitor state against the expected policy and lock plan.
    ///
    /// Implementations should fill `mismatches` and return `ReadbackMismatch`
    /// when any discrepancy is found.
    fn verify_readback(&self, mismatches: &mut [VerifyMismatch]) -> Result<usize>;

    /// Execute one-way lock transition for runtime policy.
    fn lock_runtime_policy(&mut self) -> Result<()>;

    /// Export lock status for attestation/reporting.
    fn get_lock_status(&self, out: &mut [LockStatus]) -> Result<usize>;

    /// Fetch blocked-access events after `cursor`, writing up to `out.len()`.
    fn get_blocked_events(&mut self, cursor: u32, out: &mut [BlockedEvent]) -> Result<EventPage>;

    /// Acknowledge events up to `cursor`.
    fn clear_blocked_events(&mut self, cursor: u32) -> Result<()>;

    /// Enter a bounded, authenticated update profile window.
    fn enter_update_window(
        &mut self,
        profile_id: UpdateProfileId,
        token: &UpdateWindowToken<'_>,
    ) -> Result<UpdateWindowSession>;

    /// Exit a previously opened update profile window.
    fn exit_update_window(&mut self, session: UpdateWindowSession) -> Result<()>;
}
