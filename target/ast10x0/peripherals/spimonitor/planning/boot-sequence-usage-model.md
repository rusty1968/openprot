# SPI Monitor Boot Sequence Usage Model

This document describes the actual boot-time usage of SPI monitor blocks during the AST10x0 platform bring-up, derived from aspeed-zephyr-project implementation patterns.

## Boot Sequence Overview

The SPI monitor's primary boot-time role is to **control access** to flash devices while the Root of Trust (ROT) performs critical initialization. The monitor acts as a **routing gateway** that switches flash ownership between ROT and host processors.

```
[Boot Start]
         ↓
    [Hold Phase] ← Monitor mux switches to ROT
         ↓       ← Flash is reset
         ↓       ← Host is prevented from accessing flash
         ↓
    [Policy Configuration Phase] ← Address filtering rules loaded
         ↓                         ← PFM (Provisioned FVM) data read
         ↓
    [Release Phase] ← Monitor mux switches to Host (BMC/PCH)
         ↓           ← Monitor is soft-reset
         ↓
   [Runtime]  ← Monitor enforces policy (monitoring only)
```

---

## Phase 1: Hold Phase

**Purpose:** Take exclusive control of flash, reset it, and prevent host access.

**Participants:**
- Monitor (SPIPF1..4)
- SPI Controller (SPI1/SPI2)
- Flash device (via SPI)

### Hold Operation for BMC (Single Flash)

**Code reference:** `lib/hrot_hal/gpio/gpio_aspeed.c#L85` (`BMCBootHold()`)

```
Step 1: Switch monitor mux to ROT
  switch_spim_mux(BMC_SPI_MONITOR, SPIM_EXT_MUX_ROT)
  
  Hardware effect:
    SPIPF1.routing_control |= SPIM_EXT_MUX_SEL_0  (or SEL_1, depending on CONFIG)
    → Flash now responds ONLY to ROT's SPI1 commands
    → BMC/PCH SPI1 is disconnected from flash
    
Step 2: Get flash device handle
  flash_dev = device_get_binding("spi1@0")
  
  Driver effect:
    Returns Zephyr device handle for SPI1 flash 0
    
Step 3: Hardware reset flash via SPI
  spi_nor_rst_by_cmd(flash_dev)
  
  Hardware effect:
    Sends RESET or software-reset command to flash
    Flash state returned to known condition
    
Step 4: (If dual flash) Repeat for second flash
  switch_spim_mux(BMC_SPI_MONITOR_2, SPIM_EXT_MUX_ROT)
  flash_dev = device_get_binding("spi1@1")
  spi_nor_rst_by_cmd(flash_dev)
```

**Monitor state after Hold:**
- Mux: `SPIM_EXT_MUX_ROT` (ROT controls)
- Policy: Not yet loaded
- Enforcement: Passive (no address filtering active)

### Hold Operation for PCH (Dual Controller)

**Code reference:** `lib/hrot_hal/gpio/gpio_aspeed.c#L120` (`PCHBootHold()`)

Same pattern but for SPI2:
```
switch_spim_mux(PCH_SPI_MONITOR, SPIM_EXT_MUX_ROT)
flash_dev = device_get_binding("spi2@0")
spi_nor_rst_by_cmd(flash_dev)
```

**Key observation:** Monitor and flash controller sequencing are **operationally coupled** in a single function, not separate.

---

## Phase 2: Policy Configuration

**Purpose:** Load address-privilege rules from provisioned data (PFM).

**Participants:**
- Monitor address privilege tables
- Policy engine (aspeed-pfr firmware)
- Provisioned PFM data (on BMC/PCH flash)

### Policy Load Sequence

**Code reference:** `apps/aspeed-pfr/src/intel_pfr/intel_pfr_spi_filtering.c#L32`

```
Step 1: Read PFM metadata from provisioned flash
  get_provision_data_in_flash(BMC_ACTIVE_PFM_OFFSET, ...)
  
  Retrieves:
    - Address of PFM data structure
    - Region definitions (start/end addresses for code, data, update areas)
    - Privilege levels (read-only, write-protected, locked)
    
Step 2: Parse PFM region records
  For each region in PFM:
    region_start_address = extract_region_start()
    region_end_address   = extract_region_end()
    privilege_mode       = extract_privilege()
    
Step 3: Configure monitor address privilege tables
  Set_SPI_Filter_RW_Region(
    dev_name=BMC_SPI_MONITOR,
    rw_select=READ,           // or WRITE
    op=BLOCK,                 // or ALLOW
    addr=region_start,
    len=region_end - region_start
  )
  
  Driver effect:
    SPIPF1.addr_priv_table[i] = {
      .start_addr = region_start,
      .end_addr   = region_end,
      .priv_mode  = BLOCK_READ | ALLOW_WRITE  (example)
    }
    
Step 4: (Optional) Lock policy tables
  [Lock mechanism not clearly visible in current code]
  SPIPF1.ctrl |= TABLE_WRITE_LOCK
```

### Policy Regions

Typical configuration from PFM:

| Region | Privilege | Notes |
|--------|-----------|-------|
| Code (0x0-0x100K) | Read-only | Host reads firmware, cannot write |
| Active FW (0x100K-0x400K) | Protected | Updates only during authenticated window |
| Recovery (0x400K-0x600K) | Read-only | Never writable by host |
| Data (0x600K-0x800K) | Read-write | Host data area |

**Monitor behavior:** Any SPI command to write outside allowed regions is **blocked** by hardware.

---

## Phase 3: Release Phase

**Purpose:** Switch flash back to host control and enable monitoring.

**Participants:**
- Monitor (mux, enable)
- Host controllers (BMC/PCH SPI1/2)

### Release Operation

**Code reference:** `lib/hrot_hal/gpio/gpio_aspeed.c#L165` (`BMCBootRelease()`)

```
Step 1: Switch monitor mux to Host (BMC/PCH)
  switch_spim_mux(BMC_SPI_MONITOR, SPIM_EXT_MUX_BMC_PCH)
  
  Hardware effect:
    SPIPF1.routing_control |= SPIM_EXT_MUX_SEL_1  (or SEL_0, opposite of hold)
    → Flash now responds to BMC's SPI1 commands
    → ROT's exclusive access is released
    
Step 2: Soft reset monitor
  aspeed_spi_monitor_sw_rst(dev_m)
  
  Hardware effect:
    SPIPF1.ctrl |= SW_RESET
    Monitor clears internal transaction buffers
    Status flags reset
    (Policy configuration is preserved or reloaded)
    
Step 3: (If dual flash) Repeat
  switch_spim_mux(BMC_SPI_MONITOR_2, SPIM_EXT_MUX_BMC_PCH)
  aspeed_spi_monitor_sw_rst(dev_m)
```

**Monitor state after Release:**
- Mux: `SPIM_EXT_MUX_BMC_PCH` (Host controls)
- Policy: Loaded and enforcing
- Enforcement: Active (address filtering blocks violating commands)

---

## Phase 4: Runtime Operation

**Purpose:** Monitor remains active as read-only enforcement boundary.

### Runtime Guarantees

Once released, the monitor provides:

1. **Address Filtering**
   - Commands to protected regions are **blocked in hardware**
   - ROT can read proof of enforcement by checking transaction logs
   
2. **Routing Lock**
   - Mux cannot be changed back to ROT without privileged reset
   - (Depends on current lock implementation visibility)
   
3. **Observability**
   - Monitor captures blocked transactions in status/log registers
   - ISR callback notifies firmware of policy violations

### No Dynamic Policy Changes

**Key architectural property:** 
- Policy configuration is **static after boot**
- Runtime code should **not** reconfigure address filters
- Any "policy update" flow must be explicit lifecycle transition, not ad-hoc runtime change

---

## Data Flow Diagram

```
┌─────────────────────────────────────────────────────────────┐
│ BOOT SEQUENCE: Hold → Configure → Release → Monitor        │
└─────────────────────────────────────────────────────────────┘

┌──────────┐         ┌────────────────┐         ┌──────────┐
│ HOLD     │ ─────→  │  CONFIGURE     │ ─────→  │ RELEASE  │
│          │         │                │         │          │
│ Mux: ROT │         │ Read PFM       │         │ Mux: BMC │
│ Flash: ✓ │         │ Load policy    │         │ Monitor: ✓
│ Host: ✗  │         │ Address table  │         │ Host: ✓  │
└──────────┘         └────────────────┘         └──────────┘
     │                      │                         │
     └──────────────────────┴─────────────────────────┘
                      ↓
            ┌─────────────────────┐
            │ RUNTIME MONITORING  │
            │                     │
            │ Monitor active:     │
            │ - Blocks violating  │
            │   commands          │
            │ - Logs violations   │
            │ - Reports to ROT    │
            └─────────────────────┘
```

---

## Code Locations Reference

| Component | Location | Function |
|-----------|----------|----------|
| Hold sequence | lib/hrot_hal/gpio/gpio_aspeed.c#L85 | `BMCBootHold()` |
| Release sequence | lib/hrot_hal/gpio/gpio_aspeed.c#L165 | `BMCBootRelease()` |
| Mux control | lib/hrot_hal/gpio/gpio_aspeed.c#L272 | `switch_spim_mux()` |
| Policy config | apps/aspeed-pfr/src/intel_pfr/intel_pfr_spi_filtering.c#L32 | `intel_pfr_spi_filtering_config()` |
| Address region API | apps/aspeed-pfr/src/spi_filter/spim_util.c#L30 | `Set_SPI_Filter_RW_Region()` |
| State machine | apps/aspeed-pfr/src/AspeedStateMachine/AspeedStateMachine.c#L264 | Startup sequence |

---

## Typical Boot Timeline

```
TIME     ACTION                          MONITOR STATE
────────────────────────────────────────────────────────
T0       BMCBootHold() called            Mux → ROT
T0+1ms   Flash reset via SPI1            Routing: active
T0+5ms   PolicyConfig() called           Reading PFM
T0+10ms  Address tables loaded           Policy: configured
T0+15ms  BMCBootRelease() called         Mux → BMC
T0+16ms  Monitor soft-reset              Status: cleared
T0+20ms  State machine continues         Monitor active
T0+25ms  Firmware ready                  Normal operation
```

---

## Key Invariants

1. **Exclusive control phases**
   - During Hold: ROT can access flash, Host cannot
   - During Release: Host regains access, ROT loses exclusive path
   - Transition is atomic (mux switch is single operation)

2. **Policy immutability**
   - Once configured, address privilege tables should not change
   - If reboot/reset occurs, policy must be reloaded from provisioned data

3. **Soft reset preservation**
   - Monitor soft-reset clears status/logs but preserves policy tables
   - (Assumption based on typical hardware design; verify with ASPEED)

4. **No feedback loop**
   - Current code does not read back and verify policy was applied
   - Adding verification step would increase robustness

---

## Comparison: Overview Model vs. Boot Reality

| Aspect | Overview Model | Actual Boot |
|--------|---|---|
| **Lifecycle** | configure → validate → lock → operational | hold → configure → release → monitor |
| **Primary feature** | Command table enforcement | Address region + mux routing |
| **Integration** | Separate from controller | Coupled with flash sequencing |
| **Lock semantics** | Explicit write-disable bits | Assumed but not verified |
| **Configuration timing** | After register foundation | From provisioned PFM data |
| **Validation step** | Readback verification | Not currently implemented |

---

## Potential Improvements

1. **Add readback verification**
   ```c
   // After policy load:
   verify_monitor_policy(BMC_SPI_MONITOR);
   assert_address_table_matches_pfm();
   ```

2. **Explicit lock assertion**
   ```c
   lock_monitor_policy(BMC_SPI_MONITOR);
   assert_policy_locked(BMC_SPI_MONITOR);
   ```

3. **Defensive mux switching**
   ```c
   prev_mux = read_monitor_mux(BMC_SPI_MONITOR);
   switch_spim_mux(BMC_SPI_MONITOR, new_mux);
   assert_mux_changed(BMC_SPI_MONITOR, new_mux);
   ```

4. **Boot phase logging**
   ```c
   log_monitor_state("hold_start", BMC_SPI_MONITOR);
   log_monitor_state("policy_loaded", BMC_SPI_MONITOR);
   log_monitor_state("release_done", BMC_SPI_MONITOR);
   ```

---

## Summary

The SPI monitor's boot-time usage follows a **clear three-phase model**:

1. **Hold**: Mux to ROT, reset flash, prevent host access
2. **Configure**: Load address privilege policy from provisioned data
3. **Release**: Mux to host, reset monitor, enable enforcement

This aligns with the overview document's core principle of "configure once, lock, operate" but the actual integration is **tightly coupled with flash controller sequencing**, not a standalone configuration step.

For a fully auditable, minimal-TCB posture, the boot sequence should add:
- Explicit policy verification
- Lock state assertion
- Defensive readback checks

These additions would make the monitor's security properties measurable and verifiable during platform validation.
