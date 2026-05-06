# Board Refactoring: Consolidate Hardware Orchestration

## Problem: Duplication Across Modules

### Current Architecture Issues

**1. Mux Control Duplication**

- **SCU Module** (`target/ast10x0/peripherals/scu/routing.rs`):
  - `set_spim_ext_mux()` - Sets external mux via SCU0F0.ext_mux_select_sig_of_spipfN()
  - `get_spim_ext_mux()` - Reads external mux state
  - Full implementation using SCU register access

- **SpiMonitor Trait** (`target/ast10x0/peripherals/spimonitor/traits.rs`):
  - `set_mux()` - Abstract method for mux control
  - `read_mux()` - Abstract method for mux reading

- **Ast1060Monitor Implementation** (`target/ast10x0/peripherals/spimonitor/ast1060_monitor.rs`):
  - `set_mux()` - Returns `HardwareError` (cannot access SCU)
  - `read_mux()` - Returns `HardwareError` (cannot access SCU)
  - **Root cause**: Ast1060Monitor only holds SPIPF registers, not SCU

**2. Ownership Model Problems**

Current structure:
```
scu/
  ├── registers.rs (ScuRegisters)
  ├── routing.rs   (SCU mux/passthrough logic)
  └── mod.rs

spimonitor/
  ├── registers.rs (SpiMonitorRegisters)
  ├── traits.rs (Monitor trait)
  ├── ast1060_monitor.rs (Ast1060Monitor struct)
  └── mod.rs
```

**Issue**: Both modules provide mux/routing functionality but can't see each other's register types.

**3. Test Consequences**

`test_boot_uc` cannot use `monitor.set_mux()` for mux switching. Instead, it would need:
- Separate SCU imports
- Knowledge of SCU0F0 register details
- Manual coordination between two subsystems

---

## Solution: Board Orchestration Layer

### High-Level Architecture

Create a **board-level orchestration crate** that owns and coordinates all hardware:

```
board/
  ├── Cargo.toml
  ├── src/
  │   ├── lib.rs (Ast1060Board struct definition)
  │   ├── peripherals.rs (Hardware ownership)
  │   ├── monitor.rs (Monitor orchestration)
  │   ├── clock.rs (Clock orchestration)
  │   └── reset.rs (Reset orchestration)
```

Rename existing `board_descriptors` → `board`:
```
board_descriptors/ → board/
```

### Board Struct Design

```rust
// board/src/lib.rs

pub struct Ast1060Board {
    // Hardware register ownership
    scu: ScuRegisters,
    spipf: [SpiMonitorRegisters; 4],
    uart: UartRegisters,
    // ... other peripherals
}

impl Ast1060Board {
    /// Initialize the board with all hardware blocks
    pub unsafe fn init() -> Self {
        Self {
            scu: ScuRegisters::new(),
            spipf: [
                SpiMonitorRegisters::new_for_controller(SpiMonitorController::Spim0),
                SpiMonitorRegisters::new_for_controller(SpiMonitorController::Spim1),
                SpiMonitorRegisters::new_for_controller(SpiMonitorController::Spim2),
                SpiMonitorRegisters::new_for_controller(SpiMonitorController::Spim3),
            ],
            uart: UartRegisters::new(),
        }
    }

    /// Get a Monitor orchestrator for SPI security operations
    pub fn monitor(&mut self) -> Ast1060Monitor {
        Ast1060Monitor::new(&mut self.scu, &mut self.spipf)
    }

    /// Get a Clock orchestrator for clock configuration
    pub fn clock(&self) -> &ScuRegisters {
        &self.scu
    }
}
```

### Monitor Implementation (Now Board-Aware)

```rust
// board/src/monitor.rs

pub struct Ast1060Monitor<'a> {
    scu: &'a mut ScuRegisters,
    spipf: &'a mut [SpiMonitorRegisters; 4],
}

impl<'a> Ast1060Monitor<'a> {
    pub fn new(
        scu: &'a mut ScuRegisters,
        spipf: &'a mut [SpiMonitorRegisters; 4],
    ) -> Self {
        Self { scu, spipf }
    }
}

impl<'a> Monitor for Ast1060Monitor<'a> {
    fn set_mux(&mut self, instance: MonitorInstance, mux: MuxSelect) -> BootResult<()> {
        // Now we CAN access SCU! Delegate to SCU routing
        let scu_instance = map_monitor_to_scu_instance(instance);
        let scu_mux = map_mux_select_to_scu(mux);
        self.scu.set_spim_ext_mux(scu_instance, scu_mux);
        Ok(())
    }

    fn read_mux(&self, instance: MonitorInstance) -> BootResult<MuxSelect> {
        // Real implementation using SCU
        let scu_instance = map_monitor_to_scu_instance(instance);
        let scu_mux = self.scu.get_spim_ext_mux(scu_instance);
        Ok(map_scu_mux_to_select(scu_mux))
    }

    // ... other Monitor methods
}
```

### Key Design Principle: Composition Over Duplication

**Do NOT reimplement register logic in Monitor.** Instead, **delegate to peripheral-level helpers**.

#### Pattern from `spim_wiring.rs` (Existing):

```rust
// board/src/spim_wiring.rs - CORRECT approach
unsafe fn apply_spim_wiring(
    scu: &ScuRegisters,
    controller_id: SmcController,
    wiring: SpimWiring,
    policy: &MonitorPolicy,
) -> Result<LockedSpiMonitor, SpimWiringError> {
    // Don't reimplement SCU field manipulation!
    // Instead, call the pre-built helpers:
    scu.set_spim_internal_master_route(wiring.instance, wiring.source);  // ✓ Delegate
    scu.set_spim_passthrough(wiring.instance, wiring.passthrough);        // ✓ Delegate
    scu.set_spim_ext_mux(wiring.instance, wiring.ext_mux);                // ✓ Delegate
    scu.set_spim_miso_multi_func(wiring.instance, wiring.miso_multi_func); // ✓ Delegate
    
    // Then use SPIPF methods the same way
    let monitor = unsafe { SpiMonitor::<Uninitialized>::new(monitor_controller) };
    let configured = monitor.apply_policy(policy)?;
    let locked = configured.lock()?;
    Ok(locked)
}
```

**This is correct** because:
- `spimonitor` module owns policy/lock logic
- `scu` module owns routing logic
- `board/spim_wiring.rs` composes them without reimplementing

#### Pattern for `Ast1060Monitor` (New):

`board/src/monitor.rs` should follow the **same principle**:

```rust
// board/src/monitor.rs - FOLLOW the same pattern
impl<'a> Monitor for Ast1060Monitor<'a> {
    fn set_mux(&mut self, instance: MonitorInstance, mux: MuxSelect) -> BootResult<()> {
        // ✓ Delegate to SCU routing (don't rewrite SCU0F0 bit logic!)
        self.scu.set_spim_ext_mux(Self::instance_to_scu(instance), Self::mux_to_scu(mux));
        Ok(())
    }

    fn read_mux(&self, instance: MonitorInstance) -> BootResult<MuxSelect> {
        // ✓ Delegate to SCU routing (don't reread SCU0F0 yourself!)
        let scu_mux = self.scu.get_spim_ext_mux(Self::instance_to_scu(instance));
        Ok(Self::scu_to_mux(scu_mux))
    }

    fn soft_reset(&mut self, instance: MonitorInstance) -> BootResult<()> {
        let regs = self.regs_mut(instance);
        // ✓ Only SPIPF-specific logic here (bit manipulation, polls)
        let mut ctrl = regs.read_ctrl();
        ctrl |= 0x80;
        regs.write_ctrl(ctrl);
        Ok(())
    }
}
```

**This prevents duplication** because:
- `scu/routing.rs` owns SCU mux logic → Monitor calls `set_spim_ext_mux()`, never touches SCU0F0 directly
- `spimonitor/registers.rs` owns SPIPF operations → Monitor calls `read_ctrl()`, not raw MMIO
- Board only handles type conversions (MonitorInstance ↔ SpiMonitorInstance, MuxSelect ↔ ScuExtMuxSelect)

**Anti-Pattern** (what NOT to do):

```rust
// ✗ WRONG - reimplements mux logic that already exists in scu/routing.rs
fn set_mux(&mut self, instance: MonitorInstance, mux: MuxSelect) -> BootResult<()> {
    let bit = match mux {
        MuxSelect::RotControl => 0,
        MuxSelect::HostControl => 1,
    };
    // DON'T DO THIS - raw SCU0F0 manipulation:
    self.scu.scu0f0().modify(|_, w| w.ext_mux_select_sig_of_spipf1().bit(bit));
    Ok(())
}
```

This **duplicates** `scu/routing.rs::set_spim_ext_mux()` logic and breaks layering.

---

### Test Usage (Clean Interface)

```rust
// tests/spim/test_boot_uc/target.rs

#[allow(unsafe_op_in_unsafe_fn)]
fn main() {
    let mut board = unsafe { Ast1060Board::init() };
    let mut monitor = board.monitor();

    // Phase 1: Mux to ROT
    monitor.set_mux(MonitorInstance::Spim0, MuxSelect::RotControl)?;
    
    // Phase 2: Configure policy
    monitor.set_address_privilege(/*...*/)?;
    
    // Phase 3: Lock and switch to host
    monitor.lock_policy(MonitorInstance::Spim0)?;
    monitor.set_mux(MonitorInstance::Spim0, MuxSelect::HostControl)?;
    
    // Clean, unified interface - no duplication!
}
```

---

## Migration Path

### Step 1: Refactor Board Structure
- [ ] Create `board/Cargo.toml` with dependencies on scu, spimonitor, uart, etc.
- [ ] Define `Ast1060Board` struct owning all hardware
- [ ] Implement `unsafe fn init()` factory

### Step 2: Move Monitor to Board
- [ ] Create `board/src/monitor.rs`
- [ ] Move `Ast1060Monitor` from `spimonitor/ast1060_monitor.rs` to board
- [ ] Update impl to accept `&mut ScuRegisters` + `&mut [SpiMonitorRegisters; 4]`
- [ ] Implement all Monitor trait methods using SCU + SPIPF

### Step 3: Add Delegators
- [ ] SCU methods like `set_spim_ext_mux()` remain in scu/routing.rs (low-level)
- [ ] Monitor methods call these delegators—**never reimplement register logic**
- [ ] Type conversions (MonitorInstance ↔ SpiMonitorInstance) live in Monitor
- [ ] No duplication: scu/routing handles register details, board/monitor orchestrates
- [ ] **Follow the principle**: Composition over duplication (see design section above)

### Step 4: Update Tests
- [ ] Refactor `test_boot_uc` to use `board::Ast1060Board`
- [ ] Remove manual SCU imports from test
- [ ] Use unified `monitor.set_mux()` instead of trying to access HardwareError

### Step 5: Update Build System
- [ ] Rename `board_descriptors` crate to `board`
- [ ] Add board/Cargo.toml with proper dependencies
- [ ] Update all test BUILD.bazel files to depend on board crate

---

## Benefits

| Issue | Current | After Refactor |
|-------|---------|----------------|
| **Mux Control** | Returns HardwareError | Fully implemented via delegating to `scu.set_spim_ext_mux()` |
| **Module Coupling** | SCU + SpiMonitor independent | Board coordinates both (composition) |
| **Test Clarity** | Manual SCU calls needed | Single Monitor interface |
| **Hardware Ownership** | Scattered across modules | Centralized in Board |
| **Orchestration** | No clear layer | Board provides it |
| **Duplication** | Mux logic in 2+ places | Single source of truth (scu/routing.rs) |
| **Maintainability** | Bug fixes need multiple updates | Changes in one place (peripheral layer) |
| **Layering** | Blurred (raw register access everywhere) | Clear (peripherals → board orchestration → tests) |

---

## aspeed-rust Reference

Their `SpiMonitor<SPIPF>` struct holds both:
```rust
pub struct SpiMonitor<SPIPF: SpipfInstance> {
    pub spi_monitor: &'static ast1060_pac::spipf::RegisterBlock,
    pub scu: &'static ast1060_pac::scu::RegisterBlock,  // <-- Both!
    // ...
}
```

**Our board-based approach is analogous**, but better encapsulated:
- aspeed-rust: struct holds `&'static` refs (static board)
- reference: Board struct owns and lends refs to Ast1060Monitor (more flexible)

---

## Architecture Decisions

### 1. Monitor Location: Move Entirely to Board ✓ DECIDED

**Decision**: `Ast1060Monitor` lives in `board/src/monitor.rs`, NOT in `spimonitor/`.

**Rationale**:
- **Separation of Concerns**: `spimonitor/` stays focused on SPIPF hardware; `board/` handles orchestration
- **No Circular Dependencies**: `board` depends on `{spimonitor, scu}`, clean DAG with no loops
- **Clear Hierarchy**: peripheral → board → test, explicit architecture
- **Scalability**: Board can add `clock.rs`, `reset.rs`, etc. without bloating spimonitor
- **Intent**: Tests ask board for orchestrators (`board.monitor()`), not vice versa
- **Documentation**: Trait in `spimonitor/traits.rs` defines *what*; implementation in `board/src/monitor.rs` shows *how*

### 2. Other Orchestrators Follow Same Pattern ✓ DECIDED

**Decision**: Yes—`board.clock()`, `board.reset()`, etc.

All hardware subsystems should be accessible via orchestration methods on the board, maintaining consistency.

### 3. Multi-Board Support: Generic Design ✓ DECIDED

**Decision**: Design for generics from the start.

Future: `Board<UART: UartHw, SPIPF: SpiMonitorHw, SCU: ScuHw>` trait bounds allow different board implementations to be tested identically. For now, `Ast1060Board` is concrete; abstraction layer ready for expansion.

### 4. Board Lifecycle: Recreatable Per Test ✓ DECIDED

**Decision**: Non-singleton—create fresh instance per test phase.

Each test creates `let mut board = unsafe { Ast1060Board::init() }`, ensuring clean state. During actual boot, there's a single board instance; tests can create multiple for validation phases.
