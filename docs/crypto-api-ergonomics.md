# Crypto Client API Ergonomics Review

**Date:** 2026-02-13  
**Scope:** `services/crypto/client/src/lib.rs` and its callers  
**Status:** Current API works, passes all 7 QEMU tests. This review proposes improvements.

---

## Current Call-Site Experience

```rust
// Today: caller must import constants, manage output buffers, thread handles
use crypto_client::{sha256, hmac_sha256, aes_gcm_encrypt, aes_gcm_decrypt};
use crypto_api::{SHA256_OUTPUT_SIZE, SHA384_OUTPUT_SIZE};

let mut hash = [0u8; SHA256_OUTPUT_SIZE];
sha256(handle::CRYPTO, data, &mut hash).map_err(|_| Error::Internal)?;

let mut mac = [0u8; SHA256_OUTPUT_SIZE];
hmac_sha256(handle::CRYPTO, key, data, &mut mac).map_err(|_| Error::Internal)?;

let mut ct = [0u8; 64];
let ct_len = aes_gcm_encrypt(handle::CRYPTO, &key, &nonce, pt, &mut ct)
    .map_err(|_| Error::Internal)?;
// caller must track ct_len and slice ct[..ct_len]
```

---

## Issues

### 1. Raw handle threading (high friction)

Every function takes `handle: u32` as the first argument. The handle is always
`handle::CRYPTO` — an IPC channel obtained once at process startup and never
changed. Yet every call site must pass it explicitly.

**Impact:** Boilerplate, easy to pass the wrong handle, no type safety
(any `u32` compiles).

### 2. Free functions — no discoverability

The API is 11 free functions (`sha256`, `sha384`, `sha512`, `hmac_sha256`, ...).
A caller can't type `crypto.` and see the full API surface. They must know the
function names and import them individually.

### 3. Caller-managed output buffers with size constants

For fixed-output operations (hash, HMAC, sign), the caller must:
1. Import the size constant (`SHA256_OUTPUT_SIZE`)
2. Declare a zeroed buffer of that size
3. Pass a mutable reference

This is 3 lines of ceremony for what should be 1 line. The output size is
statically known from the operation — the API should encode that.

### 4. AES-GCM returns raw `usize`

```rust
let ct_len = aes_gcm_encrypt(..., &mut ct)?;
// What is ct[ct_len..]? Garbage. Caller must remember to slice.
```

The returned length is easy to lose or misuse. There's no type-level connection
between the output buffer and the valid region.

### 5. ECDSA verify: `Result<bool>` is a tri-state

```rust
let valid: bool = ecdsa_p256_verify(...)?;
if !valid { ... }
```

Three states: `Ok(true)`, `Ok(false)`, `Err(...)`. Callers must handle both the
`Result` and the `bool`. In every crypto library I've seen (ring, RustCrypto,
OpenSSL), verify returns `Result<()>` — failure **is** the error.

### 6. Error type is discarded at every call site

```rust
sha256(...).map_err(|_| Error::Internal)?;
//          ^^^^^^^^ every single call erases the error
```

`ClientError` has good variants (`ServerError(CryptoError)`, `InvalidResponse`,
etc.) but no caller uses them. The error type is doing work at the library level
but providing no value at the application level.

---

## Recommendations

### R1: Introduce `CryptoClient` struct (priority: high)

```rust
/// Typed handle to the crypto server.
///
/// Constructed once per process, stores the IPC channel handle.
/// All operations are methods on this type.
pub struct CryptoClient {
    handle: u32,
}

impl CryptoClient {
    /// Bind to the crypto server channel.
    pub const fn new(handle: u32) -> Self {
        Self { handle }
    }
}
```

**Why:** Single construction point. Handle stored once. Full API discoverable
via `client.` autocomplete. Zero runtime cost (the struct is a `u32` wrapper).

### R2: Return fixed-size arrays for hash/HMAC/sign (priority: high)

```rust
impl CryptoClient {
    /// Compute SHA-256 digest. Returns the 32-byte hash.
    pub fn sha256(&self, data: &[u8]) -> Result<[u8; 32], ClientError> { ... }

    /// Compute SHA-384 digest. Returns the 48-byte hash.
    pub fn sha384(&self, data: &[u8]) -> Result<[u8; 48], ClientError> { ... }

    /// Compute HMAC-SHA256. Returns the 32-byte tag.
    pub fn hmac_sha256(&self, key: &[u8], data: &[u8]) -> Result<[u8; 32], ClientError> { ... }
}
```

**Why:** No caller-side buffer management. No size constant imports. The return
type documents the output size. On Cortex-M, returning `[u8; 32]` by value is
cheap (registers + small stack copy). Even `[u8; 64]` (SHA-512) is fine — the
IPC buffer copy already dominates.

**Internal implementation:** The existing `parse_response` writes into a
`[u8; N]` — just move the buffer into the function and return it.

### R3: AES-GCM writes into caller buffer, returns `&[u8]` or length (priority: medium)

AES-GCM can't return a fixed-size array because the output size depends on input.
Two options:

**Option A — return written length (current approach, minimal change):**
```rust
pub fn aes256_gcm_seal(
    &self,
    key: &[u8; 32],
    nonce: &[u8; 12],
    plaintext: &[u8],
    out: &mut [u8],          // must be >= plaintext.len() + 16
) -> Result<usize, ClientError> { ... }
```

**Option B — write into a `SealedBuf` wrapper (more type-safe):**
```rust
pub struct SealedOutput<'a> {
    buf: &'a [u8],
}

impl<'a> SealedOutput<'a> {
    pub fn as_bytes(&self) -> &[u8] { self.buf }
    pub fn len(&self) -> usize { self.buf.len() }
}
```

Recommend **Option A** for now (no_std simplicity), but rename to `seal`/`open`
which is the standard AEAD terminology.

### R4: Verify returns `Result<()>`, not `Result<bool>` (priority: high)

```rust
#[cfg(feature = "ecdsa")]
impl CryptoClient {
    /// Verify an ECDSA P-256 signature.
    ///
    /// Returns `Ok(())` if the signature is valid.
    /// Returns `Err(ClientError::ServerError(VerificationFailed))` if invalid.
    pub fn ecdsa_p256_verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8; 64],
    ) -> Result<(), ClientError> { ... }
}
```

**Why:** Matches every major crypto library (ring, RustCrypto, OpenSSL, BoringSSL).
The current `Ok(false)` path maps directly to `Err(VerificationFailed)`. Callers
just write `client.ecdsa_p256_verify(...)?;` — one line, no boolean check.

**Server-side change required:** The server currently returns a 1-byte `0x01`/`0x00`
result. Change it to return an empty success response (result_len=0) for valid,
or a `VerificationFailed` error response for invalid. The client maps both to
`Result<()>`.

### R5: Keep free functions as thin wrappers (priority: low, optional)

For backward compatibility or one-off use, keep the free functions but have them
delegate to `CryptoClient`:

```rust
/// Convenience: compute SHA-256 without constructing a client.
pub fn sha256(handle: u32, data: &[u8]) -> Result<[u8; 32], ClientError> {
    CryptoClient::new(handle).sha256(data)
}
```

This is zero-cost (inlined away) and avoids breaking existing callers during
migration.

### R6: Consider `impl Display` for `ClientError` (priority: low)

Currently the test code does `.map_err(|_| Error::Internal)` everywhere. If
`ClientError` implemented a human-readable description, callers could log it
before converting:

```rust
impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IpcError(e) => write!(f, "IPC: {:?}", e),
            Self::ServerError(e) => write!(f, "server: {:?}", e),
            Self::InvalidResponse => write!(f, "malformed response"),
            Self::BufferTooSmall => write!(f, "buffer too small"),
        }
    }
}
```

---

## Proposed Call-Site Experience (After)

```rust
use crypto_client::CryptoClient;

let crypto = CryptoClient::new(handle::CRYPTO);

// Hash — one line, no buffer, no size constant
let hash = crypto.sha256(b"hello world")?;

// HMAC — returns tag directly
let tag = crypto.hmac_sha256(key, data)?;

// AEAD
let mut ct = [0u8; 64];
let ct_len = crypto.aes256_gcm_seal(&key, &nonce, plaintext, &mut ct)?;
let mut pt = [0u8; 64];
let pt_len = crypto.aes256_gcm_open(&key, &nonce, &ct[..ct_len], &mut pt)?;

// ECDSA — verify is Result<()>, invalid = error
let sig = crypto.ecdsa_p256_sign(&private_key, message)?;
crypto.ecdsa_p256_verify(&public_key, message, &sig)?; // Err on invalid
```

---

## Scope of Changes

| File | Change | Risk |
|------|--------|------|
| `client/src/lib.rs` | Add `CryptoClient` struct, convert free fns to methods, return arrays | Medium — all internals rewritten but wire protocol unchanged |
| `tests/src/main.rs` | Update call sites to use `CryptoClient` | Low — straightforward migration |
| `server/src/main.rs` | Change verify response from `[0x01]`/`[0x00]` to success/error | Low — 4 lines |
| `api/src/protocol.rs` | No change | None |
| `api/src/backend.rs` | No change | None |

**Wire protocol is unchanged.** The request/response header format stays the same.
Only the client-side API surface and the verify response convention change.

---

## Migration Path

1. Add `CryptoClient` struct with methods that delegate to existing internal functions
2. Convert internal functions (`hash_op`, `hmac_op`, `cipher_op`) to return values instead of writing to output params
3. Change verify to return `Result<()>`
4. Update test call sites
5. Deprecate (or remove) free functions
6. Build and run QEMU tests to verify
