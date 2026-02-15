# Hubris Digest Server Implementation Analysis

This document reverse engineers the Hubris digest server implementation to understand its architecture, IPC protocol, and design patterns.

## 1. Repository Structure

| File | Purpose |
|------|---------|
| `hubris/idl/openprot-digest.idol` | IDL interface definition (360 lines) |
| `hubris/drv/digest-server/src/main.rs` | Server implementation (~1354 lines) |
| `hubris/drv/openprot-digest-api/src/lib.rs` | Client API and types (222 lines) |
| `hubris/task/hmac-client/src/main.rs` | Example test client task (208 lines) |
| `hubris/app/ast1060-digest-test/app.toml` | Task configuration example |

## 2. Interface Definition (Idol IDL)

Hubris uses **Idol** as its Interface Definition Language. The digest server interface is defined in `openprot-digest.idol`.

### 2.1 Session-Based Digest Operations

```idol
"init_sha256": (
    args: {},
    reply: Result(
        ok: "u32",  // Returns session ID
        err: CLike("DigestError"),
    ),
),

"update": (
    args: {
        "session_id": "u32",
        "len": "u32",
    },
    leases: {
        "data": (type: "[u8]", read: true, max_len: Some(1024)),
    },
    reply: Result(
        ok: "()",
        err: CLike("DigestError"),
    ),
),

"finalize_sha256": (
    args: {
        "session_id": "u32",
    },
    leases: {
        "digest_out": (type: "[u32; 8]", write: true),
    },
    reply: Result(
        ok: "()",
        err: CLike("DigestError"),
    ),
),
```

### 2.2 One-Shot Operations

```idol
"digest_oneshot_sha256": (
    args: {
        "len": "u32",
    },
    leases: {
        "data": (type: "[u8]", read: true, max_len: Some(1024)),
        "digest_out": (type: "[u32; 8]", write: true),
    },
    reply: Result(
        ok: "()",
        err: CLike("DigestError"),
    ),
),
```

### 2.3 HMAC Operations

```idol
"init_hmac_sha256": (
    args: {
        "key_len": "u32",
    },
    leases: {
        "key": (type: "[u8]", read: true, max_len: Some(64)),
    },
    reply: Result(
        ok: "u32",  // session ID
        err: CLike("DigestError"),
    ),
),

"finalize_hmac_sha256": (
    args: {
        "session_id": "u32",
    },
    leases: {
        "mac_out": (type: "[u32; 8]", write: true),
    },
    reply: Result(
        ok: "()",
        err: CLike("DigestError"),
    ),
),
```

### 2.4 Supported Algorithms

| Algorithm | Init | Update | Finalize | One-Shot |
|-----------|------|--------|----------|----------|
| SHA-256 | ✅ | ✅ | ✅ | ✅ |
| SHA-384 | ✅ | ✅ | ✅ | ✅ |
| SHA-512 | ✅ | ✅ | ✅ | ✅ |
| SHA3-256/384/512 | ❌ (planned) | - | - | - |
| HMAC-SHA256 | ✅ | ✅ | ✅ | ✅ |
| HMAC-SHA384 | ✅ | ✅ | ✅ | ✅ |
| HMAC-SHA512 | ✅ | ✅ | ✅ | ✅ |

## 3. Error Enumeration

```rust
#[repr(u32)]
pub enum DigestError {
    InvalidInputLength = 1,
    UnsupportedAlgorithm = 2,
    MemoryAllocationFailure = 3,
    InitializationError = 4,
    UpdateError = 5,
    FinalizationError = 6,
    Busy = 7,
    HardwareFailure = 8,
    InvalidOutputSize = 9,
    PermissionDenied = 10,
    NotInitialized = 11,
    InvalidSession = 12,
    TooManySessions = 13,
    InvalidKeyLength = 14,
    HmacVerificationFailed = 15,
    KeyRequired = 16,
    IncompatibleSessionType = 17,
    #[idol(server_death)]
    ServerRestarted = 100,
}
```

## 4. Server Architecture

### 4.1 High-Level Design

```
┌─────────────────┐    IPC     ┌─────────────────┐   OpenPRoT    ┌─────────────────┐
│   Client Task   │ ────────── │  Digest Server  │   HAL Traits  │   Backend       │
│                 │  (Idol)    │   (ServerImpl)  │ ──────────── │ • RustCrypto    │
│                 │            │                 │              │ • ASPEED HACE   │
│  task_slot!     │            │ • SessionStore  │              │ • Mock          │
│  Digest::from() │            │ • CryptoSession │              └─────────────────┘
└─────────────────┘            └─────────────────┘
```

### 4.2 Core Data Structures

```rust
/// Main server with generic backend support
pub struct ServerImpl<D: HubrisDigestDevice> {
    controllers: Controllers<D>,
    current_session: Option<DigestSession<D>>,
    next_session_id: u32,
}

/// Hardware controller pool
struct Controllers<D> {
    hardware: Option<D>,  // Single controller, None when in use
}

/// Active session tracking
struct DigestSession<D: HubrisDigestDevice> {
    session_id: u32,
    algorithm: DigestAlgorithm,
    context: SessionContext<D>,
    created_at: u64,
}

/// Algorithm-specific context variants
enum SessionContext<D: HubrisDigestDevice> {
    Sha256(Option<CryptoSession<D::DigestContext256, D>>),
    Sha384(Option<CryptoSession<D::DigestContext384, D>>),
    Sha512(Option<CryptoSession<D::DigestContext512, D>>),
    HmacSha256(Option<CryptoSession<D::HmacContext256, D>>),
    HmacSha384(Option<CryptoSession<D::HmacContext384, D>>),
    HmacSha512(Option<CryptoSession<D::HmacContext512, D>>),
}
```

### 4.3 RAII Device Recovery Pattern

```rust
/// RAII wrapper ensuring hardware is always returned
pub struct CryptoSession<Context, Device> {
    context: Option<Context>,
    device: Option<Device>,
}

impl<Context, Device> CryptoSession<Context, Device> {
    /// Consumes the session, returning the device for reuse
    pub fn finish(mut self) -> Device {
        self.device.take().expect("Device already taken")
    }
}

impl<Context, Device> Drop for CryptoSession<Context, Device> {
    fn drop(&mut self) {
        // Device is returned even if session is dropped without finish()
    }
}
```

### 4.4 Backend Selection (Cargo Features)

```toml
[features]
default = ["mock"]
mock = ["openprot-platform-mock"]
aspeed-hace = ["aspeed-ddk", "ast1060-pac"]
rustcrypto = ["openprot-platform-rustcrypto"]
```

## 5. Hubris IPC Mechanism

### 5.1 Task Slots

Compile-time inter-task dependency declaration:

```rust
// Client declares dependency on digest server
task_slot!(DIGEST, digest_server);

fn main() -> ! {
    let digest_task = DIGEST.get_task_id();
    let digest = Digest::from(digest_task);
    // ...
}
```

### 5.2 Leases (Zero-Copy Memory)

Idol supports **leases** for efficient large data transfer:

```idol
leases: {
    "data": (type: "[u8]", read: true, max_len: Some(1024)),
    "digest_out": (type: "[u32; 8]", write: true),
},
```

- `read: true` - Server reads from client memory
- `write: true` - Server writes to client memory
- `max_len` - Runtime validation of buffer size

### 5.3 IDL Code Generation

```rust
// Server (build.rs)
idol::Generator::new().build_server_support(
    "../../idl/openprot-digest.idol",
    "server_stub.rs",
    idol::server::ServerStyle::InOrder,
)?;

// Client (build.rs)
idol::client::build_client_stub(
    "../../idl/openprot-digest.idol",
    "client_stub.rs",
)?;
```

## 6. Client API Usage

### 6.1 Session-Based Digest

```rust
use drv_digest_api::Digest;

task_slot!(DIGEST, digest_server);

fn hash_large_data(data: &[u8]) -> [u32; 8] {
    let digest = Digest::from(DIGEST.get_task_id());
    
    // Initialize session
    let session_id = digest.init_sha256().unwrap();
    
    // Stream data in chunks
    for chunk in data.chunks(1024) {
        digest.update(session_id, chunk.len() as u32, chunk).unwrap();
    }
    
    // Finalize and get result
    let mut result = [0u32; 8];
    digest.finalize_sha256(session_id, &mut result).unwrap();
    result
}
```

### 6.2 One-Shot Digest

```rust
fn hash_small_data(data: &[u8]) -> [u32; 8] {
    let digest = Digest::from(DIGEST.get_task_id());
    
    let mut result = [0u32; 8];
    digest.digest_oneshot_sha256(
        data.len() as u32,
        data,
        &mut result
    ).unwrap();
    result
}
```

### 6.3 HMAC Operations

```rust
fn compute_hmac(key: &[u8], data: &[u8]) -> [u32; 8] {
    let digest = Digest::from(DIGEST.get_task_id());
    
    // Session-based HMAC
    let session_id = digest.init_hmac_sha256(key.len() as u32, key).unwrap();
    digest.update(session_id, data.len() as u32, data).unwrap();
    
    let mut mac = [0u32; 8];
    digest.finalize_hmac_sha256(session_id, &mut mac).unwrap();
    mac
}
```

## 7. Task Configuration (app.toml)

```toml
# Digest Server Task
[tasks.digest_server]
name = "digest-server"
priority = 2
max-sizes = {flash = 32768, ram = 16384}
start = true
stacksize = 8192
features = ["rustcrypto"]

# Client Task
[tasks.my_client]
name = "my-client-task"
priority = 3
max-sizes = {flash = 32768, ram = 8192}
start = true
stacksize = 4096
task-slots = ["digest_server"]  # Declares IPC dependency
```

## 8. Key Design Decisions

### 8.1 Session-Based vs One-Shot

| Aspect | Session-Based | One-Shot |
|--------|---------------|----------|
| Use Case | Large/streaming data | Small data (<1KB) |
| IPC Calls | 3+ (init, update×N, finalize) | 1 |
| Memory | Streaming, low memory | Must fit in single buffer |
| State | Server maintains context | Stateless |

### 8.2 HMAC Key Size Limits

```
SHA-256 block size: 64 bytes  → HMAC-SHA256 key limit: 64 bytes
SHA-384 block size: 128 bytes → HMAC-SHA384 key limit: 128 bytes
SHA-512 block size: 128 bytes → HMAC-SHA512 key limit: 128 bytes
```

**Rationale:**
- Keys ≤ block size are processed directly without additional hashing
- Keys > block size provide no additional security benefit
- Aligns with hardware accelerator constraints
- Prevents DoS from oversized key processing

### 8.3 Output Format

Digests return `[u32; N]` arrays (native word arrays) rather than `[u8; N*4]`:
- SHA-256: `[u32; 8]` (8 words × 4 bytes = 32 bytes)
- SHA-384: `[u32; 12]` (12 words × 4 bytes = 48 bytes)
- SHA-512: `[u32; 16]` (16 words × 4 bytes = 64 bytes)

This avoids byte-order conversion on little-endian embedded platforms.

## 9. OpenPRoT HAL Integration

The server uses platform-agnostic traits from `openprot-hal-blocking`:

```rust
use openprot_hal_blocking::digest::owned::{DigestInit, DigestOp};
use openprot_hal_blocking::digest::{Digest, Sha2_256, Sha2_384, Sha2_512};
use openprot_hal_blocking::mac::owned::{MacInit, MacOp};
use openprot_hal_blocking::mac::{HmacSha2_256, HmacSha2_384, HmacSha2_512};
```

Backend implementations:
- `HaceController` - ASPEED HACE hardware accelerator
- `RustCryptoController` - Software RustCrypto implementation
- `MockDigestController` - Testing mock

## 10. Comparison: Hubris vs Pigweed IPC

| Aspect | Hubris (Idol) | Pigweed Kernel |
|--------|---------------|----------------|
| IDL | `.idol` files (RON syntax) | None (manual protocol) |
| Code Gen | Build-time stub generation | Manual encoding/decoding |
| Memory | Leases (zero-copy) | `channel_transact` (copy) |
| Task Discovery | `task_slot!` macro | Handle table in system.json5 |
| Error Handling | `Result<T, Error>` in IDL | Manual header parsing |
| Type Safety | Strong (generated) | Manual |
| Blocking | `sys_recv` with notifications | `object_wait` |

## 11. Lessons for Pigweed Crypto Service

Based on the Hubris design, key patterns to adopt:

1. **Session-based API** for streaming large data
2. **One-shot API** for small data efficiency
3. **Structured error enum** with meaningful codes
4. **Backend abstraction** via traits/features
5. **RAII device recovery** for resource management
6. **Word-aligned output** for embedded efficiency
7. **Block-size key limits** for HMAC operations

---

# Part 2: Critical Design Improvements

_Analysis by a Rust expert and senior Pigweed engineer._

## 12. Problems in the Hubris Server Architecture

### 12.1 Massive Code Duplication (The Cardinal Sin)

The Hubris digest server is **1,354 lines** of mostly copy-pasted code. Every algorithm variant repeats the same structural pattern:

```
init_sha256_internal  ≈ init_sha384_internal  ≈ init_sha512_internal
init_hmac_sha256      ≈ init_hmac_sha384      ≈ init_hmac_sha512
finalize_sha256       ≈ finalize_sha384       ≈ finalize_sha512
finalize_hmac_sha256  ≈ finalize_hmac_sha384  ≈ finalize_hmac_sha512
oneshot_sha256        ≈ oneshot_sha384        ≈ oneshot_sha512
hmac_oneshot_sha256   ≈ hmac_oneshot_sha384   ≈ hmac_oneshot_sha512
```

That's **6 init methods**, **6 finalize methods**, **6 one-shot methods** that differ only in their associated types. Adding a new algorithm (SHA3, BLAKE3) would add **another 6+ methods** — a combinatorial explosion.

**Root cause:** The `SessionContext` enum forces per-variant dispatch at runtime, negating the compile-time type safety that the generic `DigestOp`/`MacOp` traits were designed to provide. The HAL traits are beautifully generic; the server throws that away immediately.

### 12.2 The `Option<Option<Context>>` Anti-Pattern

```rust
enum SessionContext<D: HubrisDigestDevice> {
    Sha256(Option<CryptoSession<D::DigestContext256, D>>),
    //     ^^^^^^ -- Why is this an Option when contained inside Option<DigestSession>?
}
```

The context is wrapped in `Option` inside `SessionContext`, inside `Option<DigestSession>`, giving two layers of optionality. This stems from the `CryptoSession` itself also using `Option` internally for its take/put dance:

```rust
pub struct CryptoSession<Context, Device> {
    context: Option<Context>,   // ← third layer of Option!
    device: Option<Device>,     // ← fourth layer of Option!
}
```

That's **four levels of `Option`** to track whether one hardware context is busy. This is not how you write zero-cost abstractions in Rust.

### 12.3 The `HubrisDigestDevice` Supertrait is a God Object

```rust
pub trait HubrisDigestDevice {
    type DigestContext256: DigestOp<...>;
    type DigestContext384: DigestOp<...>;
    type DigestContext512: DigestOp<...>;
    type HmacContext256: MacOp<...>;
    type HmacContext384: MacOp<...>;
    type HmacContext512: MacOp<...>;
    type HmacKey: ...;
    // 12 methods: 6 direct init + 6 session init
}
```

Every new algorithm demands **two new associated types** and **two new methods** in this trait, plus corresponding changes in every implementor. This violates the Open-Closed Principle. The trait should be composed from orthogonal capabilities, not be a flat collection of everything.

### 12.4 HMAC Output Type Mismatch

Digests return `Digest<N>` (word arrays), but HMACs return `[u8; N]` (byte arrays). The server then manually converts `[u8; 32]` → `[u32; 8]` in every HMAC finalize method:

```rust
let mut u32_result = [0u32; 8];
for (i, chunk) in result.chunks(4).enumerate() {
    u32_result[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
}
```

This conversion is repeated verbatim 6 times. It should be a single generic function `fn bytes_to_words<const N: usize>(src: &[u8]) -> [u32; N]`, or better yet, the HMAC output type should be unified with the digest output type.

### 12.5 No Session Timeout / Cleanup

`created_at: u64` is stored but never checked. A crashed client that holds a session will permanently lock the single hardware controller. There is no timeout mechanism, no `Drop` for the session that returns the controller, and no administrative reset command that works.

### 12.6 Single-Session Bottleneck

`MAX_SESSIONS: usize = 1` and `current_session: Option<DigestSession<D>>` — the entire server can only process one request pipeline at a time. While partially inherent to single-controller hardware, the architecture makes it impossible to support even software-backed concurrent sessions (e.g., queue multiple RustCrypto sessions).

---

## 13. Improved Generic Architecture

### 13.1 Unified Operation Trait Hierarchy

Replace the monolithic `HubrisDigestDevice` with composable traits:

```rust
/// Core trait: any crypto backend that can perform an operation.
/// One impl per (Backend, Algorithm) pair — no God object.
pub trait CryptoBackend<A: Algorithm>: Sized {
    type Context: CryptoContext<Output = A::Output, Backend = Self>;
    type Error;
    
    fn begin(self, params: A::Params) -> Result<Self::Context, Self::Error>;
}

/// Algorithm marker types with associated output dimensions.
pub trait Algorithm {
    type Output: AsRef<[u8]>;
    type Params;
    const OP_CODE: u8;
}

/// Stateful context (move-semantic, same as OpenPRoT DigestOp).
pub trait CryptoContext: Sized {
    type Output;
    type Backend;
    type Error;
    
    fn feed(self, data: &[u8]) -> Result<Self, Self::Error>;
    fn finish(self) -> Result<(Self::Output, Self::Backend), Self::Error>;
}
```

Algorithm markers replace the enum explosion:

```rust
struct Sha256;
impl Algorithm for Sha256 {
    type Output = [u8; 32];
    type Params = ();
    const OP_CODE: u8 = 0x01;
}

struct HmacSha256;
impl Algorithm for HmacSha256 {
    type Output = [u8; 32];
    type Params = HmacKey;
    const OP_CODE: u8 = 0x10;
}

struct Aes256GcmEncrypt;
impl Algorithm for Aes256GcmEncrypt {
    type Output = (); // written in-place
    type Params = AeadParams;
    const OP_CODE: u8 = 0x20;
}

struct EcdsaP256Sign;
impl Algorithm for EcdsaP256Sign {
    type Output = [u8; 64];
    type Params = SigningParams;
    const OP_CODE: u8 = 0x40;
}
```

Adding BLAKE3 or SHA3 becomes:

```rust
struct Blake3;
impl Algorithm for Blake3 {
    type Output = [u8; 32];
    type Params = ();
    const OP_CODE: u8 = 0x04;
}
impl CryptoBackend<Blake3> for RustCryptoBackend { ... }
// That's it. No God trait changes.
```

### 13.2 Generic Operation Dispatch

A single dispatcher replaces all the copy-pasted methods:

```rust
/// Dispatches a one-shot crypto operation generically.
fn dispatch_oneshot<A, B>(
    backend: &mut Option<B>,
    params: A::Params,
    data: &[u8],
    response: &mut [u8],
) -> usize
where
    A: Algorithm,
    B: CryptoBackend<A>,
    A::Output: AsRef<[u8]>,
{
    let Some(b) = backend.take() else {
        return encode_error(response, CryptoError::Busy);
    };
    
    let ctx = match b.begin(params) {
        Ok(c) => c,
        Err(_) => return encode_error(response, CryptoError::InternalError),
    };
    
    let ctx = match ctx.feed(data) {
        Ok(c) => c,
        Err(_) => return encode_error(response, CryptoError::InternalError),
    };
    
    match ctx.finish() {
        Ok((output, b)) => {
            *backend = Some(b);
            encode_success(response, output.as_ref())
        }
        Err(_) => encode_error(response, CryptoError::InternalError),
    }
}
```

This **single function** replaces 6 separate `compute_*_oneshot` methods in Hubris and 10+ `do_*` functions in our Pigweed server. Adding a new algorithm requires zero changes to dispatch logic.

### 13.3 Wire Protocol Abstraction

Separate wire format parsing from crypto logic:

```rust
/// Parsed request with borrowed fields — no copies until needed.
struct ParsedRequest<'a> {
    op: CryptoOp,
    key: &'a [u8],
    nonce: &'a [u8],
    data: &'a [u8],
}

impl<'a> ParsedRequest<'a> {
    fn parse(buf: &'a [u8]) -> Result<Self, CryptoError> { ... }
}
```

This cleanly separates concerns that are currently tangled in `dispatch_crypto_op`.

---

## 14. Applying the Improved Design to the Pigweed Crypto Server

### 14.1 Current Pigweed Service Problems

Our Pigweed crypto service has different but related problems:

| Problem | Details |
|---------|---------|
| **Hardcoded backend** | Directly imports `sha2`, `hmac`, `aes_gcm`, `p256` — no trait abstraction |
| **Flat function architecture** | 14 separate `do_*` functions with repeated error handling patterns |
| **No streaming support** | Only one-shot operations; can't hash data >1KB |
| **Overloaded nonce field** | ECDSA verify stuffs the signature into the "nonce" header field — semantically wrong |
| **Untyped wire protocol** | The `CryptoRequestHeader` uses the same 3 fields (key, nonce, data) for fundamentally different operations |
| **No backend swappability** | Can't switch to hardware crypto without rewriting the server |

### 14.2 Proposed Pigweed Crypto Service Redesign

#### Step 1: Algorithm Marker Types (New Crate: `crypto-traits`)

```rust
// crypto-traits/src/lib.rs — no_std, no dependencies

pub trait Algorithm {
    type Output: AsRef<[u8]>;
    const OP_CODE: u8;
    const OUTPUT_SIZE: usize;
}

pub trait OneShot<A: Algorithm> {
    type Error;
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, Self::Error>;
}

/// Structured input — not flat key||nonce||data.
pub enum CryptoInput<'a> {
    /// Hash: just data
    Data(&'a [u8]),
    /// MAC: key + data
    Keyed { key: &'a [u8], data: &'a [u8] },
    /// AEAD: key + nonce + plaintext/ciphertext
    Aead { key: &'a [u8], nonce: &'a [u8], data: &'a [u8] },
    /// Signature: private_key + message
    Sign { key: &'a [u8], message: &'a [u8] },
    /// Verification: public_key + message + signature
    Verify { key: &'a [u8], message: &'a [u8], signature: &'a [u8] },
}
```

#### Step 2: RustCrypto Backend (Keep in `platform/impls/rustcrypto`)

```rust
pub struct RustCryptoBackend;

impl OneShot<Sha256> for RustCryptoBackend {
    type Error = CryptoError;
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Data(data) = input else {
            return Err(CryptoError::InvalidOperation);
        };
        let hash = sha2::Sha256::digest(data);
        output[..32].copy_from_slice(&hash);
        Ok(32)
    }
}

impl OneShot<EcdsaP256Sign> for RustCryptoBackend {
    type Error = CryptoError;
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Sign { key, message } = input else {
            return Err(CryptoError::InvalidOperation);
        };
        // ... p256 signing logic
    }
}
// Each impl is ~15 lines. Adding BLAKE3 is one new impl block.
```

#### Step 3: Generic Server (Replace Current `main.rs`)

```rust
pub struct CryptoServer<B> {
    backend: B,
    request_buf: [u8; 1024],
    response_buf: [u8; 1024],
}

impl<B> CryptoServer<B>
where
    B: OneShot<Sha256>
     + OneShot<Sha384>
     + OneShot<Sha512>
     + OneShot<HmacSha256>
     + OneShot<Aes256GcmEncrypt>
     + OneShot<EcdsaP256Sign>
     // ... add bounds for supported algorithms
{
    fn dispatch(&self, request: &ParsedRequest, response: &mut [u8]) -> usize {
        match request.op {
            CryptoOp::Sha256Hash => self.run::<Sha256>(request, response),
            CryptoOp::HmacSha256 => self.run::<HmacSha256>(request, response),
            CryptoOp::EcdsaP256Sign => self.run::<EcdsaP256Sign>(request, response),
            // ...
        }
    }
    
    fn run<A: Algorithm>(&self, request: &ParsedRequest, response: &mut [u8]) -> usize
    where
        B: OneShot<A>,
    {
        let input = request.to_crypto_input::<A>();
        let result_start = CryptoResponseHeader::SIZE;
        match self.backend.compute(&input, &mut response[result_start..]) {
            Ok(len) => encode_success(response, len),
            Err(e) => encode_error(response, e),
        }
    }
}
```

The server body shrinks from **398 lines** to approximately **80 lines** of non-boilerplate logic.

#### Step 4: Session Support for Streaming

For operations that need streaming (hash large firmware images), add a session layer:

```rust
pub trait Streaming<A: Algorithm>: Sized {
    type Session;
    type Error;
    
    fn begin(&mut self) -> Result<Self::Session, Self::Error>;
    fn feed(&mut self, session: &mut Self::Session, data: &[u8]) -> Result<(), Self::Error>;
    fn finish(&mut self, session: Self::Session, output: &mut [u8]) -> Result<usize, Self::Error>;
}
```

The wire protocol already has the `flags` field in `CryptoRequestHeader` — use bit 0 to indicate session operations:

```
flags[0] = 0: one-shot
flags[0] = 1: session operation (init/update/finalize based on flags[1:2])
```

### 14.3 Impact Assessment

| Metric | Current | Improved |
|--------|---------|----------|
| Server `main.rs` lines | 398 | ~120 |
| Adding new algorithm | ~30 lines across 3 files | ~15 lines in 1 file |
| Backend swappable? | No | Yes (generic `B`) |
| Session/streaming? | No | Yes (trait-based) |
| Type safety at dispatch? | Runtime match | Compile-time `OneShot<A>` bound |
| Wire protocol semantics | Overloaded fields | Structured `CryptoInput` enum |
| Testability | Requires QEMU | Mock backend, host-testable |

### 14.4 Migration Path

1. **Phase 1**: Extract `crypto-traits` crate with algorithm markers and `OneShot` trait — zero breakage.
2. **Phase 2**: Implement `OneShot<*> for RustCryptoBackend` — wrapper impls over existing code.
3. **Phase 3**: Rewrite server `dispatch` to use `CryptoServer<B>` — same wire format, same client.
4. **Phase 4**: Add `Streaming` trait and session support.
5. **Phase 5**: Add ASPEED HACE backend implementing same traits — hardware crypto with no server changes.

---

## 15. Summary

The Hubris digest server demonstrates correct *principles* — session ownership, hardware abstraction, RAII recovery — but the *implementation* suffers from massive duplication caused by the monolithic `HubrisDigestDevice` supertrait and runtime-dispatched `SessionContext` enum. The Pigweed crypto service avoids the duplication problem by having no abstraction at all, which trades extensibility for simplicity.

The right answer is **neither**: use Rust's trait system to make the dispatch generic over algorithms, so adding a new operation is a single impl block rather than changes to 6+ functions across 3+ files. The key insight is that `Algorithm` should be a *type parameter*, not an *enum variant* — let the compiler monomorphize the dispatch instead of writing it by hand.
