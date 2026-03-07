# SPI Flash Storage Backend — Requirements, Design & Implementation Plan

**Date:** 2026-02-16  
**Status:** Draft → Revised → Simplified  
**Author:** Engineering  

---

## 1. Problem Statement

The storage service currently uses `MemFlash` (an in-RAM emulation) as its
backend. We need a real backend that drives SPI NOR flash through the
aspeed-ddk's existing SPI controller drivers.

Since QEMU fully emulates the AST1060's FMC/SPI controllers with real flash
models, `MemFlash` is unnecessary — `AspeedNorFlash` works identically in
QEMU and on real hardware. This eliminates a second code path, the `unsafe`
interior mutability hacks, and all backend selection complexity.

The deeper question: **what is the cleanest possible stack?**
Answer: **One backend. One code path. Zero custom traits.**

## 2. Existing Inventory

### 2.1 aspeed-ddk SPI Stack (bottom-up)

| Layer | Location | Role |
|-------|----------|------|
| `FmcController` / `SpiController` | `aspeed-rust/src/spi/{fmc,spi}controller.rs` | MMIO register drivers; implement `SpiBusWithCs` + `embedded_hal::SpiBus` |
| `ChipSelectDevice<B, SPIPF>` | `aspeed-rust/src/spi/device.rs` | CS-managed wrapper; implements `embedded_hal::SpiDevice` |
| `SpiNorDevice` trait | `aspeed-rust/src/spi/norflash.rs` | NOR flash ops: JEDEC probe, sector erase, page program, read |
| `NorFlashBlockDevice<T>` | `aspeed-rust/src/spi/norflashblockdevice.rs` | Wraps `SpiNorDevice` → `proposed_traits::BlockDevice` |

### 2.2 Storage Service (current — to be simplified)

| Type | Location | Role | Disposition |
|------|----------|------|-------------|
| `FlashStorage` trait | `storage/api/src/backend.rs` | Custom: `read(&self, buf, addr: usize)`, `write`, `erase`, `capacity` | **DELETE** |
| `FlashError` enum | same | Custom error type | **DELETE** |
| `BootConfig` trait | same | A/B boot management | Keep |
| `MemFlash<N>` | `storage/backend-mem/src/lib.rs` | In-RAM impl with `unsafe` interior mutability | **DELETE** (entire crate) |

### 2.3 Standard Ecosystem

| Crate | Status | Key Traits |
|-------|--------|------------|
| `embedded-storage 0.3.1` | In lockfile (transitive via `proposed-traits`) | `ReadNorFlash`, `NorFlash`, `MultiwriteNorFlash` |
| `embedded-hal 1.0` | Direct dep | `SpiBus`, `SpiDevice` |

## 3. Design Decision: Eliminate `FlashStorage`

### 3.1 The problem with the current design

The current `FlashStorage` trait is nearly identical to `embedded_storage::NorFlash`:

| | `FlashStorage` (custom) | `NorFlash` (standard) |
|---|---|---|
| read | `fn read(&self, buf, addr: usize)` | `fn read(&mut self, offset: u32, buf)` |
| write | `fn write(&self, buf, addr: usize)` | `fn write(&mut self, offset: u32, buf)` |
| erase | `fn erase(&self, addr, len: usize)` | `fn erase(&mut self, from: u32, to: u32)` |
| capacity | `fn capacity(&self) -> usize` | `fn capacity(&self) -> usize` |
| alignment | none | `WRITE_SIZE`, `ERASE_SIZE`, `READ_SIZE` |
| errors | custom `FlashError` | `NorFlashErrorKind` |

The only differences:
- **`&self` vs `&mut self`**: `FlashStorage` uses `&self`, forcing every
  backend to smuggle mutability through `unsafe` pointer casts. `NorFlash`
  uses `&mut self` — correct for a mutable device.
- **`usize` vs `u32`**: On thumbv7em (Cortex-M), `usize` IS `u32`. Zero cost.

### 3.2 The cleanest stack

**Delete `FlashStorage`. Delete `MemFlash`. Use `NorFlash` with a single
`AspeedNorFlash` backend that works in both QEMU and real hardware.**

```
Current (5 layers):                   Cleanest (single path):
═══════════════════                   ════════════════════════

Storage Server                        Storage Server
      │ FlashStorage (&self)                │ NorFlash (&mut self)
  ┌───┴──────┐                              │ (RefCell at server level)
  │          │                              │
MemFlash  SpiFlash<T>               AspeedNorFlash<T>
              │ NorFlash              (impl NorFlash)
         AspeedNorFlash                     │
              │ SpiNorDevice           SpiNorDevice
         ChipSelectDevice            (existing, unchanged)
```

**What this eliminates:**
- The `FlashStorage` trait (redundant)
- The `FlashError` enum (use `NorFlashErrorKind`)
- The `backend-spi` adapter crate (not needed)
- The `MemFlash` struct and entire `backend-mem` crate (QEMU emulates
  real FMC/SPI hardware faithfully — no need for a fake)
- All `unsafe` interior mutability
- Backend selection / feature flags

**What this preserves:**
- `BootConfig` trait (stays in `storage-api`, no change)
- `PartitionDef` (stays, no change)
- `StorageClient` (speaks IPC protocol, never touches backend traits)
- `SpiNorDevice` (stays as aspeed-ddk internal API)
- `NorFlashBlockDevice` (stays for `BlockDevice` use case)

## 4. Requirements

### REQ-1: Replace `FlashStorage` with `NorFlash`

The storage server shall use `embedded_storage::nor_flash::NorFlash` as its
backend interface. The custom `FlashStorage` trait and `FlashError` enum
shall be removed from `storage-api`.

### REQ-2: Delete `MemFlash` and `backend-mem`

The `MemFlash<N>` struct and the entire `backend-mem` crate shall be deleted.
QEMU's faithful emulation of the AST1060's FMC/SPI controllers means
`AspeedNorFlash` works identically in QEMU and on real hardware — there is
no need for a separate in-RAM flash emulation.

### REQ-3: `AspeedNorFlash<T>` implements `NorFlash`

New wrapper in aspeed-ddk that adapts `SpiNorDevice` → `NorFlash`. Handles
JEDEC probe, page-boundary splitting, sector-aligned erase. This is the
**sole backend** for both QEMU and real hardware.

### REQ-4: Server uses `RefCell` at the single ownership point

The server wraps its flash in `RefCell<AspeedNorFlash<...>>`, rather than
pushing interior mutability into every backend.

### REQ-5: HAL re-export

`openprot-hal-blocking` re-exports `embedded_storage::nor_flash::*` for
discoverability. No custom trait.

## 5. Architecture

```
┌──────────────────────────────────────────────┐
│             Storage Server (IPC)             │
│  flash: RefCell<AspeedNorFlash<...>>          │
│  dispatch(flash.borrow_mut(), ...)           │
└──────────────────┬───────────────────────────┘
                   │ NorFlash (embedded-storage 0.3.1)
           ┌───────▼─────────────────┐
           │  AspeedNorFlash<T>       │
           │  (QEMU + real hardware)  │
           └───────┬─────────────────┘
                   │ SpiNorDevice (aspeed-ddk, internal)
           ┌───────▼─────────────────┐
           │ ChipSelectDevice        │
           │ + FmcController /       │
           │   SpiController         │
           └─────────────────────────┘
```

One backend. One code path. Works in QEMU and on hardware identically.

### 5.1 Feature-Flag Gating

Backend selection is via Cargo features on the storage server crate.
Currently only `aspeed` is defined. If no backend feature is enabled,
the build fails with a clear compile error.

```rust
// services/storage/server/src/main.rs

// --- Backend selection via feature flags ---

#[cfg(feature = "aspeed")]
mod backend {
    use aspeed_ddk::spi::aspeed_norflash::AspeedNorFlash;
    use aspeed_ddk::spi::fmccontroller::FmcController;
    use aspeed_ddk::spi::device::ChipSelectDevice;
    // ...

    pub type Flash = AspeedNorFlash<ChipSelectDevice<...>>;

    pub fn create_flash() -> Flash {
        let fmc = FmcController::new(...);
        fmc.init().unwrap();
        let cs_dev = ChipSelectDevice { bus: &mut fmc, cs: 0, ... };
        AspeedNorFlash::new(cs_dev).unwrap()
    }
}

#[cfg(not(any(feature = "aspeed")))]
compile_error!("No storage backend selected. Enable a backend feature, e.g. --features aspeed");
```

The dispatch logic is fully generic — adding a future backend is just:
1. Add a new `#[cfg(feature = "vendor_x")] mod backend { ... }` block
2. Update the `compile_error!` guard: `#[cfg(not(any(feature = "aspeed", feature = "vendor_x")))]`
3. The new backend's `Flash` type just needs to implement `NorFlash`

In Bazel, the feature is passed via `crate_features`:
```python
rust_binary(
    name = "storage-server",
    crate_features = ["aspeed"],  # select backend here
    ...
)
```

## 6. Implementation Plan

### Phase 1: Foundation — Add `embedded-storage` to deps

**Step 1.1** — Add `embedded-storage = "0.3.1"` to aspeed-ddk `Cargo.toml`
- File: `aspeed-rust/Cargo.toml`
- Line: add under `[dependencies]`

**Step 1.2** — Add `embedded-storage` to `storage-api` bazel deps
- File: `services/storage/api/BUILD.bazel`
- Add dep: `@oot_crates_no_std//:embedded_storage`

**Step 1.3** — Add `embedded-storage` to HAL blocking
- File: `hal/blocking/Cargo.toml` — add `embedded-storage = { workspace = true }`
- File: `hal/blocking/src/lib.rs` — add re-export module

### Phase 2: `AspeedNorFlash` in aspeed-ddk

**Step 2.1** — Create `aspeed-rust/src/spi/aspeed_norflash.rs`
- `AspeedFlashError` enum implementing `NorFlashError`
- `AspeedNorFlash<T: SpiNorDevice>` struct
- `impl ErrorType`, `impl ReadNorFlash`, `impl NorFlash`
- Page-boundary splitting in `write()`
- Sector-aligned erase loop in `erase()`

**Step 2.2** — Register module in `aspeed-rust/src/spi/mod.rs`
- Add: `pub mod aspeed_norflash;`

### Phase 3: Update `storage-api` backend traits

**Step 3.1** — Rewrite `services/storage/api/src/backend.rs`

Remove:
- `FlashStorage` trait (entire definition)
- `FlashError` enum (entire definition)

Keep:
- `PartitionDef` struct
- `BootConfig` trait
- `BootPartitionId`, `BootPartitionStatus`, `BootConfigError`

Add:
- Re-export of `embedded_storage::nor_flash` for downstream convenience
- Helper: `nor_flash_err_to_storage(NorFlashErrorKind) -> StorageError`

### Phase 4: Delete `backend-mem` crate

**Step 4.1** — Delete the entire `services/storage/backend-mem/` directory

QEMU fully emulates the AST1060's FMC/SPI controllers with real flash
models (`w25q80bl`, `w25q02jvm`). `AspeedNorFlash` works identically in
QEMU and on real hardware — no in-RAM fake is needed.

**Step 4.2** — Remove `backend-mem` from server's `BUILD.bazel` deps

### Phase 5: Update storage server

**Step 5.1** — Rewrite `services/storage/server/src/main.rs`

The server uses feature-gated backend selection with a `compile_error!`
guard. The `backend` module exports a `Flash` type alias and a
`create_flash()` constructor. The rest of the server is generic:

```rust
// Feature-gated backend (see §5.1)
#[cfg(feature = "aspeed")]
mod backend { ... }  // exports Flash type + create_flash()

#[cfg(not(any(feature = "aspeed")))]
compile_error!("No storage backend selected. Enable a backend feature, e.g. --features aspeed");

// Entry point — generic over the selected backend:
let flash = RefCell::new(backend::create_flash());
storage_server_loop(&flash, &boot_config);

// All dispatch functions are generic:
fn dispatch_storage_op<F: NorFlash>(flash: &RefCell<F>, ...) { ... }
```

Key changes:

| Current | New |
|---------|-----|
| `use storage_api::backend::{FlashError, FlashStorage}` | `use embedded_storage::nor_flash::{NorFlash, ReadNorFlash}` |
| `let flash = MemFlash::<FLASH_SIZE>::new()` | `let flash = RefCell::new(AspeedNorFlash::new(cs_dev).unwrap())` |
| `fn dispatch_storage_op(flash: &dyn FlashStorage, ...)` | `fn dispatch_storage_op<F: NorFlash>(flash: &RefCell<F>, ...)` |
| `flash.read(output, address as usize)` | `flash.borrow_mut().read(address, output)` |
| `flash.write(&payload[..len], address as usize)` | `flash.borrow_mut().write(address, &payload[..len])` |
| `flash.erase(address as usize, length as usize)` | `flash.borrow_mut().erase(address, address + length)` |
| `flash.capacity() as u32` | `flash.borrow().capacity() as u32` |
| `flash_err_to_storage(e: FlashError)` | `nor_err_to_storage(e: impl NorFlashError)` |

**Step 5.2** — Update `services/storage/server/BUILD.bazel`
- Add dep: `@oot_crates_no_std//:embedded_storage`
- Remove dep: `//services/storage/backend-mem:storage-backend-mem`
- Add dep: aspeed-ddk (for `AspeedNorFlash`)

### Phase 6: Tests

**Step 6.1** — Existing storage tests (`services/storage/tests/`) should
pass unchanged — they test via `StorageClient` over IPC, which doesn't
touch backend traits at all.

**Step 6.2** — Add unit test for `AspeedNorFlash` using a mock `SpiNorDevice`.

**Step 6.3** — QEMU test: launch with `-M ast1060-evb,fmc-model=w25q80bl`
and `-drive file=test.bin,format=raw,if=mtd` to exercise the full
FMC → SPI NOR → storage server path.

## 7. File Change Matrix

| # | File | Action | Phase |
|---|------|--------|-------|
| 1 | `aspeed-rust/Cargo.toml` | Add `embedded-storage = "0.3.1"` | 1 |
| 2 | `aspeed-rust/src/spi/aspeed_norflash.rs` | **CREATE** — `AspeedNorFlash<T>` | 2 |
| 3 | `aspeed-rust/src/spi/mod.rs` | Add `pub mod aspeed_norflash` | 2 |
| 4 | `services/storage/api/src/backend.rs` | Remove `FlashStorage`, `FlashError`; add re-export | 3 |
| 5 | `services/storage/api/BUILD.bazel` | Add `embedded_storage` dep | 1 |
| 6 | `services/storage/backend-mem/` | **DELETE** entire directory | 4 |
| 7 | `services/storage/server/src/main.rs` | `RefCell<AspeedNorFlash>`, update all handlers | 5 |
| 8 | `services/storage/server/BUILD.bazel` | Add `embedded_storage`, remove `backend-mem` dep | 5 |
| 9 | `hal/blocking/Cargo.toml` | Add `embedded-storage` dep | 1 |
| 10 | `hal/blocking/src/lib.rs` | Re-export `nor_flash` | 1 |

**Files NOT changed:**
- `services/storage/client/src/lib.rs` — speaks IPC, no backend deps
- `services/storage/tests/src/main.rs` — tests via client, no backend deps
- `aspeed-rust/src/spi/norflash.rs` — `SpiNorDevice` stays unchanged
- `aspeed-rust/src/spi/norflashblockdevice.rs` — `BlockDevice` adapter stays

**Files DELETED:**
- `services/storage/backend-mem/src/lib.rs`
- `services/storage/backend-mem/BUILD.bazel`
- `services/storage/backend-mem/Cargo.toml` (if exists)

## 8. Dependency Graph (final)

```
embedded-storage 0.3.1   ← standard crate, already in lockfile
        │
        ├──→ openprot-hal-blocking   (re-export only, 1 line)
        │
        ├──→ aspeed-ddk              (AspeedNorFlash: NorFlash impl)
        │         └── SpiNorDevice   (internal, unchanged)
        │
        ├──→ storage-api             (re-export for error mapping)
        │         └── BootConfig     (unchanged)
        │
        └──→ storage-server          (RefCell<AspeedNorFlash>)
```

**Zero custom traits. Zero adapter crates. Zero fake backends.**

## 9. Execution Order & Dependencies

```
Phase 1 (parallel) ─────────────────────────────────
  ├─ 1.1  aspeed-ddk Cargo.toml          (no deps)
  ├─ 1.2  storage-api BUILD.bazel        (no deps)
  └─ 1.3  HAL blocking Cargo + lib.rs    (no deps)

Phase 2 (requires 1.1) ─────────────────────────────
  ├─ 2.1  Create aspeed_norflash.rs
  └─ 2.2  Register in mod.rs

Phase 3 (requires 1.2) ─────────────────────────────
  └─ 3.1  Rewrite backend.rs (remove FlashStorage/FlashError)

Phase 4 (no deps) ──────────────────────────────────
  ├─ 4.1  Delete backend-mem/
  └─ 4.2  Remove from server BUILD.bazel

Phase 5 (requires 2, 3, 4) ────────────────────────
  ├─ 5.1  Rewrite server main.rs (AspeedNorFlash + RefCell)
  └─ 5.2  Update server BUILD.bazel

Phase 6 (requires 5) ──────────────────────────────
  ├─ 6.1  Run existing storage tests
  ├─ 6.2  Add AspeedNorFlash unit test
  └─ 6.3  QEMU SPI flash integration test
```

## 10. Risk Assessment

| Risk | Mitigation |
|------|-----------|
| `NorFlash` is not dyn-compatible (has const generics) | Server uses `<F: NorFlash>` generic, not `dyn`. Single concrete type per binary. |
| `erase(from, to)` vs `erase(addr, len)` | Server converts: `erase(addr, addr + len)`. One-line change per call site. |
| `embedded-storage` bazel target name | Already resolved: `@oot_crates_no_std//:embedded_storage` (confirmed in MODULE.bazel.lock) |
| No MemFlash for offline testing | QEMU emulates FMC/SPI faithfully — same code path as hardware. `qemu-system-arm -M ast1060-evb` is sufficient. |
| QEMU flash image setup | Create `test.bin` with `dd if=/dev/zero bs=1M count=1`, pass via `-drive file=test.bin,format=raw,if=mtd` |
| Existing tests break | Tests use `StorageClient` (IPC) — backend change is invisible to them |
