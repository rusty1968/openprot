// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

/// Proprietary SMBus mailbox register IDs used by the PFR protocol layer.
pub mod regs {
    // Reuse shared mailbox offsets from the typed peripheral map to avoid
    // duplicating register IDs across layers.
    use ast10x0_peripherals::smb_mbox::registers::register_offsets;

    /// UFM SMBus ownership policy register.
    ///
    /// Writes are source-filtered:
    /// - BMC source keeps bits in `0x23`
    /// - PCH/CPU source keeps bits in `0x03`
    ///
    /// This protocol register is not modeled as a named offset in the current
    /// peripheral register map.
    pub const UFM_SMBUS_OWNERSHIP: u8 = 0x0A;

    /// UFM command trigger register.
    ///
    /// Notifications on this register are translated to `ProvisionCmd` events.
    pub const UFM_CMD_TRIGGER_VALUE: u8 = register_offsets::UFM_CMD_TRIGGER_VALUE;

    /// ACM checkpoint register.
    ///
    /// PCH/CPU-domain writable checkpoint source; emits `WdtCheckpoint(Acm)`.
    pub const ACM_CHECKPOINT: u8 = register_offsets::ACM_CHECKPOINT;

    /// BIOS checkpoint register.
    ///
    /// PCH/CPU-domain writable checkpoint source; emits `WdtCheckpoint(Bios)`.
    pub const BIOS_CHECKPOINT: u8 = register_offsets::BIOS_CHECKPOINT;

    /// PCH update-intent register.
    ///
    /// Notification value is masked by `ProtocolConfig::pch_update_intent_mask`
    /// before writeback/event emission.
    pub const PCH_UPDATE_INTENT: u8 = register_offsets::PCH_UPDATE_INTENT;

    /// BMC update-intent register.
    ///
    /// BMC-domain writable intent source; emits `UpdateRequested`.
    pub const BMC_UPDATE_INTENT: u8 = register_offsets::BMC_UPDATE_INTENT;

    /// BMC checkpoint register.
    ///
    /// BMC-domain writable checkpoint source; emits `WdtCheckpoint(Bmc)`.
    pub const BMC_CHECKPOINT: u8 = register_offsets::BMC_CHECKPOINT;

    /// BMC secondary update-intent register.
    ///
    /// Supports seamless-update ACK behavior via
    /// `ProtocolConfig::{seamless_update_bit,seamless_update_ack_bit}`.
    ///
    /// This protocol register is not modeled as a named offset in the current
    /// peripheral register map.
    pub const BMC_UPDATE_INTENT2: u8 = 0x61;

    /// PCH secondary update-intent register.
    ///
    /// PCH/CPU-domain writable intent2 source; emits `UpdateIntent2Requested`.
    ///
    /// This protocol register is not modeled as a named offset in the current
    /// peripheral register map.
    pub const PCH_UPDATE_INTENT2: u8 = 0x62;

    /// BMC reset-communication request register.
    ///
    /// Value `1` generates `BmcResetCommRequested` and is written back to `0`.
    ///
    /// This protocol register is not modeled as a named offset in the current
    /// peripheral register map.
    pub const BMC_RESET_COMMUNICATION: u8 = 0x63;
}

/// Writable ownership bits for `regs::UFM_SMBUS_OWNERSHIP` when source is BMC.
pub const BMC_OWNERSHIP_MASK: u8 = 0x23;
/// Writable ownership bits for `regs::UFM_SMBUS_OWNERSHIP` when source is PCH/CPU.
pub const PCH_OWNERSHIP_MASK: u8 = 0x03;

/// Mailbox write source domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// Baseboard Management Controller source.
    Bmc,
    /// Host-side PCH/CPU source.
    PchCpu,
}

/// Protocol-layer validation errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    /// Register write is not allowed for the caller source domain.
    AccessDenied,
    /// Register is not supported by this protocol helper.
    UnsupportedRegister,
}

/// Checkpoint register source classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointKind {
    /// Checkpoint from the BMC domain.
    Bmc,
    /// Checkpoint from ACM.
    Acm,
    /// Checkpoint from BIOS.
    Bios,
}

/// Protocol event emitted from mailbox notifications.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolEvent {
    // Mirrors UPDATE_REQUESTED flow from the behavioral spec.
    /// Primary update-intent request.
    UpdateRequested { source: Source, value: u8 },
    /// Secondary update-intent request.
    UpdateIntent2Requested { source: Source, value: u8 },
    /// Watchdog checkpoint observation.
    WdtCheckpoint { kind: CheckpointKind, value: u8 },
    /// UFM provisioning command trigger.
    ProvisionCmd { trigger: u8 },
    /// BMC reset-communication request.
    BmcResetCommRequested,
}

/// Result of decoding a mailbox notification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationResult {
    // Optional register value that should be written back before returning.
    /// Optional value that should be written back to the same register.
    pub write_back: Option<u8>,
    /// Optional high-level event for the state machine.
    pub event: Option<ProtocolEvent>,
}

impl NotificationResult {
    /// Returns an empty result with no write-back and no event.
    pub const fn none() -> Self {
        Self {
            write_back: None,
            event: None,
        }
    }

    /// Returns a result carrying only an event.
    pub const fn with_event(event: ProtocolEvent) -> Self {
        Self {
            write_back: None,
            event: Some(event),
        }
    }

    /// Returns a result carrying a write-back and an optional event.
    pub const fn with_write_back(write_back: u8, event: Option<ProtocolEvent>) -> Self {
        Self {
            write_back: Some(write_back),
            event,
        }
    }
}

/// Runtime knobs for source-aware masking and intent2 ACK handling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolConfig {
    // PCH intent is masked before writeback/event emission.
    /// Mask applied to `regs::PCH_UPDATE_INTENT` before write-back/event emission.
    pub pch_update_intent_mask: u8,
    // Intent2 ack handling clears seamless_update_bit when ack bit is set.
    /// Bit cleared when `seamless_update_ack_bit` is present.
    pub seamless_update_bit: u8,
    /// Bit indicating an ACK for intent2 handling.
    pub seamless_update_ack_bit: u8,
}

impl Default for ProtocolConfig {
    /// Returns permissive default masks/bit positions used by this helper.
    fn default() -> Self {
        Self {
            pch_update_intent_mask: 0xFF,
            seamless_update_bit: 1 << 0,
            seamless_update_ack_bit: 1 << 1,
        }
    }
}

/// Stateless protocol helper for filtering writes and decoding notifications.
pub struct SmbusProtocol {
    cfg: ProtocolConfig,
}

impl SmbusProtocol {
    /// Creates a protocol helper from explicit configuration.
    pub const fn new(cfg: ProtocolConfig) -> Self {
        Self { cfg }
    }

    /// Applies source-aware write filtering and access checks for one register write.
    ///
    /// Returns the value that should be committed to the mailbox register.
    pub fn filter_write(&self, source: Source, addr: u8, value: u8) -> Result<u8, ProtocolError> {
        // Access model enforcement for mailbox writes, including ownership masking.
        match addr {
            regs::BMC_CHECKPOINT
            | regs::BMC_UPDATE_INTENT
            | regs::BMC_UPDATE_INTENT2
            | regs::BMC_RESET_COMMUNICATION => {
                if source != Source::Bmc {
                    return Err(ProtocolError::AccessDenied);
                }
                Ok(value)
            }
            regs::ACM_CHECKPOINT | regs::BIOS_CHECKPOINT | regs::PCH_UPDATE_INTENT | regs::PCH_UPDATE_INTENT2 => {
                if source != Source::PchCpu {
                    return Err(ProtocolError::AccessDenied);
                }
                Ok(value)
            }
            regs::UFM_SMBUS_OWNERSHIP => {
                let mask = if source == Source::Bmc {
                    BMC_OWNERSHIP_MASK
                } else {
                    PCH_OWNERSHIP_MASK
                };
                Ok(value & mask)
            }
            _ => Ok(value),
        }
    }

    /// Decodes a mailbox register notification into protocol actions.
    ///
    /// Given a register address and its current value, this method returns:
    /// - an optional `write_back` value that should be written to the same
    ///   register, and
    /// - an optional high-level `ProtocolEvent` for the caller's dispatcher.
    ///
    /// Typical behavior includes:
    /// - direct event emission (for example, update intents and checkpoints),
    /// - value masking before event emission (for `PCH_UPDATE_INTENT`),
    /// - and clear-on-ack/clear-on-handle write-backs for specific registers
    ///   (for example, intent2 and reset communication paths).
    ///
    /// Unknown or unsupported register notifications return
    /// `NotificationResult::none()`.
    ///
    /// # Example
    ///
    /// ```
    /// use ast10x0_pfr::smbus_protocol::{ProtocolConfig, SmbusProtocol, regs};
    ///
    /// let proto = SmbusProtocol::new(ProtocolConfig::default());
    /// let out = proto.on_notification(regs::BMC_UPDATE_INTENT, 0xAA);
    ///
    /// assert!(out.write_back.is_none());
    /// assert!(out.event.is_some());
    /// ```
    pub fn on_notification(&self, addr: u8, value: u8) -> NotificationResult {
        // Dispatcher-side translation from register notifications to protocol events.
        match addr {
            regs::UFM_CMD_TRIGGER_VALUE => NotificationResult::with_event(ProtocolEvent::ProvisionCmd {
                trigger: value,
            }),
            regs::BMC_UPDATE_INTENT => NotificationResult::with_event(ProtocolEvent::UpdateRequested {
                source: Source::Bmc,
                value,
            }),
            regs::PCH_UPDATE_INTENT => {
                let masked = value & self.cfg.pch_update_intent_mask;
                if masked == 0 {
                    NotificationResult::with_write_back(masked, None)
                } else {
                    NotificationResult::with_write_back(
                        masked,
                        Some(ProtocolEvent::UpdateRequested {
                            source: Source::PchCpu,
                            value: masked,
                        }),
                    )
                }
            }
            regs::BMC_UPDATE_INTENT2 => {
                let ack = (value & self.cfg.seamless_update_ack_bit) != 0;
                if ack {
                    let cleared = value & !self.cfg.seamless_update_bit;
                    NotificationResult::with_write_back(
                        cleared,
                        Some(ProtocolEvent::UpdateIntent2Requested {
                            source: Source::Bmc,
                            value: cleared,
                        }),
                    )
                } else {
                    NotificationResult::with_event(ProtocolEvent::UpdateIntent2Requested {
                        source: Source::Bmc,
                        value,
                    })
                }
            }
            regs::PCH_UPDATE_INTENT2 => {
                NotificationResult::with_event(ProtocolEvent::UpdateIntent2Requested {
                    source: Source::PchCpu,
                    value,
                })
            }
            regs::BMC_CHECKPOINT => NotificationResult::with_event(ProtocolEvent::WdtCheckpoint {
                kind: CheckpointKind::Bmc,
                value,
            }),
            regs::ACM_CHECKPOINT => NotificationResult::with_event(ProtocolEvent::WdtCheckpoint {
                kind: CheckpointKind::Acm,
                value,
            }),
            regs::BIOS_CHECKPOINT => NotificationResult::with_event(ProtocolEvent::WdtCheckpoint {
                kind: CheckpointKind::Bios,
                value,
            }),
            regs::BMC_RESET_COMMUNICATION => {
                if value == 1 {
                    NotificationResult::with_write_back(0, Some(ProtocolEvent::BmcResetCommRequested))
                } else {
                    NotificationResult::none()
                }
            }
            _ => NotificationResult::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_mask_is_source_aware() {
        let proto = SmbusProtocol::new(ProtocolConfig::default());

        assert_eq!(
            proto
                .filter_write(Source::Bmc, regs::UFM_SMBUS_OWNERSHIP, 0xFF)
                .unwrap(),
            BMC_OWNERSHIP_MASK
        );
        assert_eq!(
            proto
                .filter_write(Source::PchCpu, regs::UFM_SMBUS_OWNERSHIP, 0xFF)
                .unwrap(),
            PCH_OWNERSHIP_MASK
        );
    }

    #[test]
    fn pch_update_intent_is_masked_and_gated() {
        let proto = SmbusProtocol::new(ProtocolConfig {
            pch_update_intent_mask: 0x04,
            ..ProtocolConfig::default()
        });

        let out0 = proto.on_notification(regs::PCH_UPDATE_INTENT, 0x01);
        assert_eq!(out0.write_back, Some(0));
        assert_eq!(out0.event, None);

        let out1 = proto.on_notification(regs::PCH_UPDATE_INTENT, 0x05);
        assert_eq!(out1.write_back, Some(0x04));
        assert_eq!(
            out1.event,
            Some(ProtocolEvent::UpdateRequested {
                source: Source::PchCpu,
                value: 0x04,
            })
        );
    }

    #[test]
    fn bmc_intent2_ack_clears_seamless_update_bit() {
        let proto = SmbusProtocol::new(ProtocolConfig {
            seamless_update_bit: 1 << 3,
            seamless_update_ack_bit: 1 << 2,
            ..ProtocolConfig::default()
        });

        let out = proto.on_notification(regs::BMC_UPDATE_INTENT2, 0b1100);
        assert_eq!(out.write_back, Some(0b0100));
        assert_eq!(
            out.event,
            Some(ProtocolEvent::UpdateIntent2Requested {
                source: Source::Bmc,
                value: 0b0100,
            })
        );
    }

    #[test]
    fn source_restrictions_are_enforced() {
        let proto = SmbusProtocol::new(ProtocolConfig::default());

        assert_eq!(
            proto.filter_write(Source::PchCpu, regs::BMC_UPDATE_INTENT, 0xAA),
            Err(ProtocolError::AccessDenied)
        );
        assert_eq!(
            proto.filter_write(Source::Bmc, regs::PCH_UPDATE_INTENT, 0xAA),
            Err(ProtocolError::AccessDenied)
        );
    }
}
