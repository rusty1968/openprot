<!-- Licensed under the Apache-2.0 license -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# OTP service server (`otp_server`)

Server side of the OTP userspace service. [`dispatch`] is a **pure function** —
no `userspace`, no IPC, no globals — generic over any HAL OTP device, so the
whole protocol + policy path is host-testable with a mock device and mock
board traits. The Pigweed wait/respond loop lives in a separate kernel-tagged
`otp-server-runtime` crate.

The server is the single choke point for the policy the wire deliberately does
**not** carry. Everything SoC-specific is supplied by the board through a trait;
the wire stays vendor-neutral.

## Responsibilities

| Concern | Owner | Notes |
|---|---|---|
| **Authorization** | `AccessPolicy` | Decides who may read a region, program raw bytes, or advance a floor. Denials answer `NotAuthorized` and never leak fuse contents. |
| **Anti-rollback monotonicity** | `dispatch` + `SvnCodec` | `CommitSvnFloor` reads the current floor and only ever advances it; a non-advance is an idempotent no-op. |
| **Fuse encoding & ordering** | `SvnCodec` | Owns the raw representation (one-hot, majority-vote, …) and the ordering used by the monotonic guard. The wire carries neither the encoding nor a value width. |
| **Fuse geometry** | `FieldMap` | Resolves a wire `FieldId` to this SoC's `(region, offset, len)`. Unknown ids answer `Unsupported`, so no fuse map is baked into the wire. |

**Fail-closed:** malformed, unsupported, denied, and hardware-error requests all
answer with an error header. `dispatch` never panics on any input.

## Board-supplied traits

```rust
pub trait AccessPolicy {
    fn can_read(&self, caller: CallerId, region: RegionId) -> bool;
    fn can_program(&self, caller: CallerId) -> bool;
    fn can_commit_svn(&self, caller: CallerId) -> bool;
}

pub trait SvnCodec {
    /// Is `candidate` strictly newer than the value in `current` (raw fuse bytes)?
    fn is_monotonic_advance(&self, field: FieldId, current: &[u8], candidate: &[u8]) -> bool;
    /// Encode `candidate` into `out` (the field's raw representation); returns bytes written.
    fn encode(&self, field: FieldId, candidate: &[u8], out: &mut [u8]) -> usize;
}

pub trait FieldMap {
    fn locate(&self, field: FieldId) -> Option<(RegionId, OtpOffset, usize)>;
}
```

`CallerId` is derived by the runtime from the channel a request arrived on —
trust is anchored to the channel, not to anything on the wire.

## Operations

Wire opcodes are defined in the `otp_api` crate; the server executes them as:

| `OtpOp` | Action | Privileged |
|---|---|---|
| `ReadBytes` | Raw read of `region` + `offset` + `len`. | no |
| `ReadField` | Named read: `FieldMap` resolves `FieldId`, then reads. | no |
| `ProgramBytes` | Raw program of an inline payload at `region` + `offset`. | yes |
| `CommitSvnFloor` | Advance a field's anti-rollback floor to the inline candidate value. | yes |

## Usage

```rust
use otp_server::{dispatch, AccessPolicy, CallerId, FieldMap, SvnCodec};

let mut resp = [0u8; otp_server::MAX_BUF_SIZE];
let n = dispatch(
    &mut otp_device, // impl OtpReadBytes<Region = RegionId> + OtpProgramBytes<Region = RegionId>
    &policy,         // impl AccessPolicy
    &codec,          // impl SvnCodec
    &field_map,      // impl FieldMap
    caller,          // CallerId, from the request's channel
    &request,        // wire bytes
    &mut resp,
);
// `resp[..n]` is the encoded response (always >= OtpResponseHeader::SIZE).
```

## Testing

```sh
bazelisk test //services/otp/server:otp_server_test
```

Host-only tests cover: named read, denied-region read, monotonic advance +
refuse-regress, missing authorization, and malformed-request rejection — all
without a kernel or QEMU.

## Related

- `//services/otp/api` — the wire protocol (`OtpOp`, `FieldId`, headers).
- `//hal/blocking/otp` — the HAL OTP device traits this server is generic over.
