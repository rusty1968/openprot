# Crypto Service Design Review

**Date:** February 14, 2026  
**Authors:** Steven  
**Status:** Draft  
**Scope:** Pigweed Crypto Service vs. Hubris Digest Server — architectural comparison and redesign proposal

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Systems Under Review](#2-systems-under-review)
3. [Hubris Digest Server Analysis](#3-hubris-digest-server-analysis)
4. [OpenPRoT HAL Trait Architecture](#4-openprot-hal-trait-architecture)
5. [Pigweed Crypto Service Analysis](#5-pigweed-crypto-service-analysis)
6. [Comparative Analysis](#6-comparative-analysis)
7. [Design Deficiencies — Hubris](#7-design-deficiencies--hubris)
8. [Design Deficiencies — Pigweed](#8-design-deficiencies--pigweed)
9. [Proposed Architecture](#9-proposed-architecture)
   - [9.10 Trait Layering: HAL vs. Service](#910-trait-layering-hal-vs-service)
   - [9.11 Decoupling Resource Recovery](#911-decoupling-resource-recovery)
10. [Migration Plan](#10-migration-plan)
11. [Appendix: Source Inventory](#11-appendix-source-inventory)
12. [Specification Gap Analysis](#12-specification-gap-analysis)

---

## 1. Executive Summary

This review compares two embedded crypto service implementations:

- **Hubris Digest Server** (`hubris/drv/digest-server/`) — a 1,354-line server built on the Oxide Computer Hubris RTOS, using the OpenPRoT HAL trait hierarchy and Idol IDL for IPC. Supports hardware (ASPEED HACE) and software (RustCrypto) backends via compile-time feature selection. Provides session-based *and* one-shot digest/HMAC operations.

- **Pigweed Crypto Service** (`services/crypto/`) — a 398-line server running on the Pigweed kernel, using RustCrypto crates directly with a manual wire protocol over channel IPC. Supports SHA-2, HMAC, AES-GCM, AES-CTR, and ECDSA P-256/P-384. One-shot only.

**Key Findings:**

| Finding | Hubris | Pigweed |
|---------|--------|---------|
| Backend abstraction | Yes (trait-based) | No (hardcoded RustCrypto) |
| Session/streaming API | Yes | No |
| Code duplication severity | **Critical** (6×6 boilerplate) | Moderate (14 similar functions) |
| Algorithm extensibility | Poor (God trait changes required) | Poor (touch 3+ files) |
| IPC mechanism | Generated (Idol) | Manual wire format |
| Feature coverage | Digest + HMAC only | Digest + HMAC + AEAD + ECDSA |
| Testability | Host-testable (RustCrypto backend) | QEMU-only |

**Recommendation:** Neither design is satisfactory for a production crypto service. We propose a unified architecture using algorithm marker types and the `OneShot<A>`/`Streaming<A>` trait pattern. This reduces server logic to ~120 lines, makes adding new algorithms a single-file change, and enables backend swappability with zero server modifications.

**Architectural Insight — Trait Layering:** The HAL layer (`hal/blocking/`) and service layer (`services/crypto/api/`) serve distinct purposes and should remain separate:

- **HAL traits** abstract hardware controllers — used by platform impls regardless of OS
- **Service traits** abstract IPC protocol — used by servers for backend dispatch

The HAL's `owned::DigestOp` pattern (typestate with resource recovery) should be **decoupled** from the core operation semantics. Resource recovery is valuable for baremetal code but adds friction when wrapping HAL controllers behind service traits. The proposed design uses a simple `&mut self` core trait with an optional `Owned<T>` wrapper for baremetal safety, eliminating the need for adapter layers.

---

## 2. Systems Under Review

### 2.1 Hubris Digest Server

| Component | File | Lines |
|-----------|------|-------|
| Server | `hubris/drv/digest-server/src/main.rs` | 1,354 |
| IDL definition | `hubris/idl/openprot-digest.idol` | 360 |
| Client API | `hubris/drv/openprot-digest-api/src/lib.rs` | 222 |
| HAL traits (digest) | `bazel-stuff/hal/blocking/src/digest.rs` | ~900 |
| HAL traits (mac) | `bazel-stuff/hal/blocking/src/mac.rs` | ~680 |
| Service backend trait | `bazel-stuff/platform/traits/hubris/src/lib.rs` | ~400 |
| RustCrypto backend | `bazel-stuff/platform/impls/rustcrypto/src/controller.rs` | 1,033 |
| Test client | `hubris/task/hmac-client/src/main.rs` | 208 |

**IPC Model:** Idol IDL-generated stubs with compile-time type checking. Uses leases for zero-copy data transfer (`[u8]` read leases, `[u32; N]` write leases). Server-side is `InOrder` (serialized request processing).

**Backend Model:** Compile-time feature selection (`aspeed-hace`, `rustcrypto`, `mock`). A `DefaultDigestDevice` type alias selects the concrete backend. The server is generic: `ServerImpl<D: HubrisDigestDevice>`.

### 2.2 Pigweed Crypto Service

| Component | File | Lines |
|-----------|------|-------|
| Server | `services/crypto/server/src/main.rs` | 398 |
| Wire protocol | `services/crypto/api/src/protocol.rs` | 265 |
| Client library | `services/crypto/client/src/lib.rs` | 509 |
| Integration tests | `services/crypto/tests/src/main.rs` | ~200 |

**IPC Model:** Manual wire protocol over Pigweed kernel channels. Request header (8 bytes): `op:u8 | flags:u8 | key_len:u16 | nonce_len:u16 | data_len:u16`. Response header (4 bytes): `status:u8 | reserved:u8 | result_len:u16`. Uses `zerocopy::FromBytes` for zero-copy header parsing.

**Backend Model:** Direct RustCrypto crate imports. No trait abstraction. No backend swappability.

---

## 3. Hubris Digest Server Analysis

### 3.1 Architecture

```
┌─────────────────┐    Idol IPC    ┌───────────────────┐   HAL Traits    ┌──────────────────┐
│   Client Task   │ ────────────── │    ServerImpl<D>   │ ────────────── │   Backend        │
│  task_slot!()   │  (generated)   │                    │                │ • HaceController │
│  Digest::from() │                │ • Controllers<D>   │                │ • RustCryptoCtrl │
│                 │                │ • DigestSession<D> │                │ • MockDigestCtrl │
│                 │                │ • SessionContext<D>│                └──────────────────┘
└─────────────────┘                └───────────────────┘
```

### 3.2 Session Lifecycle

The Hubris server implements a full session lifecycle for streaming large data:

```
Client                         Server                        Backend
  │                              │                              │
  │ init_sha256()                │                              │
  ├─────────────────────────────►│ take hardware controller     │
  │         session_id=1         │ init_digest_session_sha256() │
  │◄─────────────────────────────├────────────────────────────►│
  │                              │     CryptoSession created    │
  │ update(id=1, chunk1)         │                              │
  ├─────────────────────────────►│ session.update(chunk1)       │
  │                              ├────────────────────────────►│
  │                              │                              │
  │ update(id=1, chunk2)         │                              │
  ├─────────────────────────────►│ session.update(chunk2)       │
  │                              ├────────────────────────────►│
  │                              │                              │
  │ finalize_sha256(id=1)        │                              │
  ├─────────────────────────────►│ session.finalize()           │
  │        Digest<8>             │──►(output, controller)       │
  │◄─────────────────────────────│   hardware returned          │
  │                              │                              │
```

### 3.3 One-Shot Path

For small data that fits in a single lease buffer (≤1024 bytes):

```rust
// Client side — single IPC call
digest.digest_oneshot_sha256(data.len() as u32, data, &mut result)?;

// Server side — create, feed, finalize in one method
fn compute_sha256_oneshot(&mut self, ...) {
    let controller = self.controllers.hardware.take()?;
    let ctx = controller.init_digest_sha256()?;
    let ctx = ctx.update(data)?;
    let (digest, controller) = ctx.finalize()?;
    self.controllers.hardware = Some(controller);
    // write digest to lease
}
```

### 3.4 Device Recovery Invariant

The most important architectural property: **the hardware controller is always recovered**, even on error paths. This is ensured by the move-semantics of `DigestOp`:

```rust
pub trait DigestOp: Sized {
    type Controller;
    fn update(self, data: &[u8]) -> Result<Self, Self::Error>;          // consumes, returns self
    fn finalize(self) -> Result<(Self::Output, Self::Controller), ...>; // returns controller
    fn cancel(self) -> Self::Controller;                                 // returns controller
}
```

Every terminal state — success (`finalize`) or error (`cancel`) — yields the controller back. The Rust type system statically prevents forgetting to return it.

### 3.5 Data Types

**Digest output** uses const-generic word arrays:

```rust
#[repr(C)]
pub struct Digest<const N: usize> {
    pub value: [u32; N],
}
```

| Algorithm | Type | Bytes |
|-----------|------|-------|
| SHA-256 | `Digest<8>` | 32 |
| SHA-384 | `Digest<12>` | 48 |
| SHA-512 | `Digest<16>` | 64 |

**HMAC output** uses raw byte arrays: `[u8; 32]`, `[u8; 48]`, `[u8; 64]`. This type mismatch is a design flaw (see §7.4).

---

## 4. OpenPRoT HAL Trait Architecture

The OpenPRoT HAL defines a layered trait hierarchy for cryptographic operations:

### 4.1 Layer 1: Base Traits (`hal/blocking/`)

**Owned (move-semantic) API:**

```rust
// digest.rs — move-based init
pub trait DigestInit<T: DigestAlgorithm>: ErrorType + Sized {
    type Context: DigestOp<Output = Self::Output, Controller = Self>;
    type Output: IntoBytes;
    fn init(self, init_params: T) -> Result<Self::Context, Self::Error>;
}

// digest.rs — move-based operations
pub trait DigestOp: ErrorType + Sized {
    type Output: IntoBytes;
    type Controller;
    fn update(self, data: &[u8]) -> Result<Self, Self::Error>;
    fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error>;
    fn cancel(self) -> Self::Controller;
}

// mac.rs — move-based init
pub trait MacInit<A: MacAlgorithm>: ErrorType + Sized {
    type Key: KeyHandle;
    type Context: MacOp<Output = Self::Output, Controller = Self>;
    type Output: IntoBytes;
    fn init(self, algorithm: A, key: Self::Key) -> Result<Self::Context, Self::Error>;
}

// mac.rs — move-based operations
pub trait MacOp: ErrorType + Sized {
    type Output: IntoBytes;
    type Controller;
    fn update(self, data: &[u8]) -> Result<Self, Self::Error>;
    fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error>;
    fn cancel(self) -> Self::Controller;
}
```

**Scoped (borrow-semantic) API** — alternative API using `&mut self`:

```rust
pub trait DigestOp: ErrorType {  // Note: no Sized bound
    type Output: IntoBytes;
    fn update(&mut self, input: &[u8]) -> Result<(), Self::Error>;
    fn finalize(self) -> Result<Self::Output, Self::Error>;
}
```

The Hubris platform exclusively uses the **owned** API for session lifecycle management.

### 4.2 Layer 2: Crypto Service Trait (`platform/traits/hubris/`)

> **Important finding:** Despite the crate name `openprot-platform-traits-hubris` and
> doc comments claiming "Hubris IDL compatibility" and "Hubris task model," this trait
> has **zero Hubris dependencies**. Its only dependency is `openprot-hal-blocking`.
> There are no imports of `userlib`, `idol_runtime`, `task_slot!`, or any OS primitive.
> The trait is fully OS-agnostic and would work identically on Pigweed, Zephyr, or
> bare-metal. The name is misleading — it should be `openprot-platform-traits-crypto`.
>
> Furthermore, this is **not a platform trait at all**. A platform trait abstracts
> hardware capabilities (GPIO, timers, DMA). `HubrisDigestDevice` bundles 7 algorithm-
> specific associated types and 12 init methods that mirror the digest server's IDL
> operations 1:1. It is a **crypto service backend trait** — the contract between the
> digest server and its pluggable backend. It belongs alongside the server, not in
> `platform/traits/`.

```rust
pub trait HubrisDigestDevice {
    type DigestContext256: DigestOp<Controller = Self, Output = Digest<8>>;
    type DigestContext384: DigestOp<Controller = Self, Output = Digest<12>>;
    type DigestContext512: DigestOp<Controller = Self, Output = Digest<16>>;
    type HmacKey: for<'a> TryFrom<&'a [u8]>;
    type HmacContext256: MacOp<Controller = Self, Output = [u8; 32]>;
    type HmacContext384: MacOp<Controller = Self, Output = [u8; 48]>;
    type HmacContext512: MacOp<Controller = Self, Output = [u8; 64]>;

    const MAX_KEY_SIZE: usize = 128;

    // 6 direct init methods (consume self, create context)
    fn init_digest_sha256(self) -> Result<Self::DigestContext256, HubrisCryptoError>;
    fn init_digest_sha384(self) -> Result<Self::DigestContext384, HubrisCryptoError>;
    fn init_digest_sha512(self) -> Result<Self::DigestContext512, HubrisCryptoError>;
    fn init_hmac_sha256(self, key: Self::HmacKey) -> Result<Self::HmacContext256, HubrisCryptoError>;
    fn init_hmac_sha384(self, key: Self::HmacKey) -> Result<Self::HmacContext384, HubrisCryptoError>;
    fn init_hmac_sha512(self, key: Self::HmacKey) -> Result<Self::HmacContext512, HubrisCryptoError>;

    // 6 session init methods (consume self, return CryptoSession with device recovery)
    fn init_digest_session_sha256(self) -> Result<CryptoSession<Self::DigestContext256, Self>, HubrisCryptoError> where Self: Sized;
    // ... (sha384, sha512, hmac×3 variants)
}
```

### 4.3 Layer 3: RAII Session Wrapper

```rust
pub struct CryptoSession<Context, Device> {
    context: Option<Context>,
    device: Option<Device>,
}

impl<Context, Device> CryptoSession<Context, Device> {
    pub fn new(context: Context, device: Device) -> Self;

    // Digest path (Context: DigestOp)
    pub fn update(mut self, data: &[u8]) -> Result<Self, HubrisCryptoError>;
    pub fn finalize(mut self) -> Result<(Context::Output, Device), HubrisCryptoError>;

    // MAC path (Context: MacOp)
    pub fn update_mac(mut self, data: &[u8]) -> Result<Self, HubrisCryptoError>;
    pub fn finalize_mac(mut self) -> Result<(Context::Output, Device), HubrisCryptoError>;
}
```

**Critical observation:** `CryptoSession` has no `Drop` implementation. If a session is dropped without calling `finalize()`, the device (`Option<Device>`) is silently discarded. The RAII recovery guarantee documented in the design exists only conceptually, not in code.

### 4.4 Layer 4: Backend Implementations

**RustCrypto backend** (`platform/impls/rustcrypto/`):

```rust
pub struct RustCryptoController {} // Non-cloneable, empty struct

pub struct DigestContext256(sha2::Sha256);   // Newtype wrappers
pub struct DigestContext384(sha2::Sha384);
pub struct DigestContext512(sha2::Sha512);
pub struct MacContext256(Hmac<Sha256>);
pub struct MacContext384(Hmac<Sha384>);
pub struct MacContext512(Hmac<Sha512>);

// Key type: stack-allocated [u8; 128] with Zeroize-on-Drop
pub struct SecureOwnedKey { ... }
```

The RustCrypto controller is an empty struct — `cancel()` and the device-recovery path simply call `RustCryptoController::new()` since software backends have no hardware state to recover. This means the entire owned-API device-recovery machinery is **pure overhead** for software backends.

---

## 5. Pigweed Crypto Service Analysis

### 5.1 Architecture

```
┌──────────────┐   channel_transact   ┌──────────────────┐   Direct calls   ┌──────────────┐
│  Client App  │ ───────────────────── │  Crypto Server   │ ──────────────── │  RustCrypto  │
│              │   manual wire fmt     │  (flat dispatch) │                  │  Crates      │
│  crypto_     │   CryptoRequest/     │                  │    sha2          │              │
│  client::*() │   ResponseHeader     │  dispatch_       │    hmac          │              │
│              │                      │    crypto_op()   │    aes-gcm       │              │
│              │                      │    → do_sha256() │    p256, p384    │              │
│              │                      │    → do_hmac*()  │                  │              │
│              │                      │    → do_aes*()   │                  │              │
│              │                      │    → do_ecdsa*() │                  │              │
└──────────────┘                      └──────────────────┘                  └──────────────┘
```

### 5.2 Wire Protocol

**Request** (8-byte header + variable payload):

```
 0       1       2       3       4       5       6       7
+-------+-------+-------+-------+-------+-------+-------+-------+
|  op   | flags |  key_len (LE) |nonce_len (LE) | data_len (LE) |
+-------+-------+-------+-------+-------+-------+-------+-------+
| key (key_len bytes) | nonce (nonce_len bytes) | data (data_len)|
+---------------------+------------------------+-----------------+
```

**Response** (4-byte header + variable payload):

```
 0       1       2       3
+-------+-------+-------+-------+
|status |reservd|result_len (LE)|
+-------+-------+-------+-------+
| result (result_len bytes)     |
+-------------------------------+
```

The protocol is simple, efficient, and `zerocopy`-compatible. It uses fixed-size headers that can be parsed with zero allocation.

### 5.3 Server Dispatch

```rust
fn dispatch_crypto_op(request: &[u8], response: &mut [u8]) -> usize {
    let header = parse_header(request)?;
    let (key, nonce, data) = extract_payload(request, &header);

    match op {
        CryptoOp::Sha256Hash      => do_sha256(data, response),
        CryptoOp::Sha384Hash      => do_sha384(data, response),
        CryptoOp::Sha512Hash      => do_sha512(data, response),
        CryptoOp::HmacSha256      => do_hmac_sha256(key, data, response),
        CryptoOp::HmacSha384      => do_hmac_sha384(key, data, response),
        CryptoOp::HmacSha512      => do_hmac_sha512(key, data, response),
        CryptoOp::Aes256GcmEncrypt => do_aes_gcm_encrypt(key, nonce, data, response),
        CryptoOp::Aes256GcmDecrypt => do_aes_gcm_decrypt(key, nonce, data, response),
        CryptoOp::Aes256CtrEncrypt => do_aes_ctr(key, nonce, data, response),
        CryptoOp::Aes256CtrDecrypt => do_aes_ctr(key, nonce, data, response),
        CryptoOp::EcdsaP256Sign    => do_ecdsa_p256_sign(key, data, response),
        CryptoOp::EcdsaP256Verify  => do_ecdsa_p256_verify(key, data, nonce, response),
        CryptoOp::EcdsaP384Sign    => do_ecdsa_p384_sign(key, data, response),
        CryptoOp::EcdsaP384Verify  => do_ecdsa_p384_verify(key, data, nonce, response),
    }
}
```

### 5.4 Supported Operations

| Category | Operations | Key Sizes | Output |
|----------|-----------|-----------|--------|
| Digest | SHA-256, SHA-384, SHA-512 | N/A | 32, 48, 64 bytes |
| MAC | HMAC-SHA-256/384/512 | Variable | 32, 48, 64 bytes |
| AEAD | AES-256-GCM encrypt/decrypt | 32 bytes | CT+tag / PT |
| Stream cipher | AES-256-CTR | 32 bytes, IV 16 bytes | Same length |
| Signatures | ECDSA P-256/P-384 sign/verify | 32/48 bytes | 64/96 bytes |

### 5.5 Client Library

The client library (`crypto_client`) provides typed wrappers:

```rust
pub fn sha256(handle: u32, data: &[u8], output: &mut [u8; 32]) -> Result<(), ClientError>;
pub fn hmac_sha256(handle: u32, key: &[u8], data: &[u8], output: &mut [u8; 32]) -> Result<(), ClientError>;
pub fn aes_gcm_encrypt(handle: u32, key: &[u8;32], nonce: &[u8;12], ...) -> Result<usize, ClientError>;
pub fn ecdsa_p256_sign(handle: u32, private_key: &[u8;32], msg: &[u8], sig: &mut [u8;64]) -> ...;
pub fn ecdsa_p256_verify(handle: u32, pubkey: &[u8], msg: &[u8], sig: &[u8;64]) -> Result<bool, ...>;
```

Each function builds the request header, serializes `key || nonce || data`, calls `channel_transact`, and parses the response. The functions are generic over output size using `parse_response<const N: usize>()`.

---

## 6. Comparative Analysis

### 6.1 Feature Matrix

| Capability | Hubris Digest Server | Pigweed Crypto Service |
|------------|---------------------|------------------------|
| Hash (SHA-2 family) | ✅ SHA-256/384/512 | ✅ SHA-256/384/512 |
| HMAC | ✅ SHA-256/384/512 | ✅ SHA-256/384/512 |
| AEAD | ❌ | ✅ AES-256-GCM |
| Stream cipher | ❌ | ✅ AES-256-CTR |
| Digital signatures | ❌ | ✅ ECDSA P-256/P-384 |
| Session/streaming | ✅ init→update×N→finalize | ❌ One-shot only |
| Backend abstraction | ✅ Trait-based | ❌ Hardcoded |
| Hardware backend | ✅ ASPEED HACE | ❌ |
| Zero-copy IPC | ✅ Leases | ❌ Copy-based |
| IDL code generation | ✅ Idol | ❌ Manual |
| Host testability | ✅ RustCrypto backend (no HW needed) | ❌ QEMU required |

### 6.2 Code Metrics

| Metric | Hubris | Pigweed |
|--------|--------|---------|
| Server lines | 1,354 | 398 |
| Unique logic lines (est.) | ~200 | ~200 |
| Boilerplate ratio | **85%** | **50%** |
| Lines to add SHA3 | ~200 (6 methods) | ~30 (2 functions) |
| Lines to add BLAKE3 | ~200 (6 methods) | ~15 (1 function) |
| Lines to add new backend | ~200 (new impl) | Server rewrite |

### 6.3 IPC Comparison

| Aspect | Hubris (Idol) | Pigweed (Manual) |
|--------|---------------|------------------|
| Type safety | Compile-time (generated stubs) | Runtime (manual parsing) |
| Lease / zero-copy | Yes (`read`/`write` leases) | No (copy to/from channel) |
| Task discovery | `task_slot!` macro, compile-time | Handle table (`system.json5`) |
| Error propagation | `Result<T, DigestError>` generated | Manual header status byte |
| Versioning / compat | IDL revision | Ad-hoc op-code numbering |
| Data size limits | Lease `max_len` enforcement | Manual `MAX_PAYLOAD_SIZE` check |
| Overhead | ~0 (generated inline code) | ~0 (8-byte header parse) |

---

## 7. Design Deficiencies — Hubris

### 7.1 Catastrophic Code Duplication

The server has **18 near-identical method bodies** organized in a 6×3 matrix:

| Operation | SHA-256 | SHA-384 | SHA-512 |
|-----------|---------|---------|---------|
| Session init | `init_sha256_internal` | `init_sha384_internal` | `init_sha512_internal` |
| HMAC init | `init_hmac_sha256_internal` | `init_hmac_sha384_internal` | `init_hmac_sha512_internal` |
| Session finalize | `finalize_sha256` | `finalize_sha384` | `finalize_sha512` |
| HMAC finalize | `finalize_hmac_sha256` | `finalize_hmac_sha384` | `finalize_hmac_sha512` |
| Oneshot digest | `compute_sha256_oneshot` | `compute_sha384_oneshot` | `compute_sha512_oneshot` |
| HMAC oneshot | `hmac_oneshot_sha256` | `hmac_oneshot_sha384` | `hmac_oneshot_sha512` |

Each row contains 3 methods that differ **only** in:
- The associated type selected (`DigestContext256` vs `384` vs `512`)
- The session variant (`SessionContext::Sha256` vs `Sha384` vs `Sha512`)
- The output size constant

The `update_internal` method has a 6-arm match where every arm does `session.update(data)` — literally the same operation regardless of variant.

**Impact:** Adding SHA3 support requires ~200 lines of pure boilerplate across 6 new methods. A bug fix in the init sequence must be applied to 6 locations.

### 7.2 Misplaced Abstraction Layer

`HubrisDigestDevice` sits in `platform/traits/` but is not a platform abstraction. The actual platform/HAL traits are `DigestOp` and `MacOp` in `hal/blocking/` — those abstract what a crypto hardware block can do, independent of any server. `HubrisDigestDevice` is the digest server's backend contract: it says "to run this server, implement these 12 methods with these 7 context types." This is application-level coupling masquerading as platform abstraction.

### 7.3 God Trait: `HubrisDigestDevice`

The crypto service backend trait bundles **all** algorithms into a single monolithic interface:

```rust
pub trait HubrisDigestDevice {
    type DigestContext256: DigestOp<...>;   // 7 associated types
    type DigestContext384: DigestOp<...>;
    type DigestContext512: DigestOp<...>;
    type HmacKey: TryFrom<&[u8]>;
    type HmacContext256: MacOp<...>;
    type HmacContext384: MacOp<...>;
    type HmacContext512: MacOp<...>;
    // 12 methods
}
```

This is an **interface segregation violation**. A backend implementing only SHA-256 must still provide all 7 associated types and 12 methods. Adding AEAD or ECDSA support would require modifying:
1. The `HubrisDigestDevice` trait (new types + methods)
2. Every backend impl (`RustCryptoController`, `HaceController`, `MockDigestController`)
3. The `SessionContext` enum (new variants)
4. The `ServerImpl` (6+ new methods per algorithm class)

### 7.4 Four Layers of `Option` to Track One Session

```
                    current_session: Option<                     // Layer 1: is any session active?
                        DigestSession {
                            context: SessionContext::Sha256(
                                Option<                          // Layer 2: is this variant occupied?
                                    CryptoSession {
                                        context: Option<Ctx>,    // Layer 3: internal take/put
                                        device: Option<Dev>,     // Layer 4: internal take/put
                                    }
                                >
                            )
                        }
                    >
```

This quadruple-nesting exists because `CryptoSession` uses an internal `Option` dance (`take()`/`put()`) to work around the borrow checker for its move-based `update` → consume self → return new self pattern. But since `CryptoSession` already consumes `self` in `update()`, the internal `Option` is redundant given proper ownership handling.

### 7.5 Digest vs. HMAC Output Type Mismatch

Digests return `Digest<N>` (a `[u32; N]` word array) while HMACs return `[u8; M]` (a byte array). This forces the server to include a manual byte-to-word conversion:

```rust
let mut u32_result = [0u32; 8];
for (i, chunk) in result.chunks(4).enumerate() {
    u32_result[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
}
```

This 4-line pattern is repeated **6 times** (once per HMAC algorithm × once per finalize + oneshot). If HMAC output were also `Digest<N>`, this conversion would be unnecessary and the HMAC finalize methods could share code paths with digest finalize.

### 7.6 Missing `Drop` on `CryptoSession`

The `CryptoSession` struct wraps `Option<Device>` but has **no `Drop` implementation** in the actual source code. The design documentation claims RAII recovery, but if a `CryptoSession` is dropped (e.g., on panic or early return), the device is silently leaked. For hardware backends, this means permanent controller lockout until system restart.

### 7.7 No Session Timeout

```rust
struct DigestSession<D: HubrisDigestDevice> {
    created_at: u64,  // stored but NEVER CHECKED
    ...
}
```

A crashed or malicious client that calls `init_sha256()` but never `finalize()` will permanently lock the server. There is no watchdog, no keepalive, no administrative cancel command in the IDL.

### 7.8 Software Backend Overhead

For `RustCryptoController` (an empty `struct {}`):
- `cancel(self) -> Self::Controller` returns `RustCryptoController::new()` — a no-op constructor
- The entire owned-API device-recovery dance adds zero value for software backends
- The `SecureOwnedKey` type allocates 128 bytes on the stack regardless of actual key size

This is not a fatal flaw (the overhead is small), but it reveals a leaky abstraction: the API is designed for hardware single-controller semantics and imposes its cost model on all backends.

---

## 8. Design Deficiencies — Pigweed

### 8.1 Hardcoded Backend

The server directly imports crate-level types:

```rust
use sha2::{Digest, Sha256, Sha384, Sha512};
use hmac::{Hmac, Mac};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce as GcmNonce};
use p256::ecdsa::{SigningKey as P256SigningKey, ...};
```

Switching to hardware crypto (e.g., ASPEED HACE) requires rewriting the server. There is no trait abstraction, no feature gating, no pluggability.

### 8.2 Flat Function Explosion

14 separate `do_*` functions with repeated patterns:

```rust
fn do_sha256(data: &[u8], response: &mut [u8]) -> usize {
    let mut hasher = Sha256::new();   // Only this line differs
    hasher.update(data);
    let result = hasher.finalize();
    encode_success(response, &result[..SHA256_OUTPUT_SIZE])
}

fn do_sha384(data: &[u8], response: &mut [u8]) -> usize {
    let mut hasher = Sha384::new();   // Only this line differs
    hasher.update(data);
    let result = hasher.finalize();
    encode_success(response, &result[..SHA384_OUTPUT_SIZE])
}
// ... and so on for sha512
```

The SHA functions are *identical* except for the hasher type and output size. The HMAC functions are identical except for the type alias. These should be unified through generics.

### 8.3 No Streaming / Session Support

The server processes each request in a single `dispatch_crypto_op` call. There is no concept of a session, init/update/finalize lifecycle, or streaming. This means:

- Data larger than `MAX_PAYLOAD_SIZE` (512 bytes) cannot be hashed
- The entire message must fit in the IPC buffer
- Firmware image verification (common embedded use case) is impossible

The `flags` field in `CryptoRequestHeader` is defined but always set to 0 — it could carry session semantics.

### 8.4 Semantic Field Overloading

The wire protocol uses three generic fields (`key`, `nonce`, `data`) for all operation types. This leads to semantic mismatches:

| Operation | `key` field | `nonce` field | `data` field |
|-----------|------------|---------------|-------------|
| Hash | unused | unused | message |
| HMAC | HMAC key | unused | message |
| AES-GCM | AES key | GCM nonce | plaintext |
| AES-CTR | AES key | CTR IV | plaintext |
| ECDSA sign | private key | unused | message |
| **ECDSA verify** | **public key** | **signature** (!) | message |

ECDSA verify stuffs the **signature** into the `nonce` field. This is semantically wrong and confusing — a signature is not a nonce. The comment in the client says `// We use: key=pubkey, nonce=signature, data=message`, acknowledging the hack.

### 8.5 No Constant-Time HMAC Verification

The server produces HMAC tags but provides no `verify` operation. Clients must compare tags themselves, risking timing side-channel attacks if they use `==` instead of constant-time comparison. The Hubris IDL defines `hmac_verify_*` operations; the Pigweed service does not.

### 8.6 Not Host-Testable

All tests require QEMU because the server directly uses kernel syscalls (`object_wait`, `channel_read`, `channel_respond`). There is no way to test crypto logic independently of the IPC layer.

---

## 9. Proposed Architecture

### 9.1 Design Principles

1. **Algorithm = type parameter, not enum variant** — let the compiler monomorphize dispatch
2. **Backend = trait bound, not import** — swap implementations via generics
3. **Separate concerns** — wire format ↔ dispatch ↔ crypto logic ↔ backend
4. **One-shot first, session optional** — one-shot is the common case; sessions are additive
5. **Const-generic output sizes** — encode output dimensions in the type system

### 9.2 Crate Structure

```
services/crypto/
├── traits/         # NEW: no_std, no dependencies
│   └── src/lib.rs  # Algorithm, OneShot<A>, Streaming<A>, CryptoInput
├── backend-rustcrypto/   # NEW: OneShot<*> impls for RustCrypto
│   └── src/lib.rs
├── api/            # KEEP: wire protocol (unchanged)
│   └── src/protocol.rs
├── server/         # REWRITE: generic CryptoServer<B>
│   └── src/main.rs
├── client/         # KEEP: client library (unchanged)
│   └── src/lib.rs
└── tests/          # KEEP: integration tests (unchanged)
    └── src/main.rs
```

### 9.3 Algorithm Marker Types

```rust
// services/crypto/traits/src/lib.rs
#![no_std]

/// Marker trait for crypto algorithms. Each algorithm is a zero-sized type.
pub trait Algorithm {
    /// The fixed output size in bytes (0 for variable-output operations like AEAD).
    const OUTPUT_SIZE: usize;
    /// The wire protocol op code.
    const OP_CODE: u8;
}

// --- Digest algorithms ---

pub struct Sha256;
impl Algorithm for Sha256 { const OUTPUT_SIZE: usize = 32; const OP_CODE: u8 = 0x01; }

pub struct Sha384;
impl Algorithm for Sha384 { const OUTPUT_SIZE: usize = 48; const OP_CODE: u8 = 0x02; }

pub struct Sha512;
impl Algorithm for Sha512 { const OUTPUT_SIZE: usize = 64; const OP_CODE: u8 = 0x03; }

// --- MAC algorithms ---

pub struct HmacSha256;
impl Algorithm for HmacSha256 { const OUTPUT_SIZE: usize = 32; const OP_CODE: u8 = 0x10; }

pub struct HmacSha384;
impl Algorithm for HmacSha384 { const OUTPUT_SIZE: usize = 48; const OP_CODE: u8 = 0x11; }

pub struct HmacSha512;
impl Algorithm for HmacSha512 { const OUTPUT_SIZE: usize = 64; const OP_CODE: u8 = 0x12; }

// --- AEAD algorithms ---

pub struct Aes256GcmEncrypt;
impl Algorithm for Aes256GcmEncrypt { const OUTPUT_SIZE: usize = 0; const OP_CODE: u8 = 0x20; }

pub struct Aes256GcmDecrypt;
impl Algorithm for Aes256GcmDecrypt { const OUTPUT_SIZE: usize = 0; const OP_CODE: u8 = 0x21; }

// --- Stream cipher algorithms ---

pub struct Aes256Ctr;
impl Algorithm for Aes256Ctr { const OUTPUT_SIZE: usize = 0; const OP_CODE: u8 = 0x30; }

// --- Signature algorithms ---

pub struct EcdsaP256Sign;
impl Algorithm for EcdsaP256Sign { const OUTPUT_SIZE: usize = 64; const OP_CODE: u8 = 0x40; }

pub struct EcdsaP256Verify;
impl Algorithm for EcdsaP256Verify { const OUTPUT_SIZE: usize = 1; const OP_CODE: u8 = 0x41; }

pub struct EcdsaP384Sign;
impl Algorithm for EcdsaP384Sign { const OUTPUT_SIZE: usize = 96; const OP_CODE: u8 = 0x42; }

pub struct EcdsaP384Verify;
impl Algorithm for EcdsaP384Verify { const OUTPUT_SIZE: usize = 1; const OP_CODE: u8 = 0x43; }
```

### 9.4 Structured Input Type

Replace the flat `key || nonce || data` concatenation with a semantically typed enum:

```rust
/// Structured crypto input — each variant carries exactly the fields
/// its operation class needs. No more stuffing signatures into "nonce".
pub enum CryptoInput<'a> {
    /// Hash operations (SHA-*): just the message data.
    Digest { data: &'a [u8] },

    /// MAC operations (HMAC-*): key + message data.
    Mac { key: &'a [u8], data: &'a [u8] },

    /// AEAD operations (AES-GCM): key + nonce + plaintext/ciphertext.
    /// For decrypt: data = ciphertext || tag.
    Aead { key: &'a [u8], nonce: &'a [u8], data: &'a [u8] },

    /// Stream cipher (AES-CTR): key + IV + data.
    StreamCipher { key: &'a [u8], iv: &'a [u8], data: &'a [u8] },

    /// Signing: private key + message.
    Sign { private_key: &'a [u8], message: &'a [u8] },

    /// Verification: public key + message + signature.
    Verify { public_key: &'a [u8], message: &'a [u8], signature: &'a [u8] },
}

impl<'a> CryptoInput<'a> {
    /// Construct from parsed wire format header + payload.
    /// This is the ONLY place that knows about the key/nonce/data encoding.
    pub fn from_wire(op: CryptoOp, key: &'a [u8], nonce: &'a [u8], data: &'a [u8]) -> Self {
        match op {
            CryptoOp::Sha256Hash | CryptoOp::Sha384Hash | CryptoOp::Sha512Hash =>
                CryptoInput::Digest { data },
            CryptoOp::HmacSha256 | CryptoOp::HmacSha384 | CryptoOp::HmacSha512 =>
                CryptoInput::Mac { key, data },
            CryptoOp::Aes256GcmEncrypt | CryptoOp::Aes256GcmDecrypt =>
                CryptoInput::Aead { key, nonce, data },
            CryptoOp::Aes256CtrEncrypt | CryptoOp::Aes256CtrDecrypt =>
                CryptoInput::StreamCipher { key, iv: nonce, data },
            CryptoOp::EcdsaP256Sign | CryptoOp::EcdsaP384Sign =>
                CryptoInput::Sign { private_key: key, message: data },
            CryptoOp::EcdsaP256Verify | CryptoOp::EcdsaP384Verify =>
                CryptoInput::Verify { public_key: key, message: data, signature: nonce },
        }
    }
}
```

### 9.5 Backend Traits

```rust
/// One-shot crypto operation trait. One impl per (Backend, Algorithm) pair.
///
/// The backend is `&self` (not consumed) because software backends are stateless.
/// Hardware backends that need exclusive access should use internal `RefCell`
/// or be wrapped in an `Option<HwController>` at the server level.
pub trait OneShot<A: Algorithm> {
    type Error;

    /// Execute a one-shot crypto operation.
    ///
    /// `input`:  structured crypto input matching the algorithm class.
    /// `output`: mutable buffer for the result (at least `A::OUTPUT_SIZE` bytes,
    ///           or `data.len() + TAG_SIZE` for AEAD).
    ///
    /// Returns the number of bytes written to `output`.
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, Self::Error>;
}

/// Session-based streaming trait (optional — only needed for large data).
pub trait Streaming<A: Algorithm> {
    type Session;
    type Error;

    fn begin(&mut self) -> Result<Self::Session, Self::Error>;
    fn feed(&mut self, session: &mut Self::Session, data: &[u8]) -> Result<(), Self::Error>;
    fn finish(&mut self, session: Self::Session, output: &mut [u8]) -> Result<usize, Self::Error>;
    fn cancel(&mut self, session: Self::Session);
}
```

### 9.6 RustCrypto Backend

```rust
// services/crypto/backend-rustcrypto/src/lib.rs
#![no_std]

use crypto_traits::{Algorithm, CryptoInput, OneShot, CryptoError};
use crypto_traits::{Sha256, Sha384, Sha512, HmacSha256, HmacSha384, HmacSha512};
use crypto_traits::{Aes256GcmEncrypt, Aes256GcmDecrypt, Aes256Ctr};
use crypto_traits::{EcdsaP256Sign, EcdsaP256Verify, EcdsaP384Sign, EcdsaP384Verify};

pub struct RustCryptoBackend;

// --- Generic digest helper (eliminates do_sha256/384/512 duplication) ---

fn do_digest<D: sha2::Digest>(data: &[u8], output: &mut [u8]) -> Result<usize, CryptoError> {
    let result = D::digest(data);
    let size = result.len();
    output[..size].copy_from_slice(&result);
    Ok(size)
}

impl OneShot<Sha256> for RustCryptoBackend {
    type Error = CryptoError;
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Digest { data } = input else { return Err(CryptoError::InvalidOperation) };
        do_digest::<sha2::Sha256>(data, output)
    }
}

impl OneShot<Sha384> for RustCryptoBackend {
    type Error = CryptoError;
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Digest { data } = input else { return Err(CryptoError::InvalidOperation) };
        do_digest::<sha2::Sha384>(data, output)
    }
}

impl OneShot<Sha512> for RustCryptoBackend {
    type Error = CryptoError;
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Digest { data } = input else { return Err(CryptoError::InvalidOperation) };
        do_digest::<sha2::Sha512>(data, output)
    }
}

// --- Generic HMAC helper (eliminates do_hmac_sha256/384/512 duplication) ---

fn do_hmac<D: hmac::digest::core_api::CoreProxy>(
    key: &[u8], data: &[u8], output: &mut [u8]
) -> Result<usize, CryptoError>
where
    hmac::Hmac<D>: hmac::Mac,
{
    use hmac::Mac;
    let mut mac = <hmac::Hmac<D> as Mac>::new_from_slice(key)
        .map_err(|_| CryptoError::InvalidKeyLength)?;
    mac.update(data);
    let result = mac.finalize().into_bytes();
    let size = result.len();
    output[..size].copy_from_slice(&result);
    Ok(size)
}

// ... (similar compact impls for HmacSha256, AES-GCM, ECDSA)

// Adding BLAKE3 is exactly this:
//
// impl OneShot<Blake3> for RustCryptoBackend {
//     type Error = CryptoError;
//     fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
//         let CryptoInput::Digest { data } = input else { return Err(CryptoError::InvalidOperation) };
//         let hash = blake3::hash(data);
//         output[..32].copy_from_slice(hash.as_bytes());
//         Ok(32)
//     }
// }
```

### 9.7 Generic Server

```rust
// services/crypto/server/src/main.rs
pub struct CryptoServer<B> {
    backend: B,
}

impl<B> CryptoServer<B>
where
    B: OneShot<Sha256, Error = CryptoError>
     + OneShot<Sha384, Error = CryptoError>
     + OneShot<Sha512, Error = CryptoError>
     + OneShot<HmacSha256, Error = CryptoError>
     + OneShot<HmacSha384, Error = CryptoError>
     + OneShot<HmacSha512, Error = CryptoError>
     + OneShot<Aes256GcmEncrypt, Error = CryptoError>
     + OneShot<Aes256GcmDecrypt, Error = CryptoError>
     + OneShot<Aes256Ctr, Error = CryptoError>
     + OneShot<EcdsaP256Sign, Error = CryptoError>
     + OneShot<EcdsaP256Verify, Error = CryptoError>
     + OneShot<EcdsaP384Sign, Error = CryptoError>
     + OneShot<EcdsaP384Verify, Error = CryptoError>
{
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// The entire dispatch logic. Compare with the current 300+ line dispatch.
    pub fn dispatch(&self, request: &[u8], response: &mut [u8]) -> usize {
        let (header, key, nonce, data) = match parse_request(request) {
            Ok(parsed) => parsed,
            Err(e) => return encode_error(response, e),
        };

        let input = CryptoInput::from_wire(header.operation().unwrap(), key, nonce, data);

        match header.operation().unwrap() {
            CryptoOp::Sha256Hash       => self.run::<Sha256>(&input, response),
            CryptoOp::Sha384Hash       => self.run::<Sha384>(&input, response),
            CryptoOp::Sha512Hash       => self.run::<Sha512>(&input, response),
            CryptoOp::HmacSha256       => self.run::<HmacSha256>(&input, response),
            CryptoOp::HmacSha384       => self.run::<HmacSha384>(&input, response),
            CryptoOp::HmacSha512       => self.run::<HmacSha512>(&input, response),
            CryptoOp::Aes256GcmEncrypt => self.run::<Aes256GcmEncrypt>(&input, response),
            CryptoOp::Aes256GcmDecrypt => self.run::<Aes256GcmDecrypt>(&input, response),
            CryptoOp::Aes256CtrEncrypt => self.run::<Aes256Ctr>(&input, response),
            CryptoOp::Aes256CtrDecrypt => self.run::<Aes256Ctr>(&input, response),
            CryptoOp::EcdsaP256Sign    => self.run::<EcdsaP256Sign>(&input, response),
            CryptoOp::EcdsaP256Verify  => self.run::<EcdsaP256Verify>(&input, response),
            CryptoOp::EcdsaP384Sign    => self.run::<EcdsaP384Sign>(&input, response),
            CryptoOp::EcdsaP384Verify  => self.run::<EcdsaP384Verify>(&input, response),
        }
    }

    /// Generic one-shot dispatch — ONE function for all algorithms.
    fn run<A: Algorithm>(&self, input: &CryptoInput, response: &mut [u8]) -> usize
    where
        B: OneShot<A, Error = CryptoError>,
    {
        let result_start = CryptoResponseHeader::SIZE;
        match self.backend.compute(input, &mut response[result_start..]) {
            Ok(len) => {
                let header = CryptoResponseHeader::success(len as u16);
                response[..CryptoResponseHeader::SIZE]
                    .copy_from_slice(zerocopy::IntoBytes::as_bytes(&header));
                result_start + len
            }
            Err(e) => encode_error(response, e),
        }
    }
}
```

**Total server dispatch logic: ~50 lines.** The remaining ~70 lines are the event loop, request parsing, and error encoding — shared infrastructure that doesn't change when algorithms are added.

### 9.8 Session Extension (Future)

The wire protocol already has an unused `flags` byte. Define session semantics:

```
flags bit 0:    0 = one-shot, 1 = session operation
flags bits 1-2: 00 = init, 01 = update, 10 = finalize, 11 = cancel
flags bits 3-7: reserved
```

The server would add:

```rust
if header.flags & 0x01 != 0 {
    let session_op = (header.flags >> 1) & 0x03;
    match session_op {
        0 => self.session_init(op, key, response),
        1 => self.session_update(header.data_length() as u32, data, response),
        2 => self.session_finalize(op, response),
        3 => self.session_cancel(response),
        _ => encode_error(response, CryptoError::InvalidOperation),
    }
} else {
    self.dispatch_oneshot(op, &input, response)
}
```

This requires no wire format changes, no client library changes for one-shot users, and no server changes for existing algorithms.

### 9.9 Architecture Diagram

```
                         ┌─────────────────────────────────────────┐
                         │          crypto-traits crate            │
                         │   (no_std, zero dependencies)           │
                         │                                         │
                         │  pub trait Algorithm { OUTPUT_SIZE, .. } │
                         │  pub trait OneShot<A> { compute() }     │
                         │  pub trait Streaming<A> { begin/feed/.. }│
                         │  pub enum CryptoInput { Digest, Mac, .. }│
                         │                                         │
                         │  Sha256, Sha384, HmacSha256, ...        │
                         │  Aes256GcmEncrypt, EcdsaP256Sign, ...   │
                         └────────────┬────────────────────────────┘
                                      │
                    ┌─────────────────┴─────────────────┐
                    │                                   │
          ┌─────────▼──────┐                 ┌──────────▼─────────┐
          │  RustCrypto    │                 │  ASPEED HACE       │
          │  Backend       │                 │  Backend           │
          │  (host + target)│                │  (target only)     │
          │                │                 │                    │
          │ impl OneShot   │                 │ impl OneShot       │
          │    <Sha256>    │                 │    <Sha256>        │
          │    <HmacSha256>│                 │    <HmacSha256>    │
          │    <AesGcm..>  │                 │    ...             │
          │    <Ecdsa..>   │                 │                    │
          └───────┬────────┘                 └──────────┬─────────┘
                  │                                     │
                  └──────────────┬───────────────────────┘
                                     │
                          ┌──────────▼──────────┐
                          │  CryptoServer<B>    │
                          │                     │
                          │  dispatch()         │
                          │    → run::<Sha256>() │  ← monomorphized per algorithm
                          │    → run::<Hmac..>() │
                          │    → run::<Aes..>()  │
                          │                     │
                          │  Uses:              │
                          │  • crypto-api (wire)│
                          │  • crypto-traits    │
                          │  • kernel syscalls  │
                          └──────────┬──────────┘
                                     │
                          ┌──────────▼──────────┐
                          │  crypto-client      │
                          │  (unchanged API)    │
                          │                     │
                          │  sha256()           │
                          │  hmac_sha256()      │
                          │  aes_gcm_encrypt()  │
                          │  ecdsa_p256_sign()  │
                          └─────────────────────┘
```

### 9.10 Trait Layering: HAL vs. Service

The architecture contains two distinct trait layers that serve different purposes:

```
┌─────────────────────────────────────────────────────────────┐
│                     Crypto Client                           │
│  (user apps calling CryptoClient::sha256(), etc.)           │
└────────────────────────┬────────────────────────────────────┘
                         │ IPC (channel_call)
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                    Crypto Server                            │
│  dispatch_crypto_op() → OneShot<A> / Streaming<A>           │
└────────────────────────┬────────────────────────────────────┘
                         │ Service-layer traits
                         ▼
┌─────────────────────────────────────────────────────────────┐
│              services/crypto/api/backend.rs                 │  ◄── SERVICE LAYER
│  OneShot<A>, Streaming<A>, CryptoInput, BackendError        │
│  (IPC-oriented: writes to &mut [u8], session handles)       │
└────────────────────────┬────────────────────────────────────┘
                         │ impl OneShot<A> for ...
                         ▼
┌──────────────────────────────┬──────────────────────────────┐
│   RustCryptoBackend          │      HaceBackend             │
│   (software impl)            │      (hardware impl)         │
└──────────────────────────────┴───────────────┬──────────────┘
                                               │ uses HAL
                                               ▼
┌─────────────────────────────────────────────────────────────┐
│                hal/blocking/src/digest.rs                   │  ◄── HAL LAYER
│  owned::DigestInit, owned::DigestOp, scoped::*              │
│  (hardware-oriented: typestate, resource recovery)          │
└─────────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│               platform/impls/hace/                          │
│  HaceController impl owned::DigestInit<Sha2_256>            │
└─────────────────────────────────────────────────────────────┘
```

**HAL Layer** (`hal/blocking/`) — abstracts hardware vs. software crypto **implementations**. Used by platform impls (HACE, RustCrypto controllers) regardless of whether they run baremetal or inside a server.

**Service Layer** (`services/crypto/api/`) — abstracts **IPC protocol** between server and backends. Handles session management, writes to caller-provided buffers, and presents a uniform interface to the server dispatch logic.

This separation is intentional — HAL traits handle hardware resource management while service traits handle IPC concerns.

### 9.11 Decoupling Resource Recovery

The HAL's `owned::DigestOp` currently bundles two concerns:

**Concern 1: Operation semantics** — what the API does
```rust
fn update(&mut self, data: &[u8]) -> Result<(), Error>;
fn finalize(&mut self, output: &mut [u8]) -> Result<usize, Error>;
```

**Concern 2: Resource lifecycle** — typestate enforcement for baremetal safety
```rust
fn finalize(self) -> Result<(Output, Controller), Error>;  // consume + recover
fn cancel(self) -> Controller;                              // recover without completing
```

This coupling creates friction:

- Software backends (RustCrypto) have no resources to recover — the pattern is pure overhead
- Servers already manage sessions via explicit handles — the typestate ceremony is redundant
- Wrapping HAL controllers in service traits requires adapting between the two models

**Proposed: Decouple into composable parts**

```rust
// Core trait — used by HAL impls and servers directly
pub trait DigestOp {
    fn update(&mut self, data: &[u8]) -> Result<(), Error>;
    fn finalize(&mut self, output: &mut [u8]) -> Result<usize, Error>;
    fn reset(&mut self);  // reuse without destroying
}

// Optional typestate wrapper — for baremetal code that wants compile-time safety
pub struct Owned<T>(T);

impl<T: DigestOp> Owned<T> {
    pub fn update(mut self, data: &[u8]) -> Result<Self, Error> { ... }
    pub fn finalize(self) -> Result<(Output, T), Error> { ... }  // recovers inner
    pub fn cancel(self) -> T { ... }
}
```

**Benefits of decoupling:**

| Aspect | Current (coupled) | Proposed (decoupled) |
|--------|-------------------|----------------------|
| HAL controllers | Must impl owned semantics | Impl simple `&mut self` trait |
| Server backends | Wrap HAL with adapters | Use HAL controllers directly |
| Baremetal apps | ✅ Typestate enforcement | Opt-in via `Owned<T>` wrapper |
| Trait duplication | HAL + Service traits | Single core trait |
| Software backends | Fake resource recovery | No overhead |

The service-layer `Streaming<A>` trait already uses the simpler `&mut self` + session handle pattern. Aligning the HAL to this model would eliminate the adaptation layer between them.

---

## 10. Migration Plan

### Phase 1: Extract `crypto-traits` crate

**Effort:** ~150 lines, 1 day  
**Risk:** None (additive only, no existing code changes)  
**Deliverable:** New crate with `Algorithm`, `OneShot<A>`, `CryptoInput` types

### Phase 2: Implement `backend-rustcrypto`

**Effort:** ~250 lines, 1–2 days  
**Risk:** Low (wrapper impls over existing working code)  
**Deliverable:** `impl OneShot<*> for RustCryptoBackend` — all 14 algorithm impls  
**Validation:** Unit tests against known test vectors (RFC 4231, NIST vectors)

### Phase 3: Rewrite server with `CryptoServer<B>`

**Effort:** ~120 lines (down from 398), 1 day  
**Risk:** Medium (functional rewrite, same wire protocol)  
**Deliverable:** Generic server — existing clients work unchanged  
**Validation:** Existing integration tests pass without modification

### Phase 4: Add `Streaming` support

**Effort:** ~100 lines server + ~50 lines client, 2 days  
**Risk:** Medium (new wire protocol semantics using `flags` byte)  
**Deliverable:** Session-based hash API for firmware image verification  
**Validation:** New test: hash 8KB data in 1KB chunks, compare with one-shot

### Phase 5: Add ASPEED HACE backend

**Effort:** ~300 lines (new crate), 3–5 days  
**Risk:** High (hardware integration, DMA, register access)  
**Deliverable:** `impl OneShot<Sha256/384/512> for HaceBackend`  
**Validation:** Same test suite as RustCrypto — results must match

### Why No Mock Backend Is Needed

`RustCryptoBackend` is a pure software implementation with **zero hardware dependencies** — it compiles and runs identically on host, QEMU, and target. Unlike a mock that returns canned responses, RustCrypto provides real cryptographic validation against known test vectors (RFC 4231, NIST). Using it for host testing gives actual correctness assurance rather than tautological "mock returns what you told it to return" tests.

A mock would only be useful for:
- Forcing specific error paths (e.g., `HardwareFailure`) that RustCrypto never produces
- Deterministic timing tests
- Fuzzing the server's error handling

These are narrow scenarios that don't justify a dedicated backend. Error-path testing can be done with a thin wrapper that injects faults around the real backend.

### Impact Summary

| Metric | Current | After Phase 3 | After Phase 5 |
|--------|---------|--------------|--------------|
| Server lines | 398 | ~120 | ~120 (unchanged) |
| Add SHA3 | ~30 lines, 3 files | ~15 lines, 1 file | ~15 lines, 1 file |
| Add BLAKE3 | ~30 lines, 3 files | ~15 lines, 1 file | ~15 lines, 1 file |
| Backend swap | Server rewrite | Change type param | Change type param |
| Host tests | ❌ | ✅ (RustCrypto runs on host) | ✅ |
| Streaming | ❌ | ❌ (Phase 4) | ✅ |
| HW crypto | ❌ | ❌ | ✅ |

---

## 11. Appendix: Source Inventory

### Hubris Digest Server Sources

| File | Lines | Purpose |
|------|-------|---------|
| `hubris/drv/digest-server/src/main.rs` | 1,354 | Server with Idol IDL integration |
| `hubris/idl/openprot-digest.idol` | 360 | Interface definition (RON syntax) |
| `hubris/drv/openprot-digest-api/src/lib.rs` | 222 | Client API types and error enum |
| `hubris/task/hmac-client/src/main.rs` | 208 | Test client task |

### OpenPRoT HAL Sources

| File | Lines | Purpose |
|------|-------|---------|
| `bazel-stuff/hal/blocking/src/digest.rs` | ~900 | DigestOp, DigestInit traits (owned + scoped) |
| `bazel-stuff/hal/blocking/src/mac.rs` | ~680 | MacOp, MacInit traits (owned + scoped) |
| `bazel-stuff/platform/traits/hubris/src/lib.rs` | ~400 | HubrisDigestDevice, CryptoSession |
| `bazel-stuff/platform/impls/rustcrypto/src/controller.rs` | 1,033 | RustCryptoController + tests |

### Pigweed Crypto Service Sources

| File | Lines | Purpose |
|------|-------|---------|
| `services/crypto/api/src/protocol.rs` | 265 | Wire protocol, CryptoOp enum, headers |
| `services/crypto/server/src/main.rs` | 398 | Crypto server (flat dispatch) |
| `services/crypto/client/src/lib.rs` | 509 | Client library |
| `services/crypto/tests/src/main.rs` | ~200 | Integration tests (QEMU) |

### Key Design Decisions Summary

| Decision | Hubris Approach | Pigweed Approach | Proposed Approach |
|----------|----------------|------------------|-------------------|
| Algorithm dispatch | Runtime enum match × 6 | Runtime enum match × 1 | Compile-time monomorphization |
| Backend abstraction | God trait (`HubrisDigestDevice`) | None | Composable `OneShot<A>` traits |
| Output types | `Digest<N>` for hash, `[u8;N]` for HMAC | `[u8]` everywhere | `[u8]` everywhere (simplicity) |
| Session management | `CryptoSession` RAII wrapper | None | Optional `Streaming<A>` trait |
| IPC mechanism | Generated (Idol) | Manual wire protocol | Keep manual (simpler for Pigweed) |
| Error handling | `DigestError` enum (17 variants) | `CryptoError` enum (12 variants) | Keep `CryptoError` (sufficient) |
| Key management | `SecureOwnedKey` with Zeroize | Raw `&[u8]` slices | `&[u8]` for now, `SecureKey` later |
| Testing | RustCrypto (host-testable) | QEMU only | RustCrypto (host-testable) |

---

## 12. Specification Gap Analysis

Cross-reference of the OpenPRoT specification (`docs/src/specification/`) against
the currently implemented crypto operations in `CryptoOp` (14 ops).

### 12.1 Specification Sources

The following specification documents were reviewed:

- `specification/middleware/spdm.md` — SPDM algorithm requirements (primary crypto source)
- `specification/services/attestation.md` — Attestation architecture, RATS EAT, COSE, DICE
- `specification/services/fwupdate.md` — PLDM firmware update (no additional crypto beyond integrity)
- `specification/middleware/pldm.md` — PLDM monitoring/update (no additional crypto)
- `specification/middleware/mctp.md` — Transport layer (no crypto)
- `specification/firmware_resiliency.md` — NIST SP 800-193 (TBD sections, no specific algorithms yet)

### 12.2 Currently Implemented Operations

| Category | Operations | Spec Status |
|----------|-----------|-------------|
| Hash | SHA-256, SHA-384, SHA-512 | Listed in SPDM hash algorithms |
| MAC | HMAC-SHA-256, HMAC-SHA-384, HMAC-SHA-512 | Needed for KDF / SPDM session key derivation |
| AEAD | AES-256-GCM encrypt/decrypt | Listed in SPDM AEAD ciphers |
| Stream Cipher | AES-256-CTR encrypt/decrypt | Not in SPDM spec; internal use only |
| Signature | ECDSA P-256 sign/verify, ECDSA P-384 sign/verify | Listed in SPDM asymmetric algorithms |

### 12.3 Missing — Mandatory

The SPDM spec states that hardware **must** support at minimum:

- `TPM_ALG_ECDSA_ECC_NIST_P384` (already implemented)
- `TPM_ALG_SHA3_384` (**not implemented**)

| Algorithm | Op Codes Needed | Priority |
|-----------|----------------|----------|
| **SHA3-384** | `Sha3_384Hash` | **REQUIRED** — mandatory minimum per SPDM spec |

### 12.4 Missing — Listed in SPDM Spec (Optional)

These algorithms are explicitly listed in the SPDM algorithms section and may
be used if supported by hardware.

#### 12.4.1 Hash Algorithms

| Algorithm | Op Code | Notes |
|-----------|---------|-------|
| SHA3-256 | `Sha3_256Hash` | Listed under SPDM hash algorithms |
| SHA3-512 | `Sha3_512Hash` | Listed under SPDM hash algorithms |

#### 12.4.2 Asymmetric Signature Algorithms

| Algorithm | Op Codes | Notes |
|-----------|----------|-------|
| EdDSA Ed25519 | `Ed25519Sign`, `Ed25519Verify` | Listed under SPDM asymmetric |
| EdDSA Ed448 | `Ed448Sign`, `Ed448Verify` | Listed under SPDM asymmetric |

#### 12.4.3 AEAD Ciphers

| Algorithm | Op Codes | Notes |
|-----------|----------|-------|
| AES-128-GCM | `Aes128GcmEncrypt`, `Aes128GcmDecrypt` | Listed under SPDM AEAD |
| ChaCha20-Poly1305 | `Chacha20Poly1305Encrypt`, `Chacha20Poly1305Decrypt` | Listed under SPDM AEAD |

### 12.5 Missing — Implied by Protocol Requirements

These are not directly listed in the SPDM algorithms table but are required or
implied by SPDM session establishment and attestation workflows.

| Algorithm | Category | Rationale |
|-----------|----------|-----------|
| ECDH P-256 | Key Exchange | SPDM `KEY_EXCHANGE` command; derives shared secret |
| ECDH P-384 | Key Exchange | SPDM `KEY_EXCHANGE` command; mandatory P-384 curve |
| X25519 | Key Exchange | SPDM key exchange with Ed25519 suite |
| HKDF-SHA-256 | KDF | SPDM session key derivation (NIST SP 800-108 ref) |
| HKDF-SHA-384 | KDF | SPDM session key derivation with SHA-384 suite |

### 12.6 Summary

| Status | Count | Algorithms |
|--------|-------|------------|
| Implemented | 14 ops | SHA-256/384/512, HMAC-SHA-256/384/512, AES-256-GCM enc/dec, AES-256-CTR enc/dec, ECDSA P-256/P-384 sign/verify |
| **Mandatory gap** | 1 | SHA3-384 |
| Spec-listed optional | 9 ops | SHA3-256, SHA3-512, Ed25519 sign/verify, Ed448 sign/verify, AES-128-GCM enc/dec, ChaCha20-Poly1305 enc/dec |
| Implied by sessions | 5+ ops | ECDH P-256/P-384, X25519, HKDF-SHA-256/384 |
| **Total new ops** | ~15-20 | Depending on how key exchange and KDF are modeled |

**Notes:**

1. AES-256-CTR (currently implemented) is not in the SPDM spec. It may be
   useful for internal firmware encryption but is not required for protocol
   compliance.

2. Key exchange (ECDH) and KDF (HKDF) may warrant their own op-code ranges
   rather than being crammed into existing categories. Suggested ranges:
   - Key exchange: `0x50-0x5F`
   - KDF: `0x60-0x6F`
   - SHA3: `0x04-0x06` (extend digest range)

3. Ed25519/Ed448 signature ops could use `0x44-0x47` (extend ECDSA range into
   a general "signatures" range).

4. The HAL layer will also need corresponding traits for any new algorithms
   that require hardware acceleration.

---

*End of design review.*
