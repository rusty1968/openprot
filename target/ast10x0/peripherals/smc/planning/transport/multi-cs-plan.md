# Multi-CS Command Routing — Implementation Plan

Date: 2026-05-02

## Background

`transceive_user` currently hard-codes CS0 for all register writes and uses
`controller_id.flash_window_address()` as the single AHB window pointer.
aspeed-rust's `spi_nor_transceive_user()` parameterises everything on
`self.current_cs`: the CS control register selection, the user-mode base
value, the normal-read restore value, and the AHB window start address all
come from per-CS tables indexed by `current_cs`.

The reference HAL has CS0 and CS1 config (`SmcConfig.cs0` / `cs0.cs1`) and
separate register accessors (`read_cs0_ctrl` / `write_cs0_ctrl` /
`read_cs1_ctrl` / `write_cs1_ctrl`) but does not yet plumb a CS selector
through `transceive_user`.

---

## aspeed-rust Analogue

```
// aspeed-rust pattern (simplified):
fn activate_user(&mut self) {
    let cs = self.current_cs;          // 0 or 1
    let user_reg = cmd_mode[cs].user;
    cs_ctrlreg_w!(self, cs, user_reg | INACTIVE);
    cs_ctrlreg_w!(self, cs, user_reg);
}

fn deactivate_user(&mut self) {
    let cs = self.current_cs;
    cs_ctrlreg_w!(self, cs, cmd_mode[cs].user | INACTIVE);
    cs_ctrlreg_w!(self, cs, cmd_mode[cs].normal_read);
}

fn spi_nor_transceive_user(&mut self, op_info: ...) {
    let cs = self.current_cs;
    let start_ptr = decode_addr[cs].start as *mut u32;
    self.activate_user();
    cs_ctrlreg_w!(self, cs, cmd_mode[cs].user | cmd_io_bits);
    spi_write_data(start_ptr, &[opcode]);
    // ... addr, tx, rx phases with same cs-indexed register ...
    self.deactivate_user();
}
```

Key observations:
- `current_cs` is an index (0 or 1) stored on the controller struct.
- Both the **CS control register** and the **AHB window base address** are
  indexed by CS.
- Per-CS `normal_read` value is stored in a `cmd_mode[cs]` table.
- The AHB window base per-CS comes from a `decode_addr[cs].start` table
  populated during segment setup.

---

## Proposed Changes

### Step 1 — Add `ChipSelect` enum to `types.rs`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipSelect {
    Cs0 = 0,
    Cs1 = 1,
}
```

Re-export from `mod.rs` alongside the other public types.

### Step 2 — Extend `Smc<Ready>` to carry per-CS normal-read values

Currently `normal_read_ctrl: u32` holds only the CS0 value.  Replace with an
array:

```rust
// Before
normal_read_ctrl: u32,

// After
normal_read_ctrl: [u32; 2],
```

Populate both slots during `init()`:
- `normal_read_ctrl[0]` ← `regs.read_cs0_ctrl()` (already done for CS0)
- `normal_read_ctrl[1]` ← `regs.read_cs1_ctrl()` (new; only meaningful when
  `config.cs1.is_some()`)

### Step 3 — Add per-CS AHB window base tracking

aspeed-rust's `decode_addr[cs].start` is the per-CS AHB aperture start.
Add a parallel array to `Smc<Ready>`:

```rust
flash_window_base: [usize; 2],
```

Populate during `init()` / `setup_segments()`:
- `flash_window_base[0]` = `controller_id.flash_window_address()`
- `flash_window_base[1]` = `controller_id.flash_window_address() + cs0_size`

The CS1 window immediately follows the CS0 segment in the controller's
AHB aperture, matching the segment register layout already computed in
`setup_segments()`.

### Step 4 — Add `cs_ctrl_read` / `cs_ctrl_write` helpers on `SmcRegisters`

Add a dispatch method rather than an `if cs == 0` branch scattered through
`transceive_user`:

```rust
impl SmcRegisters {
    pub fn read_cs_ctrl(&self, cs: ChipSelect) -> u32 {
        match cs {
            ChipSelect::Cs0 => self.read_cs0_ctrl(),
            ChipSelect::Cs1 => self.read_cs1_ctrl(),
        }
    }

    pub fn write_cs_ctrl(&self, cs: ChipSelect, value: u32) {
        match cs {
            ChipSelect::Cs0 => self.write_cs0_ctrl(value),
            ChipSelect::Cs1 => self.write_cs1_ctrl(value),
        }
    }
}
```

### Step 5 — Add `cs` parameter to `transceive_user`

```rust
// Before
pub fn transceive_user(
    &self,
    cmd: &[u8],
    tx_payload: &[u8],
    rx: &mut [u8],
    mode: TransferMode,
) -> Result<(), SmcError>

// After
pub fn transceive_user(
    &self,
    cs: ChipSelect,
    cmd: &[u8],
    tx_payload: &[u8],
    rx: &mut [u8],
    mode: TransferMode,
) -> Result<(), SmcError>
```

Inside the body replace every hard-coded `write_cs0_ctrl` / `write_cs0_ctrl`
with `write_cs_ctrl(cs, …)`, index `normal_read_ctrl[cs as usize]` for the
user-base derivation and restore, and use `flash_window_base[cs as usize]`
for the AHB window pointer.

Add a guard: if `cs == ChipSelect::Cs1` and `config.cs1.is_none()` return
`Err(SmcError::InvalidChipSelect)`.

### Step 6 — Update FMC/SPI facade wrappers

`FmcReady::transceive_user` and `SpiReady::transceive_user` both forward to
`ReadySmc::transceive_user`.  Add the `cs` parameter to both; existing single-
CS call sites pass `ChipSelect::Cs0` unchanged.

For the FMC facade the only valid CS selection at construction time is
determined by the config.  The facade can provide a convenience wrapper
`transceive_user_cs0` that hardwires `ChipSelect::Cs0` to preserve the
CS0-only API for callers that don't need multi-CS.

### Step 7 — Update device layer (`SpiNorFlash`)

`SpiNorFlash` carries a reference to a `FmcReady` or `SpiReady`.  It needs
to know which CS its flash device sits on.  Add a `cs: ChipSelect` field,
defaulting to `Cs0` in `from_fmc` / `from_spi`.  Provide a `from_fmc_cs` /
`from_spi_cs` constructor for explicit CS selection.

All `transceive_user` call sites in `issue_command` / `read_status_impl`
pass `self.cs`.

### Step 8 — Update tests

- All existing tests pass `ChipSelect::Cs0` (behaviour unchanged).
- Add a new QEMU integration test `target_device_qemu_multi_cs.rs`:
  - Construct an `FmcReady` with both `cs0` and `cs1` populated.
  - Issue RDSR on `Cs0` and `Cs1`; assert both return status bytes without
    asserting each other's CS.
  - Confirm `InvalidChipSelect` is returned when `Cs1` is requested on a
    controller configured with `cs1: None`.

---

## File Change Summary

| File | Change |
|---|---|
| `types.rs` | Add `ChipSelect` enum; extend re-export |
| `controller.rs` | `normal_read_ctrl: [u32; 2]`; `flash_window_base: [u32; 2]`; `cs` param on `transceive_user`; CS guard |
| `registers.rs` | Add `read_cs_ctrl` / `write_cs_ctrl` dispatch helpers |
| `fmc.rs` | Add `cs` param to `transceive_user`; add `transceive_user_cs0` convenience wrapper |
| `spi.rs` | Same as `fmc.rs` |
| `device/flash.rs` | Add `cs: ChipSelect` field; `from_fmc_cs` / `from_spi_cs` constructors; pass `self.cs` to all `transceive_user` calls |
| `mod.rs` | Re-export `ChipSelect` |
| `tests/smc/target_device_qemu_multi_cs.rs` | New QEMU integration test |
| `tests/smc/BUILD.bazel` | New `smc_device_qemu_multi_cs_test` target |
| All existing test files | Add `ChipSelect::Cs0` to `transceive_user` call sites |

---

## Invariants and Guard Policy

| Condition | Error returned |
|---|---|
| `cs == Cs1` and `config.cs1.is_none()` | `SmcError::InvalidChipSelect` |
| Controller not in `Ready` state (any CS) | `SmcError::ControllerNotReady` |

---

## Scope Exclusions

- CS2+ (hardware supports up to 5 on some variants): out of scope; array size
  2 matches current `SmcConfig` shape.
- DMA multi-CS: DMA uses a single-segment model; routing DMA by CS is a
  separate effort.
- Simultaneous multi-CS access: not required; single-transaction model
  unchanged.
