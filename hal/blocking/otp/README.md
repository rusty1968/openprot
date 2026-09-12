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
- `BUILD.bazel` — independent `driver`, `otp`, and `otp_test` targets.

The `driver` library has no dependency on the composition layer, so a platform
implementation can implement the capability traits directly without pulling in
higher-level policy.

## Design Criteria

Traits are split by abstraction level and dependency direction:

- **`driver.rs`** holds primitive operations a hardware implementation can
  satisfy directly (read a value or a byte range, report region geometry, report
  region status, program a value or a byte range).
- **`otp.rs`** composes those primitives into device contracts and convenience
  bounds. Policy such as alignment and bounds checking belongs here, expressed
  in terms of the facts the driver traits expose.

```text
platform implementation
        |  implements
        v
   driver.rs traits
        |  composed by
        v
     otp.rs devices
        |  used by
        v
   application code
```

## Key Abstractions

### `OtpOffset`

A byte offset relative to a region or address-space base. Offsets are always
measured in bytes; implementations enforce their own alignment and capacity
rules. This avoids ambiguity between byte offsets and word indices.

### `OtpRegion`

A marker trait for a platform's logical region identifier (for example, a
partition or fuse bank). Each capability trait carries its own `Region`
associated type bound by `OtpRegion`.

### Error Model

- `ErrorKind` — a non-exhaustive classification of OTP failures
  (`InvalidAddress`, `AlignmentError`, `RegionProtected`, `Hardware`,
  `Timeout`, `Unsupported`).
- `Error` — implemented by a hardware-specific error type; maps to `ErrorKind`.
- `ErrorType` — associates the error type with a capability.

### Read Capabilities

| Trait | Purpose |
|-------|---------|
| `OtpRead<T>` | Foundational typed read: `read(region, offset) -> Result<T, _>` for any `T: Copy`. |
| `OtpWordRead` | Refinement of `OtpRead<u32>` for fixed-width 32-bit register/window interfaces. |
| `OtpReadBytes` | Bulk byte read: `read_bytes(region, offset, &mut [u8])`, the natural primitive for partition/window controllers. |
| `OtpRegionLayout` | Region byte capacity and read alignment. |
| `OtpRegionStatusAccess` | Hardware access state via `OtpRegionStatus` (`Readable`, `ReadProtected`, `Error`). |

`OtpRead<T>` is the reusable base. `OtpWordRead` names the fixed-width contract
used by register/window hardware; it does not add new methods, it constrains the
read width to `u32`. `OtpReadBytes` is independent of `OtpRead`: a device may
implement either or both. It suits controllers that transfer whole partitions in
one operation, mapping directly onto a bulk-read primitive.

### Programming Capabilities

| Trait | Purpose |
|-------|---------|
| `OtpProgram<T>` | Optional typed programming: `write(region, offset, data)` and `lock_region(region)`. |
| `OtpWordProgram` | Fixed-width refinement (`OtpProgram<u32> + OtpWordRead`). |
| `OtpProgramBytes` | Bulk byte programming: `program_bytes(region, offset, &[u8])`, paired with `OtpReadBytes`. |

Programming is a separate, opt-in capability. Read-only firmware does not
implement it, so a read-only device is read-only by construction.

### Composed Devices

`otp.rs` provides blanket-implemented convenience bounds:

- `OtpReadDevice<T>` = `OtpRead<T> + OtpRegionLayout`
- `OtpProvisioningDevice<T>` = `OtpReadDevice<T> + OtpProgram<T>`

Any type implementing the constituent capabilities automatically satisfies the
composed trait.

Region status is deliberately *not* required by `OtpReadDevice`: controllers
whose readability is governed globally by lifecycle state — rather than per
region — are still read devices. `OtpRegionStatusAccess` is required only where
it is used, namely by the status-checked read path below.

### Checked Wrapper

`BlockingOtp<D>` wraps a device and enforces policy before delegating:

- `read_checked` verifies region status (via `OtpRegionStatusAccess`), offset
  alignment, and in-bounds access before calling `OtpRead`.
- `write_checked` verifies alignment and bounds before calling `OtpProgram`.

Failures are reported as `OtpError<E>`, which separates a `Policy(ErrorKind)`
rejection made before hardware access from a `Device(E)` error returned by the
underlying device.

## Caliptra-SS Mapping

Caliptra-SS exposes fixed-width register and memory-window access rather than a
general writable OTP array, so it composes cleanly without an adapter:

- Runtime firmware implements `OtpRead<u32>` (plus `OtpWordRead`) or
  `OtpReadBytes` for partition-style reads, together with `OtpRegionLayout` —
  read-only. Per-region status (`OtpRegionStatusAccess`) is implemented only
  when the controller exposes it; a device governed by global lifecycle state
  can omit it and still be an `OtpReadDevice`.
- Provisioning or test firmware additionally implements `OtpProgram<u32>` /
  `OtpWordProgram`, or `OtpProgramBytes` for bulk writes.

Offsets are explicitly measured in bytes. Implementations enforce the alignment,
capacity, lifecycle, and hardware-error rules of their generated register block.