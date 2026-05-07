# Gaps Assessment: Post-Phase-2 Update

Date: 2026-05-07 (Updated after SmcTopology + board descriptor implementation)

## Overview

This document reassesses the parity gaps from `gaps.md` in light of Phase 1 (SmcTopology enum) and Phase 2 (board descriptor topology mapping) work. Some gaps have been addressed structurally; others remain unchanged.

## Summary of Changes from Phase 1 + Phase 2

**Phase 1:** Added `SmcTopology` enum:
```rust
pub enum SmcTopology {
    BootSpi { master_idx: u8 },      // FMC / boot firmware
    HostSpi { master_idx: u8 },      // SPI1 / host BMC access
    NormalSpi { master_idx: u8 },    // SPI2 / normal user SPI
}
```

**Phase 2:** Board descriptor now maps controller identity to topology:
```rust
match self.controller {
    SmcController::Fmc => SmcTopology::BootSpi { master_idx: 0 },
    SmcController::Spi1 => SmcTopology::HostSpi { master_idx: 0 },
    SmcController::Spi2 => SmcTopology::NormalSpi { master_idx: 2 },
}
```

## Gap Reassessment

### ✅ ADDRESSED: B11 (Master-index awareness)

**Original Gap:**
> No master-index awareness. aspeed-rust's `SpiConfig.master_idx` differentiates BootSpi/HostSpi/NormalSpi and influences decode-range pre-init, timing calibration skip, and SPIM bracketing.

**Current Status:** ✅ PARTIALLY CLOSED
- `SmcTopology` now encodes `master_idx` directly as a struct field
- The topology enum variant itself serves the semantic role (BootSpi vs HostSpi vs NormalSpi)
- Board descriptor is single source of truth for topology mapping
- **Remaining work (Phase 3+):** Consume topology in `controller.rs` to gate behaviors (decode-range sizing, calibration skip, SPIM bracketing per variant)

**Implication for D10 (Master-index / shared-flash topology):**
- The type-system foundation is now in place
- Phase 3 will wire behavior gating per topology variant
- Optional per-transaction bracketing (D11) remains deferred but is now architecturally clearer

---

### 🔄 UNCHANGED: A. Capability-by-capability matrix gaps

The following gaps from the matrix remain open:

**Silicon-feature gaps (aspeed-rust has, OpenPRoT missing):**
- **B1 (AHB-read mode):** No `spi_nor_read_init` equivalent; normal-read mode fixed to POR defaults
- **B2 (DMA read):** Exists in HAL but not wired through device facade or backend
- **B3 (DMA write):** Absent entirely
- **B4 (Dynamic segment re-init):** Segments programmed once, never revisited
- **B5 (Timing calibration):** Only single divider lookup; no HCLK sweep
- **B6 (SCU/HCLK introspection):** Hard-coded `sysclk_mhz = 200`
- **B7 (Dummy-cycle register support):** No callee-side encoding; must inline into tx_payload
- **B8 (SR2/SR3 access):** Only SR1 read; QE bit unreachable
- **B9 (Software reset):** No facade method; opcodes only in SPIPF allow-list
- **B12 (JEDEC discovery):** Limited to three Winbond parts; Macronix rejects

**Topology/arbitration gaps:**
- **B10 (Per-transaction SPIM bracketing):** Deliberately deferred; OpenPRoT lock-once model precludes this
- **D11 (Optional per-transaction SPIM bracketing):** Requires non-default SpiMonitor variant; architectural decision needed

**Trait integration gaps:**
- **B13 (embedded-hal/proposed_traits):** No `SpiBus`, `SpiDevice`, `BlockDevice` implementations

---

### ✅ NEW CAPABILITY: Topology-aware behavior gating (Phase 3 foundation)

**Added in Phase 1+2:**
```
SmcConfig now carries:
  - controller_id: Hardware identity (register base, IRQ, window address)
  - topology: Semantic role (BootSpi/HostSpi/NormalSpi + master_idx)
```

This enables Phase 3+ logic:
```rust
match config.topology {
    SmcTopology::BootSpi { master_idx } => { 
        // Decode-range sizing: full 256M
        // Calibration: run fully
        // SPIM bracketing: unnecessary (direct path)
    },
    SmcTopology::HostSpi { master_idx } => {
        // Decode-range sizing: 256M (single master assumption)
        // Calibration: skip if master_idx != 0
        // SPIM bracketing: required for shared bus
    },
    SmcTopology::NormalSpi { master_idx } => {
        // Decode-range sizing: per master_idx (e.g., 256M when idx=2)
        // Calibration: skip if master_idx != 0
        // SPIM bracketing: required for shared bus
    }
}
```

**Prerequisite for closing B1, B4, B5, B10, B11:**
- Phase 3 must consume `config.topology` in `Smc<Ready>::init` to gate above behaviors
- B1 requires `spi_nor_read_init` wired to `controller.rs:write_cs0_ctrl`/`write_cs1_ctrl`
- B4 requires decode-range re-init logic keyed on topology
- B5 requires conditional timing calibration branch on topology
- B10+B11 require optional SPIM bracketing loop around transceive operations

---

## Revised Priority for Parity Closure

Based on the topology foundation now in place:

### Tier 1 (Unlock via Phase 3 topology gating)
- **D7 (SCU/HCLK):** Read during board init; pass into `FlashConfig`; use in `controller.rs` divider calc
- **D3 (AHB-read mode):** `spi_nor_read_init` in `Smc<Ready>::init`; gate on topology to control segment behavior
- **D4 (SR2/SR3):** Add to `SpiNorFlashDevice`; no dependency on topology but needed for safe quad-enable
- **D5 (Software reset):** Add `reset()` to `SpiNorFlashDevice`; opcodes already allowed
- **D9 (Timing calibration):** Conditional sweep in `Smc<Ready>::init` gated on `SmcTopology::{BootSpi, HostSpi, NormalSpi}` and `master_idx`

### Tier 2 (DMA, wider device support)
- **D1 (DMA read):** Wire `dma_read` through device facade; use for >page payloads on AHB-read-capable setup
- **D2 (DMA write):** Port aspeed-rust's write_dma; plumb through program() for throughput
- **D6 (JEDEC auto-discovery):** Expand `cfg_from_jedec` to decode capacity generically; add Macronix + ISSI tables

### Tier 3 (Trait integration, optional bracketing)
- **D14 (embedded-hal traits):** Adapter layer; can be independent of topology work
- **D11 (Optional per-transaction bracketing):** Requires explicit board descriptor opt-in and non-default `SpiMonitor` typestate; defer until topology gating is stable in Phase 3

---

## Impact on D10 and D11 Decisions

**D10 (Master-index / shared-flash topology):**
- ✅ Foundation laid: `SmcTopology` encodes role + master_idx
- ⏳ Remaining: Phase 3 must gate decode-range, calibration, and optional bracketing per topology variant

**D11 (Optional per-transaction SPIM bracketing):**
- ⏳ Architectural gate: Requires moving `SpiMonitor` from `Locked` typestate to a variant that remains `Configured` and permits repeated `apply_routing` calls per transaction
- ⏳ Board descriptor gate: Needs explicit flag to opt into bracketing (e.g., `SpiMonitorMode::LockOnce` vs `SpiMonitorMode::Configured`)
- ⏳ Implementation gate: Deferred until Phase 3 topology gating is proven stable
- ⏳ Risk: Weakens defense-in-depth (SPIPF lock-once advantage disappears if mode is configurable)

**Recommendation for D11:**
- Phase 3: Consume topology to gate per-variant behaviors (closed-form gating per role)
- Phase 4+: If shared-flash use cases materialize, revisit D11 as an opt-in feature flag
- Status quo (lock-once) remains the default and recommended posture for BMC deployments

---

## Gaps Closed by Phase 1+2 (Structural)

| Gap ID | Description | Status |
|--------|-------------|--------|
| B11 | Master-index awareness | ✅ Type system now models `SmcTopology { master_idx }` |
| D10 | Master-index gating prerequisite | ✅ Board descriptor maps topology; Phase 3 gates behaviors |

---

## Gaps Remaining (No Change from gaps.md)

| Gap ID | Category | Status |
|--------|----------|--------|
| B1 | AHB-read mode programming | ⏳ Phase 3 (needs topology gating + spi_nor_read_init) |
| B2 | DMA read path enabled | ⏳ Phase 3+ (needs wiring through device facade) |
| B3 | DMA write | ⏳ Phase 3+ (needs port from aspeed-rust) |
| B4 | Dynamic segment re-init | ⏳ Phase 3 (needs topology-gated decode_range_reinit) |
| B5 | Timing calibration | ⏳ Phase 3+ (needs topology-gated HCLK sweep) |
| B6 | SCU/HCLK introspection | ⏳ Phase 3 (read during board init; pass into config) |
| B7 | Dummy-cycle register encoding | ⏳ Phase 3 (needed for AHB-read mode, B1) |
| B8 | SR2/SR3 access | ⏳ Phase 3+ (independent of topology) |
| B9 | Software reset facade | ⏳ Phase 3+ (independent of topology) |
| B10 | Per-transaction SPIM bracketing | ⏳ Phase 4+ (architectural; lock-once by design) |
| B12 | JEDEC discovery beyond Winbond | ⏳ Phase 3+ (independent of topology) |
| B13 | embedded-hal trait implementations | ⏳ Phase 3+ (independent of topology) |

---

## Next Steps

1. **Phase 3 (Topology-Aware Behavior Gating):**
   - Consume `config.topology` in `Smc<Ready>::init` to gate decode-range, calibration, optional bracketing
   - Implement B1 (AHB-read mode), B4 (dynamic segment re-init), B5 (timing calibration), B9 (software reset)
   - Add enhanced rustdocs explaining per-variant behavior differences

2. **Phase 4+ (Device Capability Expansion):**
   - DMA wiring (B2, B3)
   - JEDEC discovery widening (B12)
   - Trait integration (B13)

3. **Post-Phase-4 (Shared-Flash Topologies):**
   - Revisit D11 and B10 if BMC products require host/BMC flash arbitration
   - Requires explicit opt-in via board descriptor and architectural review of SPIPF policy state machine

---

## Conclusion

The Phase 1+2 foundation (SmcTopology + board descriptor mapping) closes the type-system gap for master-index modeling (B11, D10) and creates the infrastructure for Phase 3 topology-aware behavior gating. The remaining gaps are primarily implementation (wiring DMA, calibration, AHB-read mode) and feature expansion (JEDEC discovery, trait integration), not architectural.

The lock-once SPIPF model remains the correct default for BMC security posture. Optional per-transaction bracketing (D11, B10) is deferred pending proof of stable topology gating and explicit business justification for shared-flash topologies.
