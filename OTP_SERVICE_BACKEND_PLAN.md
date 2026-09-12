# Plan: OTP service backed by Caliptra OtpController

Goal: connect the pure `otp_server::dispatch` (`services/otp/server/src/lib.rs`) to
real hardware using `veer_peripherals::OtpController` (the DAI driver) plus a kernel
IPC runtime.

## The gap
`dispatch<D, ..>` requires `D: OtpReadBytes<Region = RegionId> + OtpProgramBytes<Region = RegionId>`.
`OtpController` only implements the word traits (`OtpRead<u32>` / `OtpProgram<u32>`)
with `Region = Partition`. Two mismatches:

1. **word vs byte** — `read_word`/`write_word` vs `read_bytes`/`program_bytes`
2. **`Partition { base, size }` vs wire `RegionId(u16)`**

## Components

1. **Byte-over-word on the controller** (in `veer_peripherals`, `Region = Partition`):
   implement `OtpReadBytes`/`OtpProgramBytes` by looping `read_word`/`write_word`.
   - 4-byte-align offset + len → `AlignmentError`; bounds vs `region.size()` → `InvalidAddress`.
   - reads may cover a partial trailing word (read the word, copy the needed bytes).
   - writes stay word-aligned (write-once); reject sub-word writes.

2. **Caliptra backend adapter** (new crate, e.g. `services/otp/backend-caliptra`):
   newtype `CaliptraOtpBackend(OtpController)` implementing the byte traits with
   `Region = RegionId`, translating `RegionId → Partition` via
   `caliptra_ss_registers::fuses`, delegating to #1.
   - `REGION_SVN` → `SVN_PARTITION_BYTE_OFFSET/SIZE` (0x390 / 0x28)
   - `REGION_VENDOR_HASHES_MANUF` → `VENDOR_HASHES_MANUF_PARTITION_*` (0x3f8 / 0x40)
   - unknown region → `InvalidAddress`

3. **Board traits** (`AccessPolicy` / `SvnCodec` / `FieldMap`):
   - `CaliptraFieldMap`: `FieldId → (RegionId, offset, len)` mirroring the server test `Fields`.
   - `CaliptraSvnCodec`: one-hot/bitmap SVN encoding + monotonic compare (match emulator `svn_to_bitmap`).
   - `CaliptraAccessPolicy`: `CallerId` (from channel) → read/program/commit rights.

4. **`otp-server-runtime` crate** (kernel-tagged, NEW; the README defers it):
   pw_kernel wait/respond loop; `OtpController::from_addr(OTP_CTRL_ADDR)` → backend →
   derive `CallerId` from the channel → `dispatch` → respond.
   Model on `drivers/usart/server/runtime.rs` / the mctp server.

5. **Wiring**: `system.json5` app/process + `channel_handler`; `BUILD.bazel` deps
   (`otp_api`, `otp_server`, `veer_peripherals`, backend, registers, `target_common`, kernel).

## Testing
- **host**: extend the `dispatch` unit tests; factor the `RegionId → Partition` map +
  alignment into a pure function and unit-test it on the host (no MMIO).
- **emulator**: integration test provisioning known fuses via `OtpArgs`, doing a
  `ReadField`/`ReadBytes` round-trip; extend the existing `//target/veer/tests/otp` harness.

## Decisions to confirm before implementing
1. **Scope**: read path first, or include `ProgramBytes` + `CommitSvnFloor` now?
2. **Placement**: backend adapter + board traits as a new Caliptra-specific crate
   (keeps `veer_peripherals` vendor-neutral)?
3. **Full IPC runtime now**, or backend adapter + host/emulator validation first, IPC after?

## Key facts
- `OTP_CTRL_ADDR = 0x7000_0000`; DAI word granule 4B; controller directory spelled "peripehrals".
- Fuse table via `caliptra_ss_registers::fuses` (`target/veer/registers/registers.rs`).
- To pick up local caliptra-mcu-sw edits in openprot bazel:
  `--override_repository=caliptra_mcu_sw=/home/antrocha/work/otp/caliptra-mcu-sw`
- Interrupts abandoned (emulator raises no OTP IRQ); stay polling.
