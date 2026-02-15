# Crypto Service Refactor — Implementation Plan

**Date:** February 14, 2026  
**Status:** Approved  
**Reference:** [crypto-service-design-review.md](crypto-service-design-review.md)

---

## Overview

This plan implements the proposed architecture from the design review:
- Algorithm marker types with compile-time dispatch
- `OneShot<A>` trait for backend abstraction
- Generic `CryptoServer<B>` that works with any backend
- Streaming sessions via `Streaming<A>` trait

---

## Design Decision: Why Not Reuse `RustCryptoController`?

The existing `platform/impls/rustcrypto/src/controller.rs` (640 lines) implements
the `openprot-hal-blocking` traits. We considered reusing it but chose **not to**
for long-term maintainability:

### Current Architecture (God Trait)

```rust
// hal/blocking/src/digest.rs
pub trait DigestInit<A> { 
    type Context: DigestOp; 
    fn init(self, algo: A) -> Result<Self::Context, Self::Error>;
}

// platform/impls/rustcrypto
impl DigestInit<Sha2_256> for RustCryptoController { ... }
impl DigestInit<Sha2_384> for RustCryptoController { ... }
impl DigestInit<Sha2_512> for RustCryptoController { ... }
impl DigestOp for DigestContext256 { ... }
impl DigestOp for DigestContext384 { ... }
impl DigestOp for DigestContext512 { ... }
// 6 impls just for digest, 6 more for MAC = 12+ total
```

**Problem:** Adding SHA3-384 requires:
1. Modify `DigestInit` trait (or add new marker type)
2. Add `DigestContext3_384` in RustCryptoController
3. Add impl for HACE backend
4. Add impl for any test mock

### Proposed Architecture (Composable Traits)

```rust
// crypto-traits/src/lib.rs
pub trait OneShot<A: Algorithm> {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError>;
}

// backend-rustcrypto/src/lib.rs
impl OneShot<Sha256> for RustCryptoBackend { ... }
impl OneShot<Sha384> for RustCryptoBackend { ... }
impl OneShot<Sha3_384> for RustCryptoBackend { ... }  // ← Adding algorithm = adding impl

// backend-hace/src/lib.rs (HACE only supports SHA-2)
impl OneShot<Sha256> for HaceBackend { ... }
impl OneShot<Sha384> for HaceBackend { ... }
// No Sha3_384 impl needed if HACE doesn't support it
```

**Benefit:** Adding SHA3-384 requires:
1. Add `pub struct Sha3_384; impl Algorithm for Sha3_384 { ... }` to traits
2. Add `impl OneShot<Sha3_384> for RustCryptoBackend { ... }` to backend
3. Done — no other files touched

### Impact on Algorithm Roadmap

The SPDM spec requires 10-15 additional algorithms. Each algorithm addition:

| Approach | Files Changed | Risk |
|----------|--------------|------|
| Current God trait | 3-4 per algorithm | High (touches shared traits) |
| Composable traits | 1-2 per algorithm | Low (additive only) |

**Decision:** Take the upfront refactor cost now for clean extensibility later.
The existing `RustCryptoController` code will be referenced for implementation
details but not directly reused.

---

## Architectural Insight: Trait Layering (HAL vs Service)

During implementation review, we identified an important layering distinction:

```
┌─────────────────────────────────────────────────────────────────┐
│                        Crypto Client                            │
│  (services/crypto/client)                                       │
└───────────────────────────┬─────────────────────────────────────┘
                            │ IPC (channel_call)
┌───────────────────────────▼─────────────────────────────────────┐
│                        Crypto Server                            │
│  (services/crypto/server)                                       │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │              Service Layer Traits                         │  │
│  │  OneShot<A>    — one-shot operations via &self            │  │
│  │  Streaming<A>  — session-based via session handle         │  │
│  │  (services/crypto/api/src/backend.rs)                     │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │                                    │
│                            │ impl                               │
│                            ▼                                    │
│  ┌──────────────────┐  ┌──────────────────┐                     │
│  │ RustCryptoBackend│  │   HaceBackend    │ ... (future)        │
│  │ (software)       │  │ (hardware accel) │                     │
│  └────────┬─────────┘  └────────┬─────────┘                     │
│           │                     │                               │
└───────────┼─────────────────────┼───────────────────────────────┘
            │                     │
            │ (may use)           │ (uses)
            ▼                     ▼
┌───────────────────────────────────────────────────────────────────┐
│                        HAL Layer Traits                           │
│  owned::DigestInit  — typestate pattern (moves self)              │
│  owned::DigestOp    — resource recovery via fn cancel(self)       │
│  (hal/blocking/src/digest.rs)                                     │
└───────────────────────────────────────────────────────────────────┘
            │
            │ impl
            ▼
┌───────────────────────────────────────────────────────────────────┐
│  RustCryptoController │ HaceController │ ... (platform impls)     │
│  (platform/impls/)    │                │                          │
└───────────────────────────────────────────────────────────────────┘
```

### Key Distinction

| Layer | Location | Purpose | Pattern |
|-------|----------|---------|----------|
| **Service** | `services/crypto/api/src/backend.rs` | Abstract IPC protocol | `&self` + session handle |
| **HAL** | `hal/blocking/src/digest.rs` | Abstract hardware controllers | Typestate (moves self) |

### Why Two Layers?

1. **Service Layer:** Designed for IPC semantics where state lives across multiple
   requests. The server owns a `StreamingSession` and clients reference it via session ID.

2. **HAL Layer:** Designed for baremetal use where resource recovery is critical.
   The typestate pattern ensures hardware controllers are properly released on
   success, failure, or cancellation.

### Relationship

- The HAL layer is used **by** platform implementations (both baremetal and server backends)
- The Service layer is used **by** the crypto server to abstract over different backends
- A `HaceBackend` would wrap a `HaceController` (HAL) to implement `OneShot<A>` (Service)

---

## Architectural Insight: Decoupling Resource Recovery

The HAL traits in `hal/blocking/src/digest.rs` bundle two concerns:

1. **Operation semantics:** init/update/finalize flow
2. **Resource recovery:** typestate pattern ensuring controller is returned

```rust
// Current HAL: both concerns bundled
pub mod owned {
    pub trait DigestOp {
        type Output;
        type Controller;
        type Error;
        
        fn update(self, data: &[u8]) -> Result<Self, Self::Error>;
        fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error>;
        fn cancel(self) -> Self::Controller;  // ← Resource recovery
    }
}
```

### Why This Matters for Service Layer

For the crypto server:
- Sessions are **already managed** via session handles (IDs in a table)
- Resource recovery is handled at the **session table level**, not per-operation
- The typestate overhead provides no benefit for software backends

### Proposed Decoupling (Future Work)

Decouple into two orthogonal patterns:

```rust
// 1. Core operation trait (minimal, &mut self)
pub trait DigestOp {
    type Output;
    type Error;
    
    fn update(&mut self, data: &[u8]) -> Result<(), Self::Error>;
    fn finalize(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;
}

// 2. Optional wrapper for resource recovery (typestate)
pub struct Owned<T, C> {
    op: T,
    controller: C,
}

impl<T: DigestOp, C> Owned<T, C> {
    pub fn update(self, data: &[u8]) -> Result<Self, (C, T::Error)> { ... }
    pub fn finalize(self) -> Result<(T::Output, C), (C, T::Error)> { ... }
    pub fn cancel(self) -> C { ... }  // ← Resource recovery here
}
```

### Benefits

| Concern | Baremetal | Server |
|---------|-----------|--------|
| Operation semantics | Uses core `DigestOp` | Uses core `DigestOp` |
| Resource recovery | Wraps in `Owned<T, C>` | Not needed (session table) |

This decoupling is **not required** for the current refactor—the Service layer
traits are already designed without typestate. However, this insight informs
future HAL evolution.

---

## Threading Model Note

Pigweed userspace processes are **single-threaded event loops**. The crypto server:
- Handles requests **sequentially** within a single thread
- Cannot spawn multiple threads to handle concurrent requests
- For true parallelism, would need multiple separate server processes

This simplifies session management—no need for thread-safe session tables or locks.
The single-threaded model also means `&mut self` on `Streaming<A>` methods is safe
without additional synchronization.

---

## Phase 1: Create `crypto-traits` Crate

**Goal:** Define algorithm marker types and backend traits in a minimal, dependency-free crate.

### 1.1 Create directory structure

```
services/crypto/traits/
├── Cargo.toml
├── BUILD.bazel
└── src/
    └── lib.rs
```

### 1.2 Create `Cargo.toml`

```toml
[package]
name = "crypto-traits"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[dependencies]
# Intentionally minimal — no_std, no dependencies
```

### 1.3 Create `src/lib.rs`

Contents:

```rust
#![no_std]

//! Crypto traits for backend abstraction.
//!
//! This crate defines:
//! - Algorithm marker types (Sha256, HmacSha256, etc.)
//! - OneShot<A> trait for one-shot crypto operations
//! - Streaming<A> trait for session-based streaming
//! - CryptoInput enum for semantically typed inputs

// --- Algorithm trait ---
pub trait Algorithm {
    const OUTPUT_SIZE: usize;
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

// --- Signature algorithms (gated) ---
#[cfg(feature = "ecdsa")]
pub struct EcdsaP256Sign;
#[cfg(feature = "ecdsa")]
impl Algorithm for EcdsaP256Sign { const OUTPUT_SIZE: usize = 64; const OP_CODE: u8 = 0x40; }

#[cfg(feature = "ecdsa")]
pub struct EcdsaP256Verify;
#[cfg(feature = "ecdsa")]
impl Algorithm for EcdsaP256Verify { const OUTPUT_SIZE: usize = 1; const OP_CODE: u8 = 0x41; }

#[cfg(feature = "ecdsa")]
pub struct EcdsaP384Sign;
#[cfg(feature = "ecdsa")]
impl Algorithm for EcdsaP384Sign { const OUTPUT_SIZE: usize = 96; const OP_CODE: u8 = 0x42; }

#[cfg(feature = "ecdsa")]
pub struct EcdsaP384Verify;
#[cfg(feature = "ecdsa")]
impl Algorithm for EcdsaP384Verify { const OUTPUT_SIZE: usize = 1; const OP_CODE: u8 = 0x43; }

// --- Structured input types ---
pub enum CryptoInput<'a> {
    Digest { data: &'a [u8] },
    Mac { key: &'a [u8], data: &'a [u8] },
    Aead { key: &'a [u8], nonce: &'a [u8], data: &'a [u8] },
    Sign { private_key: &'a [u8], message: &'a [u8] },
    Verify { public_key: &'a [u8], message: &'a [u8], signature: &'a [u8] },
}

// --- Error type ---
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    InvalidOperation,
    InvalidKeyLength,
    InvalidNonceLength,
    InvalidDataLength,
    InvalidSignature,
    BufferTooSmall,
    AuthenticationFailed,
    HardwareFailure,
}

// --- OneShot trait ---
pub trait OneShot<A: Algorithm> {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError>;
}

// --- Streaming trait ---
pub trait Streaming<A: Algorithm> {
    type Session;
    
    fn begin(&mut self) -> Result<Self::Session, CryptoError>;
    fn update(&mut self, session: &mut Self::Session, data: &[u8]) -> Result<(), CryptoError>;
    fn finalize(&mut self, session: Self::Session, output: &mut [u8]) -> Result<usize, CryptoError>;
    fn cancel(&mut self, session: Self::Session);
}
```

### 1.4 Create `BUILD.bazel`

```python
load("@rules_rust//rust:defs.bzl", "rust_library")

rust_library(
    name = "crypto-traits",
    srcs = ["src/lib.rs"],
    crate_features = select({
        "//target:ecdsa": ["ecdsa"],
        "//conditions:default": [],
    }),
    visibility = ["//visibility:public"],
)
```

### 1.5 Verification

```bash
bazel build //services/crypto/traits:crypto-traits
```

---

## Phase 2: Create `backend-rustcrypto` Crate

**Goal:** Implement `OneShot<A>` for all algorithms using RustCrypto.

### 2.1 Create directory structure

```
services/crypto/backend-rustcrypto/
├── Cargo.toml
├── BUILD.bazel
└── src/
    └── lib.rs
```

### 2.2 Create `Cargo.toml`

```toml
[package]
name = "crypto-backend-rustcrypto"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[features]
default = []
ecdsa = ["crypto-traits/ecdsa", "p256", "p384"]

[dependencies]
crypto-traits = { path = "../traits" }

# RustCrypto
sha2 = { version = "0.10", default-features = false }
hmac = { version = "0.12", default-features = false }
aes-gcm = { version = "0.10", default-features = false, features = ["aes"] }

# ECDSA (optional)
p256 = { version = "0.13", default-features = false, features = ["ecdsa"], optional = true }
p384 = { version = "0.13", default-features = false, features = ["ecdsa"], optional = true }
```

### 2.3 Create `src/lib.rs`

```rust
#![no_std]

use crypto_traits::{
    Algorithm, CryptoError, CryptoInput, OneShot,
    Sha256, Sha384, Sha512,
    HmacSha256, HmacSha384, HmacSha512,
    Aes256GcmEncrypt, Aes256GcmDecrypt,
};

#[cfg(feature = "ecdsa")]
use crypto_traits::{EcdsaP256Sign, EcdsaP256Verify, EcdsaP384Sign, EcdsaP384Verify};

use sha2::Digest as Sha2Digest;
use hmac::{Hmac, Mac};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce as GcmNonce, aead::AeadInPlace};

/// RustCrypto-based backend. Stateless — can be freely copied.
#[derive(Clone, Copy, Default)]
pub struct RustCryptoBackend;

impl RustCryptoBackend {
    pub const fn new() -> Self {
        Self
    }
}

// --- Generic digest helper ---
fn do_digest<D: Sha2Digest>(data: &[u8], output: &mut [u8]) -> Result<usize, CryptoError> {
    let mut hasher = D::new();
    hasher.update(data);
    let result = hasher.finalize();
    let size = result.len();
    if output.len() < size {
        return Err(CryptoError::BufferTooSmall);
    }
    output[..size].copy_from_slice(&result);
    Ok(size)
}

impl OneShot<Sha256> for RustCryptoBackend {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Digest { data } = input else { return Err(CryptoError::InvalidOperation) };
        do_digest::<sha2::Sha256>(data, output)
    }
}

impl OneShot<Sha384> for RustCryptoBackend {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Digest { data } = input else { return Err(CryptoError::InvalidOperation) };
        do_digest::<sha2::Sha384>(data, output)
    }
}

impl OneShot<Sha512> for RustCryptoBackend {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Digest { data } = input else { return Err(CryptoError::InvalidOperation) };
        do_digest::<sha2::Sha512>(data, output)
    }
}

// --- Generic HMAC helper ---
fn do_hmac<D>(key: &[u8], data: &[u8], output: &mut [u8]) -> Result<usize, CryptoError>
where
    D: sha2::Digest + hmac::digest::core_api::BlockSizeUser + Clone,
    Hmac<D>: Mac,
{
    let mut mac = <Hmac<D> as Mac>::new_from_slice(key)
        .map_err(|_| CryptoError::InvalidKeyLength)?;
    mac.update(data);
    let result = mac.finalize().into_bytes();
    let size = result.len();
    if output.len() < size {
        return Err(CryptoError::BufferTooSmall);
    }
    output[..size].copy_from_slice(&result);
    Ok(size)
}

impl OneShot<HmacSha256> for RustCryptoBackend {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Mac { key, data } = input else { return Err(CryptoError::InvalidOperation) };
        do_hmac::<sha2::Sha256>(key, data, output)
    }
}

impl OneShot<HmacSha384> for RustCryptoBackend {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Mac { key, data } = input else { return Err(CryptoError::InvalidOperation) };
        do_hmac::<sha2::Sha384>(key, data, output)
    }
}

impl OneShot<HmacSha512> for RustCryptoBackend {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Mac { key, data } = input else { return Err(CryptoError::InvalidOperation) };
        do_hmac::<sha2::Sha512>(key, data, output)
    }
}

// --- AES-GCM ---
const AES_KEY_SIZE: usize = 32;
const GCM_NONCE_SIZE: usize = 12;
const GCM_TAG_SIZE: usize = 16;

impl OneShot<Aes256GcmEncrypt> for RustCryptoBackend {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Aead { key, nonce, data } = input else { 
            return Err(CryptoError::InvalidOperation) 
        };
        if key.len() != AES_KEY_SIZE {
            return Err(CryptoError::InvalidKeyLength);
        }
        if nonce.len() != GCM_NONCE_SIZE {
            return Err(CryptoError::InvalidNonceLength);
        }
        let output_len = data.len() + GCM_TAG_SIZE;
        if output.len() < output_len {
            return Err(CryptoError::BufferTooSmall);
        }
        
        let key_array: [u8; 32] = key.try_into().map_err(|_| CryptoError::InvalidKeyLength)?;
        let cipher = Aes256Gcm::new(&key_array.into());
        let gcm_nonce = GcmNonce::from_slice(nonce);
        
        output[..data.len()].copy_from_slice(data);
        let tag = cipher.encrypt_in_place_detached(gcm_nonce, &[], &mut output[..data.len()])
            .map_err(|_| CryptoError::HardwareFailure)?;
        output[data.len()..output_len].copy_from_slice(&tag);
        
        Ok(output_len)
    }
}

impl OneShot<Aes256GcmDecrypt> for RustCryptoBackend {
    fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
        let CryptoInput::Aead { key, nonce, data } = input else { 
            return Err(CryptoError::InvalidOperation) 
        };
        if key.len() != AES_KEY_SIZE {
            return Err(CryptoError::InvalidKeyLength);
        }
        if nonce.len() != GCM_NONCE_SIZE {
            return Err(CryptoError::InvalidNonceLength);
        }
        if data.len() < GCM_TAG_SIZE {
            return Err(CryptoError::InvalidDataLength);
        }
        
        let ciphertext_len = data.len() - GCM_TAG_SIZE;
        if output.len() < ciphertext_len {
            return Err(CryptoError::BufferTooSmall);
        }
        
        let key_array: [u8; 32] = key.try_into().map_err(|_| CryptoError::InvalidKeyLength)?;
        let cipher = Aes256Gcm::new(&key_array.into());
        let gcm_nonce = GcmNonce::from_slice(nonce);
        
        let ciphertext = &data[..ciphertext_len];
        let tag = &data[ciphertext_len..];
        
        output[..ciphertext_len].copy_from_slice(ciphertext);
        cipher.decrypt_in_place_detached(
            gcm_nonce,
            &[],
            &mut output[..ciphertext_len],
            tag.into(),
        ).map_err(|_| CryptoError::AuthenticationFailed)?;
        
        Ok(ciphertext_len)
    }
}

// --- ECDSA (feature-gated) ---
#[cfg(feature = "ecdsa")]
mod ecdsa_impl {
    use super::*;
    use p256::ecdsa::{SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey, Signature as P256Signature, signature::{Signer, Verifier}};
    use p384::ecdsa::{SigningKey as P384SigningKey, VerifyingKey as P384VerifyingKey, Signature as P384Signature};
    
    impl OneShot<EcdsaP256Sign> for RustCryptoBackend {
        fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
            let CryptoInput::Sign { private_key, message } = input else {
                return Err(CryptoError::InvalidOperation);
            };
            if private_key.len() != 32 {
                return Err(CryptoError::InvalidKeyLength);
            }
            if output.len() < 64 {
                return Err(CryptoError::BufferTooSmall);
            }
            
            let key = P256SigningKey::from_slice(private_key)
                .map_err(|_| CryptoError::InvalidKeyLength)?;
            let sig: P256Signature = key.sign(message);
            output[..64].copy_from_slice(&sig.to_bytes());
            Ok(64)
        }
    }
    
    impl OneShot<EcdsaP256Verify> for RustCryptoBackend {
        fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
            let CryptoInput::Verify { public_key, message, signature } = input else {
                return Err(CryptoError::InvalidOperation);
            };
            if signature.len() != 64 {
                return Err(CryptoError::InvalidSignature);
            }
            if output.is_empty() {
                return Err(CryptoError::BufferTooSmall);
            }
            
            let key = P256VerifyingKey::from_sec1_bytes(public_key)
                .map_err(|_| CryptoError::InvalidKeyLength)?;
            let sig = P256Signature::from_slice(signature)
                .map_err(|_| CryptoError::InvalidSignature)?;
            
            match key.verify(message, &sig) {
                Ok(()) => {
                    output[0] = 1; // verified
                    Ok(1)
                }
                Err(_) => {
                    output[0] = 0; // failed
                    Ok(1)
                }
            }
        }
    }
    
    impl OneShot<EcdsaP384Sign> for RustCryptoBackend {
        fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
            let CryptoInput::Sign { private_key, message } = input else {
                return Err(CryptoError::InvalidOperation);
            };
            if private_key.len() != 48 {
                return Err(CryptoError::InvalidKeyLength);
            }
            if output.len() < 96 {
                return Err(CryptoError::BufferTooSmall);
            }
            
            let key = P384SigningKey::from_slice(private_key)
                .map_err(|_| CryptoError::InvalidKeyLength)?;
            let sig: P384Signature = Signer::sign(&key, message);
            output[..96].copy_from_slice(&sig.to_bytes());
            Ok(96)
        }
    }
    
    impl OneShot<EcdsaP384Verify> for RustCryptoBackend {
        fn compute(&self, input: &CryptoInput, output: &mut [u8]) -> Result<usize, CryptoError> {
            let CryptoInput::Verify { public_key, message, signature } = input else {
                return Err(CryptoError::InvalidOperation);
            };
            if signature.len() != 96 {
                return Err(CryptoError::InvalidSignature);
            }
            if output.is_empty() {
                return Err(CryptoError::BufferTooSmall);
            }
            
            let key = P384VerifyingKey::from_sec1_bytes(public_key)
                .map_err(|_| CryptoError::InvalidKeyLength)?;
            let sig = P384Signature::from_slice(signature)
                .map_err(|_| CryptoError::InvalidSignature)?;
            
            match Verifier::verify(&key, message, &sig) {
                Ok(()) => {
                    output[0] = 1;
                    Ok(1)
                }
                Err(_) => {
                    output[0] = 0;
                    Ok(1)
                }
            }
        }
    }
}
```

### 2.4 Verification

```bash
bazel build //services/crypto/backend-rustcrypto:crypto-backend-rustcrypto
```

---

## Phase 3: Rewrite Server with `CryptoServer<B>`

**Goal:** Replace the 496-line server with a ~150-line generic implementation.

### 3.1 Modify `services/crypto/server/Cargo.toml`

Replace direct RustCrypto dependencies with trait crate:

```toml
[dependencies]
crypto-api = { path = "../api" }
crypto-traits = { path = "../traits" }
crypto-backend-rustcrypto = { path = "../backend-rustcrypto" }
zerocopy = { version = "0.8", features = ["derive"] }
# Remove: sha2, hmac, aes-gcm, p256, p384
```

### 3.2 Rewrite `services/crypto/server/src/main.rs`

New structure:

```rust
#![no_main]
#![no_std]

use crypto_api::{CryptoError as WireError, CryptoOp, CryptoRequestHeader, CryptoResponseHeader};
use crypto_traits::{
    Algorithm, CryptoError, CryptoInput, OneShot, Streaming,
    Sha256, Sha384, Sha512,
    HmacSha256, HmacSha384, HmacSha512,
    Aes256GcmEncrypt, Aes256GcmDecrypt,
};
#[cfg(feature = "ecdsa")]
use crypto_traits::{EcdsaP256Sign, EcdsaP256Verify, EcdsaP384Sign, EcdsaP384Verify};

use crypto_backend_rustcrypto::RustCryptoBackend;
use pw_status::Result;
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;
use app_crypto_server::handle;

const MAX_REQUEST_SIZE: usize = 1024;
const MAX_RESPONSE_SIZE: usize = 1024;

// Type alias for backend — change this one line to swap backends
type Backend = RustCryptoBackend;

/// Generic crypto server.
struct CryptoServer<B> {
    backend: B,
    // Streaming session state (single session at a time)
    session: StreamingSession,
}

enum StreamingSession {
    None,
    Sha256(Sha256State),
    Sha384(Sha384State),
    Sha512(Sha512State),
}

// Session state types (backend-specific, but hidden behind Streaming<A> trait)
struct Sha256State { /* backend session handle */ }
struct Sha384State { /* backend session handle */ }
struct Sha512State { /* backend session handle */ }

impl<B> CryptoServer<B>
where
    B: OneShot<Sha256> + OneShot<Sha384> + OneShot<Sha512>
     + OneShot<HmacSha256> + OneShot<HmacSha384> + OneShot<HmacSha512>
     + OneShot<Aes256GcmEncrypt> + OneShot<Aes256GcmDecrypt>
     #[cfg(feature = "ecdsa")]
     + OneShot<EcdsaP256Sign> + OneShot<EcdsaP256Verify>
     + OneShot<EcdsaP384Sign> + OneShot<EcdsaP384Verify>
{
    fn new(backend: B) -> Self {
        Self {
            backend,
            session: StreamingSession::None,
        }
    }

    fn dispatch(&self, request: &[u8], response: &mut [u8]) -> usize {
        // Parse header
        if request.len() < CryptoRequestHeader::SIZE {
            return encode_error(response, WireError::InvalidDataLength);
        }
        
        let header_bytes = &request[..CryptoRequestHeader::SIZE];
        let Some(header) = zerocopy::Ref::<_, CryptoRequestHeader>::from_bytes(header_bytes).ok() else {
            return encode_error(response, WireError::InvalidDataLength);
        };
        let header: &CryptoRequestHeader = &*header;
        
        let op = match header.operation() {
            Ok(op) => op,
            Err(e) => return encode_error(response, e),
        };
        
        // Extract fields
        let payload = &request[CryptoRequestHeader::SIZE..];
        let key_len = header.key_length();
        let nonce_len = header.nonce_length();
        let data_len = header.data_length();
        
        if payload.len() < key_len + nonce_len + data_len {
            return encode_error(response, WireError::InvalidDataLength);
        }
        
        let key = &payload[..key_len];
        let nonce = &payload[key_len..key_len + nonce_len];
        let data = &payload[key_len + nonce_len..key_len + nonce_len + data_len];
        
        // Build semantic input
        let input = Self::build_input(op, key, nonce, data);
        
        // Dispatch by op code
        match op {
            CryptoOp::Sha256Hash => self.run::<Sha256>(&input, response),
            CryptoOp::Sha384Hash => self.run::<Sha384>(&input, response),
            CryptoOp::Sha512Hash => self.run::<Sha512>(&input, response),
            CryptoOp::HmacSha256 => self.run::<HmacSha256>(&input, response),
            CryptoOp::HmacSha384 => self.run::<HmacSha384>(&input, response),
            CryptoOp::HmacSha512 => self.run::<HmacSha512>(&input, response),
            CryptoOp::Aes256GcmEncrypt => self.run::<Aes256GcmEncrypt>(&input, response),
            CryptoOp::Aes256GcmDecrypt => self.run::<Aes256GcmDecrypt>(&input, response),
            #[cfg(feature = "ecdsa")]
            CryptoOp::EcdsaP256Sign => self.run::<EcdsaP256Sign>(&input, response),
            #[cfg(feature = "ecdsa")]
            CryptoOp::EcdsaP256Verify => self.run::<EcdsaP256Verify>(&input, response),
            #[cfg(feature = "ecdsa")]
            CryptoOp::EcdsaP384Sign => self.run::<EcdsaP384Sign>(&input, response),
            #[cfg(feature = "ecdsa")]
            CryptoOp::EcdsaP384Verify => self.run::<EcdsaP384Verify>(&input, response),
            // Streaming ops (Phase 4)
            CryptoOp::Sha256Begin | CryptoOp::Sha256Update | CryptoOp::Sha256Finish |
            CryptoOp::Sha384Begin | CryptoOp::Sha384Update | CryptoOp::Sha384Finish |
            CryptoOp::Sha512Begin | CryptoOp::Sha512Update | CryptoOp::Sha512Finish => {
                encode_error(response, WireError::InvalidOperation) // TODO Phase 4
            }
            #[cfg(not(feature = "ecdsa"))]
            CryptoOp::EcdsaP256Sign | CryptoOp::EcdsaP256Verify |
            CryptoOp::EcdsaP384Sign | CryptoOp::EcdsaP384Verify => {
                encode_error(response, WireError::InvalidOperation)
            }
        }
    }

    fn build_input<'a>(op: CryptoOp, key: &'a [u8], nonce: &'a [u8], data: &'a [u8]) -> CryptoInput<'a> {
        match op {
            CryptoOp::Sha256Hash | CryptoOp::Sha384Hash | CryptoOp::Sha512Hash => 
                CryptoInput::Digest { data },
            CryptoOp::HmacSha256 | CryptoOp::HmacSha384 | CryptoOp::HmacSha512 => 
                CryptoInput::Mac { key, data },
            CryptoOp::Aes256GcmEncrypt | CryptoOp::Aes256GcmDecrypt => 
                CryptoInput::Aead { key, nonce, data },
            CryptoOp::EcdsaP256Sign | CryptoOp::EcdsaP384Sign => 
                CryptoInput::Sign { private_key: key, message: data },
            CryptoOp::EcdsaP256Verify | CryptoOp::EcdsaP384Verify => 
                CryptoInput::Verify { public_key: key, message: data, signature: nonce },
            _ => CryptoInput::Digest { data: &[] }, // streaming handled separately
        }
    }

    fn run<A: Algorithm>(&self, input: &CryptoInput, response: &mut [u8]) -> usize
    where
        B: OneShot<A>,
    {
        let result_start = CryptoResponseHeader::SIZE;
        match self.backend.compute(input, &mut response[result_start..]) {
            Ok(len) => encode_success(response, len),
            Err(e) => encode_error(response, map_error(e)),
        }
    }
}

fn encode_error(response: &mut [u8], err: WireError) -> usize {
    let header = CryptoResponseHeader::error(err);
    let header_bytes = zerocopy::IntoBytes::as_bytes(&header);
    response[..CryptoResponseHeader::SIZE].copy_from_slice(header_bytes);
    CryptoResponseHeader::SIZE
}

fn encode_success(response: &mut [u8], result_len: usize) -> usize {
    let header = CryptoResponseHeader::success(result_len as u16);
    let header_bytes = zerocopy::IntoBytes::as_bytes(&header);
    response[..CryptoResponseHeader::SIZE].copy_from_slice(header_bytes);
    CryptoResponseHeader::SIZE + result_len
}

fn map_error(e: CryptoError) -> WireError {
    match e {
        CryptoError::InvalidOperation => WireError::InvalidOperation,
        CryptoError::InvalidKeyLength => WireError::InvalidKeyLength,
        CryptoError::InvalidNonceLength => WireError::InvalidNonceLength,
        CryptoError::InvalidDataLength => WireError::InvalidDataLength,
        CryptoError::InvalidSignature => WireError::InvalidSignature,
        CryptoError::BufferTooSmall => WireError::BufferTooSmall,
        CryptoError::AuthenticationFailed => WireError::AuthenticationFailed,
        CryptoError::HardwareFailure => WireError::HardwareFailure,
    }
}

#[entry]
fn main() -> ! {
    match crypto_server_loop() {
        Ok(()) => unreachable!(),
        Err(e) => {
            pw_log::error!("Crypto server error: {:?}", e);
            loop { cortex_m::asm::wfi(); }
        }
    }
}

fn crypto_server_loop() -> Result<()> {
    pw_log::info!("Crypto server starting");
    
    let server = CryptoServer::new(RustCryptoBackend::new());
    let mut request_buf = [0u8; MAX_REQUEST_SIZE];
    let mut response_buf = [0u8; MAX_RESPONSE_SIZE];

    loop {
        syscall::object_wait(handle::CRYPTO, Signals::READABLE, Instant::MAX)?;
        let len = syscall::channel_read(handle::CRYPTO, 0, &mut request_buf)?;
        let response_len = server.dispatch(&request_buf[..len], &mut response_buf);
        syscall::channel_respond(handle::CRYPTO, &response_buf[..response_len])?;
    }
}
```

### 3.3 Verification

```bash
bazel test --config=virt_ast1060_evb //target/ast1060/crypto:crypto_test --test_output=all
```

All existing tests must pass unchanged.

---

## Phase 4: Add Streaming Support via `Streaming<A>` Trait

**Goal:** Implement streaming hash using the trait-based design.

### 4.1 Add to `crypto-traits/src/lib.rs`

Already defined in Phase 1. Add session state types.

### 4.2 Add to `backend-rustcrypto/src/lib.rs`

```rust
use sha2::{Sha256 as Sha2_256, Sha384 as Sha2_384, Sha512 as Sha2_512};

pub struct Sha256Session(Sha2_256);
pub struct Sha384Session(Sha2_384);
pub struct Sha512Session(Sha2_512);

impl Streaming<Sha256> for RustCryptoBackend {
    type Session = Sha256Session;
    
    fn begin(&mut self) -> Result<Self::Session, CryptoError> {
        Ok(Sha256Session(Sha2_256::new()))
    }
    
    fn update(&mut self, session: &mut Self::Session, data: &[u8]) -> Result<(), CryptoError> {
        session.0.update(data);
        Ok(())
    }
    
    fn finalize(&mut self, session: Self::Session, output: &mut [u8]) -> Result<usize, CryptoError> {
        if output.len() < 32 {
            return Err(CryptoError::BufferTooSmall);
        }
        let result = session.0.finalize();
        output[..32].copy_from_slice(&result);
        Ok(32)
    }
    
    fn cancel(&mut self, _session: Self::Session) {
        // Session dropped, nothing to clean up for software impl
    }
}

// Similar for Sha384, Sha512
```

### 4.3 Update server dispatch

Add streaming op handling in `CryptoServer::dispatch()`.

### 4.4 Verification

```bash
bazel test --config=virt_ast1060_evb //target/ast1060/crypto:crypto_test --test_output=all
```

Streaming test (`test_sha256_streaming`) must pass.

---

## Phase 5: HACE Backend (Future)

**Goal:** Add hardware-accelerated backend for AST1060.

### 5.1 Create `backend-hace` crate

```
services/crypto/backend-hace/
├── Cargo.toml
├── BUILD.bazel
└── src/
    └── lib.rs
```

### 5.2 Implement `OneShot<A>` and `Streaming<A>`

Wrap the `HaceController` from `aspeed-rust` to implement the traits.

### 5.3 Feature-gate in server

```rust
#[cfg(feature = "hace")]
type Backend = HaceBackend;

#[cfg(not(feature = "hace"))]
type Backend = RustCryptoBackend;
```

---

## Task Summary

| Phase | Task | New Files | Modified | LOC | Effort |
|-------|------|-----------|----------|-----|--------|
| 1 | Create `crypto-traits` crate | 3 | 0 | ~150 | 0.5 day |
| 2 | Create `backend-rustcrypto` crate | 3 | 0 | ~300 | 1 day |
| 3 | Rewrite server with `CryptoServer<B>` | 0 | 2 | ~150 (was 496) | 1 day |
| 4 | Add streaming via `Streaming<A>` | 0 | 3 | ~100 | 0.5 day |
| 5 | HACE backend (future) | 3 | 1 | ~300 | 3 days |
| **Total** | | **9** | **6** | **~1000** | **~6 days** |

---

## What Happens to Existing Code

| Component | Action |
|-----------|--------|
| `hal/blocking/src/digest.rs` | **Keep** — still used by non-crypto-server code |
| `hal/blocking/src/mac.rs` | **Keep** — still used by non-crypto-server code |
| `platform/impls/rustcrypto/src/controller.rs` | **Reference only** — mine for implementation patterns |
| `services/crypto/server/src/main.rs` | **Rewrite** — becomes generic `CryptoServer<B>` |
| `services/crypto/api/` | **Keep** — wire protocol unchanged |
| `services/crypto/client/` | **Keep** — client API unchanged |
| `services/crypto/tests/` | **Keep** — tests pass without modification |

---

## Code Organization After Refactor

```
services/crypto/
├── traits/                          # NEW: Algorithm markers + OneShot/Streaming traits
│   └── src/lib.rs                   #      ~150 lines, no dependencies
├── backend-rustcrypto/              # NEW: OneShot<A> impls using RustCrypto
│   └── src/lib.rs                   #      ~300 lines
├── backend-hace/                    # FUTURE: OneShot<A> impls using HACE
│   └── src/lib.rs                   #      ~300 lines
├── api/                             # UNCHANGED: Wire protocol
│   └── src/protocol.rs
├── server/                          # REWRITTEN: Generic CryptoServer<B>
│   └── src/main.rs                  #      ~150 lines (was 496)
├── client/                          # UNCHANGED: Client library
│   └── src/lib.rs
└── tests/                           # UNCHANGED: Integration tests
    └── src/main.rs
```

---

## Verification Checkpoints

After each phase:

1. **Build:** `bazel build //services/crypto/...`
2. **Lint:** `bazel build //services/crypto/... --aspects=@aspect_rules_lint//lint:lint.bzl%clippy_lints`
3. **Test:** `bazel test --config=virt_ast1060_evb //target/ast1060/crypto:crypto_test --test_output=all`
4. **Commit:** One commit per phase with descriptive message

---

## Rollback Plan

Each phase is independently deployable:
- Phase 1-2 are additive (no existing code changes)
- Phase 3 is the critical rewrite — keep old server as `main.rs.bak` until tests pass
- Phase 4-5 are additive features

---

## Decision Record

**Date:** February 13, 2026

**Decision:** Implement new composable `OneShot<A>` / `Streaming<A>` traits rather
than reusing existing `openprot-hal-blocking` + `RustCryptoController`.

**Rationale:**
1. Current architecture uses "God trait" pattern requiring all backends to implement all algorithms
2. Adding new algorithms touches shared trait definitions, violating Open-Closed Principle
3. SPDM spec requires 10-15 additional algorithms; current approach would be 3-4 files per algorithm
4. Composable traits allow adding algorithms in 1-2 files with no modifications to existing code
5. HAL traits bundle resource recovery (typestate) with operation semantics—unnecessary for server
6. Service layer traits use session handles, better fit for IPC-based state management

**Tradeoff:** ~6 days upfront work vs. ongoing maintenance cost for 2+ years of algorithm additions.

**Accepted by:** User (February 13, 2026)

**Updated:** February 14, 2026 — Added architectural insights on trait layering and resource recovery decoupling

---

*Ready to execute.*
