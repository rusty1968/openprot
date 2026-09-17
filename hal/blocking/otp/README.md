# Blocking OTP HAL

This HAL provides composable abstractions for reading and (optionally)
programming One-Time Programmable (OTP) and fuse storage. It defines small,
independent capability traits for low-level hardware access and composes them
into higher-level device contracts.

## Module Layout

The OTP module follows the blocking flash layout:

- `driver.rs` — low-level, composable capability traits (`hal_otp_driver`).
- `otp.rs` — the public blocking interface that composes those capabilities
  (`hal_otp`), plus focused tests.
- `BUILD.bazel` — independent `driver` and `otp` (with an `otp_test`) targets.

The `driver` library has no dependency on the composition layer, so a platform
implementation can implement the capability traits directly without pulling in
higher-level policy.

## Why these traits look like this

The trait family isn't a generic "read/write memory" API — several physical
and security properties specific to OTP/fuse hardware are encoded directly in
the shapes of the types and traits:

- **Write-once, irreversible.** There is no `erase` method anywhere in this
  family, unlike e.g. the `Flash` HAL — the API doesn't offer an operation
  the hardware can't do. `OtpLock`/`lock_region` is the only finalization
  primitive, and it's one-directional by construction.
- **Redundant physical backing, not just pass/fail.** `OtpBitStatus::
  remaining_attempts` and `ErrorKind::AttemptsExhausted` capture a
  fuse-specific reality — a logical bit is often backed by several physical
  fuses for reliability — as a first-class field and error kind, distinct
  from `RegionProtected` ("never allowed" vs. "was allowed, now used up").
- **Fixed native access granularity per region, not a uniform byte stream.**
  Real OTP controllers pick 32-bit, 64-bit, or single-bit transfer
  granularity *per partition*, in hardware — not by caller choice. The
  word/dword/bit split, and `OtpRegionLayout::read_alignment`'s **exact**
  width-match contract (not just a minimum alignment), model that directly:
  `OtpPolicy::check_access` rejects a `u32` access against a region whose
  native width is 8 bytes rather than silently aliasing it.
- **Readability and writability are independent axes.** `OtpRegionStatus`
  (`Readable`/`ReadProtected`/`Error`) is not a single "protected" bit:
  `read_checked` rejects `ReadProtected`, but `write_checked` doesn't,
  because a region can be read-locked without being write-locked (matches
  real silicon, which often has separate read-lock and write-protect-enable
  bits).
- **Locking is region-level, independent of access width.** `OtpLock<R>` is
  its own trait rather than a method on `OtpProgram<T>`, because the
  hardware's digest-lock command operates on a whole partition regardless of
  whether software accesses it as `u32` or `u64`.
- **Region-relative, multi-granularity addressing.** `OtpOffset` (bytes) and
  `OtpBitIndex` (bit position) are distinct newtypes, not reused for each
  other or for a flat global address — because on real hardware they mean
  genuinely different things.

## Key Abstractions

### `OtpOffset` / `OtpBitIndex`

A byte offset (`OtpOffset`) or bit position (`OtpBitIndex`) relative to a
region. Always region-relative, never a flat address; the two are distinct
types so a byte-family and bit-family access can't be confused for each
other.

### `OtpRegion`

A marker trait for a platform's logical region identifier (for example, a
partition or fuse bank). Each capability trait carries its own `Region`
associated type bound by `OtpRegion`.

### Error Model

- `ErrorKind` — a non-exhaustive classification of OTP failures
  (`InvalidAddress`, `AlignmentError`, `RegionProtected`, `AttemptsExhausted`,
  `Hardware`, `Timeout`, `Unsupported`).
- `Error` — implemented by a hardware-specific error type; maps to `ErrorKind`.
- `ErrorType` — associates the error type with a capability.

### Byte-Oriented Capabilities

| Trait | Purpose |
|-------|---------|
| `OtpRead<T>` | Foundational typed read: `read(region, offset) -> Result<T, _>` for any `T: Copy`. |
| `OtpWordRead` / `OtpDwordRead` | Fixed-width refinements of `OtpRead<u32>` / `OtpRead<u64>`, for partitions whose native DAI granule is 32 or 64 bits. |
| `OtpReadBytes` | Bulk byte read: `read_bytes(region, offset, &mut [u8])`, for partition/window controllers; independent of `OtpRead` — a device may implement either or both. |
| `OtpRegionLayout` | Region byte capacity and the region's exact native access width (see above). |
| `OtpRegionStatusAccess` | Hardware access state via `OtpRegionStatus` (`Readable`, `ReadProtected`, `Error`). |
| `OtpLock<R>` | Permanent region locking, independent of access width. |
| `OtpProgram<T>` (+ `OtpWordProgram` / `OtpDwordProgram`) | Optional typed programming: `write(region, offset, data)`; requires `OtpLock` as a supertrait. |
| `OtpProgramBytes` | Bulk byte programming, paired with `OtpReadBytes`. |

Programming is a separate, opt-in capability. Read-only firmware does not
implement it, so a read-only device is read-only by construction.

### Bit-Oriented Capabilities

For regions that are genuinely bit-addressed by hardware, each bit backed by
independently-programmable redundant physical fuses (e.g. straps) — a
different hardware shape from the byte family, not a reinterpretation of it:

| Trait | Purpose |
|-------|---------|
| `OtpBitRead` | Read one bit's value. |
| `OtpBitStatusAccess` | Read one bit's value, protection, and `remaining_attempts` as `OtpBitStatus`. |
| `OtpBitRegionLayout` | Region capacity in bits. |
| `OtpBitProgram` | Program one bit. A protected bit rejects the write even if the requested value already matches — protection means the hardware refuses the command itself, not just that a change would occur. |
| `OtpBitProgramBatch` | Program a range of bits, validating the whole batch before programming any of them — an atomicity guarantee a per-bit loop can't offer. |

### `OtpPolicy<D>`

Wraps a device and validates a request — region status, offset/width match,
bounds — before it ever reaches hardware; a rejected request never touches
the device. Unlike a `BlockingFlash`-style adapter, it does no protocol or
concurrency bridging (the device is already fully synchronous); it's a
validation gate, not an execution-model adapter.

- `read_checked` / `read_bytes_checked` reject `Error` or `ReadProtected`
  region status before calling `OtpRead` / `OtpReadBytes`.
- `write_checked` / `program_bytes_checked` reject `Error` region status
  (but not `ReadProtected` — see above) before calling `OtpProgram` /
  `OtpProgramBytes`.

Failures are reported as `OtpError<E>`, which separates a `Policy(ErrorKind)`
rejection made before hardware access from a `Device(E)` error returned by the
underlying device.
