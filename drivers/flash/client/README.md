# flash_client

Userspace IPC client facade for the flash driver.

Bazel target: `//drivers/flash/client:flash_client`

## Purpose

`flash_client` gives any userspace task a simple, blocking API to read, write,
and erase flash over a Pigweed IPC channel. It handles all request
serialization, response parsing, and error mapping.  It has no knowledge of
the underlying hardware — that lives in the platform backend on the server side.

## Layer position

```
Application task
      │
      ▼
FlashClient  ←  this crate
      │  channel_transact (Pigweed IPC)
      ▼
flash_server  →  PlatformFlashBackend  →  SMC / FMC controller
```

See `drivers/flash/README.md` for the full layered architecture.

## API

### Construction

```rust
let flash = FlashClient::new(handle);  // handle: u32 IPC channel handle
```

### Probe

```rust
flash.exists()?;            // returns Ok(()) if server is reachable
```

### Geometry

```rust
let bytes = flash.capacity()?;    // total flash size in bytes
let chunk  = flash.chunk_size()?; // max bytes per single read/write
```

### Read

```rust
let mut buf = [0u8; 256];
let n = flash.read(address, &mut buf)?;
// or with a deadline:
let n = flash.read_with_timeout(address, &mut buf, deadline)?;
```

`buf.len()` must be ≤ `chunk_size()`. For reads larger than one chunk, the
caller is responsible for issuing multiple calls and advancing the address.

### Write

```rust
let written = flash.write(address, &data[..])?;
// or with a deadline:
let written = flash.write_with_timeout(address, &data[..], deadline)?;
```

`data.len()` must be ≤ `chunk_size()`.

### Erase

```rust
flash.erase(address, length)?;
// or with a deadline:
flash.erase_with_timeout(address, length, deadline)?;
```

Both `address` and `length` must be aligned to and a multiple of the backend
erase granule (reported as `chunk_size` for now; this may be refined per
platform).

## Error handling

```rust
pub enum ClientError {
    IpcError(pw_status::Error),   // transport-level failure
    ServerError(FlashError),      // server returned a flash error code
    InvalidResponse,              // response frame is malformed
    BufferTooSmall,               // caller buffer exceeds MAX_PAYLOAD_SIZE
}
```

`FlashError` variants (defined in `flash_api`):

| Variant | Meaning |
|---|---|
| `InvalidOperation` | Unrecognised opcode |
| `InvalidAddress` | Address out of range |
| `InvalidLength` | Length zero, overflow, or misaligned |
| `BufferTooSmall` | Server-side buffer constraint |
| `Busy` | Backend busy |
| `Timeout` | Operation timed out |
| `WouldBlock` | Operation could not be completed immediately |
| `IoError` | Media-level failure |
| `NotPermitted` | Write-protected or restricted region |
| `InternalError` | Unclassified server fault |

## Constraints

- `no_std` — targets Pigweed kernel userspace tasks only.
- Bazel `target_compatible_with` is scoped to AST10x0 targets (`TARGET_COMPATIBLE_WITH`).
- Single call is limited to `MAX_PAYLOAD_SIZE` (256 bytes) per transaction.
- All calls are synchronous / blocking on `channel_transact`.

## Dependencies

| Crate | Role |
|---|---|
| `flash_api` | Wire types (`FlashOp`, `FlashRequestHeader`, `FlashResponseHeader`, `FlashError`) |
| `userspace` (Pigweed) | `syscall::channel_transact`, `time::Instant` |
| `pw_status` | IPC transport error type |
| `zerocopy` | Zero-copy header deserialization |
