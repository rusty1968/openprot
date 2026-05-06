# SPI Monitor: Boot Implementation

This document defines the concrete implementation steps for SPI monitor support during AST10x0 platform boot.

## Phase 1: Hold (ROT Exclusive Access)

### Objective
Route all flash access through ROT, prevent host access, reset flash to known state.

### Implementation Steps

```rust
pub fn bmc_boot_hold(monitor: &mut MonitorController, flash_ctrl: &mut FlashController) -> Result<()> {
    // Step 1: Configure monitor routing
    monitor.set_mux(MonitorInstance::Spim0, MuxSelect::RotControl)?;
    
    // Step 2: Reset flash via SPI (requires SPI1 control)
    flash_ctrl.reset_spi1_flash()?;
    
    // Step 3: If dual-flash, repeat for SPI1 flash1
    #[cfg(feature = "bmc_dual_flash")]
    {
        monitor.set_mux(MonitorInstance::Spim1, MuxSelect::RotControl)?;
        flash_ctrl.reset_spi1_flash_1()?;
    }
    
    Ok(())
}

pub fn pch_boot_hold(monitor: &mut MonitorController, flash_ctrl: &mut FlashController) -> Result<()> {
    // Step 1: Assert PCH hold (platform-specific GPIO)
    platform_hold_pch_reset()?;
    
    // Step 2: Configure monitor routing
    monitor.set_mux(MonitorInstance::Spim2, MuxSelect::RotControl)?;
    
    // Step 3: Reset flash via SPI (requires SPI2 control)
    flash_ctrl.reset_spi2_flash()?;
    
    // Step 4: If dual-flash, repeat
    #[cfg(feature = "pch_dual_flash")]
    {
        monitor.set_mux(MonitorInstance::Spim3, MuxSelect::RotControl)?;
        flash_ctrl.reset_spi2_flash_1()?;
    }
    
    Ok(())
}
```

### Monitor State After Hold

| Property | Value |
|----------|-------|
| Mux | RotControl |
| Flash Access | ROT only |
| Host Access | Blocked |
| Policy | Not loaded |
| Monitoring | Passive |

---

## Phase 2: Policy Configuration

### Objective
Load address privilege rules from provisioned data (PFM), configure monitor address tables.

### Implementation Steps

```rust
pub fn configure_monitor_policy(
    monitor: &mut MonitorController,
    flash_reader: &FlashReader,
    pfm_offset: u32,
) -> Result<()> {
    // Step 1: Read PFM metadata from provisioned flash
    let pfm_metadata = flash_reader.read_pfm_metadata(pfm_offset)?;
    
    // Step 2: Extract region definitions
    let regions = pfm_metadata.extract_regions()?;
    
    // Step 3: For each region, configure monitor address privilege table
    for region in regions {
        monitor.set_address_privilege(
            MonitorInstance::Spim0,  // or appropriate instance
            AddressRegion {
                start: region.start_address,
                end: region.end_address,
                privilege: region.privilege_mode,
                read_policy: region.read_allowed,
                write_policy: region.write_allowed,
            },
        )?;
    }
    
    // Step 4: (Future) Validate policy against PFM
    // verify_monitor_policy(monitor, &pfm_metadata)?;
    
    Ok(())
}

// Helper: Parse typical region types from PFM
pub fn parse_bmc_regions(pfm: &ProvisionalFirmwareManifest) -> Result<Vec<AddressRegion>> {
    let mut regions = Vec::new();
    
    // Code region: read-only
    regions.push(AddressRegion {
        start: pfm.code_start,
        end: pfm.code_end,
        privilege: Privilege::ReadOnly,
        read_allowed: true,
        write_allowed: false,
    });
    
    // Active firmware: write-protected (update window only)
    regions.push(AddressRegion {
        start: pfm.active_fw_start,
        end: pfm.active_fw_end,
        privilege: Privilege::Protected,
        read_allowed: true,
        write_allowed: false,  // Only during update flow
    });
    
    // Recovery: read-only
    regions.push(AddressRegion {
        start: pfm.recovery_start,
        end: pfm.recovery_end,
        privilege: Privilege::ReadOnly,
        read_allowed: true,
        write_allowed: false,
    });
    
    // Data area: read-write
    regions.push(AddressRegion {
        start: pfm.data_start,
        end: pfm.data_end,
        privilege: Privilege::ReadWrite,
        read_allowed: true,
        write_allowed: true,
    });
    
    Ok(regions)
}
```

### Monitor State After Configuration

| Property | Value |
|----------|-------|
| Mux | RotControl (unchanged) |
| Policy | Loaded in address tables |
| Region Count | 4+ (code, active, recovery, data) |
| Enforcement | Ready (not yet active) |

---

## Phase 3: Release (Host Access Enabled)

### Objective
Transfer flash control back to host, activate monitor enforcement.

### Implementation Steps

```rust
pub fn bmc_boot_release(monitor: &mut MonitorController) -> Result<()> {
    // Step 1: Switch monitor mux to host (BMC/PCH)
    monitor.set_mux(MonitorInstance::Spim0, MuxSelect::HostControl)?;
    
    // Step 2: Soft-reset monitor (clears status, preserves policy)
    monitor.soft_reset(MonitorInstance::Spim0)?;
    
    // Step 3: If dual-flash, repeat
    #[cfg(feature = "bmc_dual_flash")]
    {
        monitor.set_mux(MonitorInstance::Spim1, MuxSelect::HostControl)?;
        monitor.soft_reset(MonitorInstance::Spim1)?;
    }
    
    // Step 4: Verify release (defensive check)
    monitor.assert_mux(MonitorInstance::Spim0, MuxSelect::HostControl)?;
    
    Ok(())
}

pub fn pch_boot_release(monitor: &mut MonitorController) -> Result<()> {
    // Step 1: Switch monitor mux to host
    monitor.set_mux(MonitorInstance::Spim2, MuxSelect::HostControl)?;
    
    // Step 2: Soft-reset monitor
    monitor.soft_reset(MonitorInstance::Spim2)?;
    
    // Step 3: If dual-flash, repeat
    #[cfg(feature = "pch_dual_flash")]
    {
        monitor.set_mux(MonitorInstance::Spim3, MuxSelect::HostControl)?;
        monitor.soft_reset(MonitorInstance::Spim3)?;
    }
    
    // Step 4: Release PCH hold
    platform_release_pch_hold()?;
    
    Ok(())
}
```

### Monitor State After Release

| Property | Value |
|----------|-------|
| Mux | HostControl |
| Flash Access | Host (BMC/PCH) |
| ROT Access | Blocked |
| Policy | Active enforcement |
| Monitoring | Blocking violations |

---

## Phase 4: Runtime Monitoring (Operational)

### Objective
Monitor remains passive enforcement boundary; no policy changes.

### Expected Behavior

```rust
pub fn monitor_runtime_state(monitor: &MonitorController, instance: MonitorInstance) -> Result<MonitorStatus> {
    let status = monitor.read_status(instance)?;
    
    // Typical expected state:
    assert_eq!(status.mux, MuxSelect::HostControl);
    assert_eq!(status.policy_locked, true);
    assert_eq!(status.enforcement_active, true);
    
    // Check for policy violations (if any occurred)
    if status.violation_count > 0 {
        log_violation_events(monitor, instance, status.violation_count)?;
    }
    
    Ok(status)
}

// Expected API for violation reporting (via ISR callback)
pub fn handle_monitor_violation(instance: MonitorInstance, violation: MonitorViolation) {
    match violation {
        MonitorViolation::UnauthorizedWrite { address, command } => {
            log_error!(
                "Monitor {}: Unauthorized write at 0x{:x}, command 0x{:02x}",
                instance as u32, address, command
            );
        },
        MonitorViolation::UnauthorizedRead { address } => {
            log_error!("Monitor {}: Unauthorized read at 0x{:x}", instance as u32, address);
        },
    }
}
```

### No Dynamic Policy Changes

**Key constraint:** Once runtime begins, policy configuration is **immutable**.

```rust
// This should NOT be allowed in runtime:
// monitor.set_address_privilege(...)?;  // ❌ COMPILE ERROR or PANIC

// Policy changes only occur during explicit lifecycle transitions:
// - Authenticated firmware update window (separate state)
// - Recovery/reflash (requires reset)
// - Manufacturing/provisioning (only at boot)
```

---

## Boot Call Sequence

### Typical Startup Flow

```
Platform::startup()
    ├─ bmc_boot_hold(&monitor, &flash_ctrl)
    │   ├─ set_mux(Spim0, RotControl)
    │   └─ reset_spi1_flash()
    │
    ├─ pch_boot_hold(&monitor, &flash_ctrl)          [if PCH present]
    │   ├─ platform_hold_pch_reset()
    │   ├─ set_mux(Spim2, RotControl)
    │   └─ reset_spi2_flash()
    │
    ├─ configure_monitor_policy(&monitor, &flash_reader, PFM_OFFSET)
    │   ├─ read_pfm_metadata()
    │   ├─ extract_regions()
    │   └─ set_address_privilege() [for each region]
    │
    ├─ bmc_boot_release(&monitor)
    │   ├─ set_mux(Spim0, HostControl)
    │   └─ soft_reset(Spim0)
    │
    ├─ pch_boot_release(&monitor)                    [if PCH present]
    │   ├─ set_mux(Spim2, HostControl)
    │   ├─ soft_reset(Spim2)
    │   └─ platform_release_pch_hold()
    │
    ├─ verify_boot_state(&monitor)
    │
    └─ firmware_ready()
        └─ monitor_runtime_state() [periodic monitoring]
```

---

## Error Handling

### Failure Recovery

```rust
pub fn bmc_boot_hold_with_recovery(
    monitor: &mut MonitorController,
    flash_ctrl: &mut FlashController,
    max_retries: u32,
) -> Result<()> {
    for attempt in 0..max_retries {
        match bmc_boot_hold(monitor, flash_ctrl) {
            Ok(()) => {
                log_info!("BMC boot hold succeeded");
                return Ok(());
            }
            Err(e) => {
                log_error!("BMC boot hold attempt {} failed: {}", attempt, e);
                if attempt < max_retries - 1 {
                    // Reset monitor and retry
                    monitor.hardware_reset(MonitorInstance::Spim0)?;
                    platform_delay_ms(10);
                }
            }
        }
    }
    Err(BootError::MaxRetriesExceeded)
}

pub fn verify_boot_sequence(monitor: &MonitorController) -> Result<()> {
    // Verify BMC hold
    assert_eq!(
        monitor.read_mux(MonitorInstance::Spim0)?,
        MuxSelect::HostControl,
        "BMC monitor mux not in host control"
    );
    
    // Verify policy loaded
    let region_count = monitor.read_region_count(MonitorInstance::Spim0)?;
    assert!(region_count > 0, "No policy regions loaded");
    
    // Verify lock engaged (if supported)
    if monitor.supports_policy_lock() {
        assert_eq!(
            monitor.read_policy_locked(MonitorInstance::Spim0)?,
            true,
            "Policy not locked"
        );
    }
    
    Ok(())
}
```

---

## Configuration Flags

### Build-Time Configuration

```toml
# Cargo.toml or feature flags
[features]
bmc_single_flash = []       # Default: BMC has one flash
bmc_dual_flash = []         # BMC has dual flash (SPI1@0 + SPI1@1)
pch_single_flash = []       # Default: PCH has one flash
pch_dual_flash = []         # PCH has dual flash (SPI2@0 + SPI2@1)
monitor_policy_lock = []    # Monitor supports policy write-lock
monitor_enforcement = []    # Monitor enforces address privilege
monitor_logging = []        # Monitor logs violations
```

### Runtime Configuration

```rust
pub struct BootConfig {
    pub enable_bmc_hold: bool,
    pub enable_pch_hold: bool,
    pub enable_policy_config: bool,
    pub enable_release: bool,
    pub enable_verification: bool,
    pub policy_source: PolicySource,
}

pub enum PolicySource {
    Provisioned(u32),  // Offset in provisioned flash
    Hardcoded(&'static [u8]),  // For test/mfg
}
```

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bmc_boot_hold_sets_mux() {
        let mut monitor = MockMonitor::new();
        let mut flash = MockFlashController::new();
        
        bmc_boot_hold(&mut monitor, &mut flash).unwrap();
        
        assert_eq!(monitor.mux_state(Spim0), MuxSelect::RotControl);
    }
    
    #[test]
    fn test_policy_regions_non_overlapping() {
        let pfm = ProvisionalFirmwareManifest::default();
        let regions = parse_bmc_regions(&pfm).unwrap();
        
        assert!(regions_non_overlapping(&regions));
    }
    
    #[test]
    fn test_boot_release_unlocks_host_access() {
        let mut monitor = MockMonitor::new();
        monitor.set_mux(Spim0, MuxSelect::RotControl).unwrap();
        
        bmc_boot_release(&mut monitor).unwrap();
        
        assert_eq!(monitor.mux_state(Spim0), MuxSelect::HostControl);
    }
}
```

### Integration Tests

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    
    #[test]
    #[ignore]  // Requires hardware
    fn test_full_boot_sequence() {
        let mut monitor = RealMonitor::new();
        let mut flash = RealFlashController::new();
        
        bmc_boot_hold(&mut monitor, &mut flash).unwrap();
        configure_monitor_policy(&mut monitor, &flash, 0x1000).unwrap();
        bmc_boot_release(&mut monitor).unwrap();
        
        verify_boot_sequence(&monitor).unwrap();
    }
}
```

---

## Validation Checklist

Before declaring boot support complete:

- [ ] `bmc_boot_hold()` switches mux to ROT and resets flash
- [ ] `pch_boot_hold()` (if applicable) holds PCH and resets flash
- [ ] `configure_monitor_policy()` reads PFM and loads address regions
- [ ] Policy regions are non-overlapping and cover expected address space
- [ ] `bmc_boot_release()` switches mux to host and soft-resets monitor
- [ ] `pch_boot_release()` (if applicable) releases PCH hold
- [ ] Runtime state confirms mux in host control and policy active
- [ ] Policy violations are logged/reported via ISR callback
- [ ] No policy changes possible in runtime (type/API prevents it)
- [ ] Readback verification passes after each phase
- [ ] Boot sequence completes within timing budget (e.g., <100ms)
- [ ] Hardware validation passes on silicon (policy lock, routing)

---

## References

- [Overview and Usage Model](overview-and-usage-model.md)
- [Boot Sequence Usage Model](boot-sequence-usage-model.md)
- [Review Against aspeed-zephyr-project](review-against-aspeed-zephyr.md)

See aspeed-zephyr-project source:
- Hold/release: `lib/hrot_hal/gpio/gpio_aspeed.c#L85`
- Policy config: `apps/aspeed-pfr/src/intel_pfr/intel_pfr_spi_filtering.c#L32`
- Mux control: `lib/hrot_hal/gpio/gpio_aspeed.c#L272`
