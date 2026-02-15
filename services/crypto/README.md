# Crypto Service

IPC-based cryptographic service for the Pigweed kernel, providing hash, MAC, AEAD, and digital signature operations to unprivileged tasks.

## Architecture

```
┌─────────────────┐     IPC      ┌─────────────────┐
│  Crypto Client  │─────────────▶│  Crypto Server  │
│   (user task)   │◀─────────────│  (RustCrypto)   │
└─────────────────┘              └─────────────────┘
```

- **Client** (`crypto_client`) — Ergonomic Rust API for applications
- **Server** (`crypto_server`) — Implements operations using RustCrypto crates  
- **API** (`crypto_api`) — Wire protocol definitions and backend traits

## Supported Algorithms

| Category | Algorithm | Output Size | Notes |
|----------|-----------|-------------|-------|
| **Hash** | SHA-256 | 32 bytes | FIPS 180-4 |
| | SHA-384 | 48 bytes | FIPS 180-4 |
| | SHA-512 | 64 bytes | FIPS 180-4 |
| **MAC** | HMAC-SHA256 | 32 bytes | RFC 2104 |
| | HMAC-SHA384 | 48 bytes | RFC 2104 |
| | HMAC-SHA512 | 64 bytes | RFC 2104 |
| **AEAD** | AES-256-GCM | 16-byte tag | NIST SP 800-38D |
| **Signature**† | ECDSA P-256 | 64 bytes | RFC 6979 (deterministic) |
| | ECDSA P-384 | 96 bytes | RFC 6979 (deterministic) |

† ECDSA requires `crate_features = ["ecdsa"]` on client and server.

## Client API

```rust
use crypto_client::CryptoClient;

// Bind to the crypto server channel
let crypto = CryptoClient::new(handle::CRYPTO);

// Hashing — returns fixed-size array directly
let hash: [u8; 32] = crypto.sha256(b"hello world")?;
let hash: [u8; 48] = crypto.sha384(data)?;
let hash: [u8; 64] = crypto.sha512(data)?;

// HMAC — returns tag directly  
let tag: [u8; 32] = crypto.hmac_sha256(key, data)?;
let tag: [u8; 48] = crypto.hmac_sha384(key, data)?;
let tag: [u8; 64] = crypto.hmac_sha512(key, data)?;

// AES-256-GCM seal/open
let ct_len = crypto.aes256_gcm_seal(&key, &nonce, plaintext, &mut ciphertext)?;
let pt_len = crypto.aes256_gcm_open(&key, &nonce, &ciphertext[..ct_len], &mut plaintext)?;

// ECDSA (requires "ecdsa" feature)
let sig: [u8; 64] = crypto.ecdsa_p256_sign(&private_key, message)?;
crypto.ecdsa_p256_verify(&public_key, message, &sig)?;  // Ok(()) = valid
```

## Error Handling

```rust
pub enum ClientError {
    IpcError(pw_status::Error),   // Channel failure
    ServerError(CryptoError),      // Crypto operation failed
    InvalidResponse,               // Malformed response
    BufferTooSmall,               // Output buffer insufficient
    VerificationFailed,           // Signature/tag invalid
}
```

`ClientError` implements `Display` for logging.

## Wire Protocol

Request format:
```
┌────────────────┬────────────────┬────────────────┬────────────────┐
│   opcode (1)   │  key_len (2)   │ nonce_len (1)  │  data_len (2)  │
├────────────────┴────────────────┴────────────────┴────────────────┤
│                          key bytes                                │
├───────────────────────────────────────────────────────────────────┤
│                         nonce bytes                               │
├───────────────────────────────────────────────────────────────────┤
│                         data bytes                                │
└───────────────────────────────────────────────────────────────────┘
```

Response format:
```
┌────────────────┬────────────────┬─────────────────────────────────┐
│   status (1)   │  reserved (1)  │          data_len (2)           │
├────────────────┴────────────────┴─────────────────────────────────┤
│                         result bytes                              │
└───────────────────────────────────────────────────────────────────┘
```

## Build

```bash
# Build for AST1060 QEMU target
bazel build --config=k_qemu_ast1060 //target/ast1060/crypto:crypto

# Run tests in QEMU
bazel test --config=k_qemu_ast1060 //target/ast1060/crypto:crypto_test --test_output=all

# Build for AST1060-EVB hardware target
bazel build --config=k_ast1060_evb //target/ast1060-evb/crypto:crypto
```

## Binary Sizes

| Component | Flash | RAM | Stack |
|-----------|-------|-----|-------|
| Client | 5.7 KB | 16 KB | 4 KB |
| Server | 42.7 KB | 48 KB | 8 KB |
| Kernel | 19.0 KB | 128 KB | — |

System image: 299 KB (`.bin`), 579 KB (`.elf`)

## Directory Structure

```
services/crypto/
├── api/          # Wire protocol, CryptoOp enum, backend traits
├── client/       # CryptoClient API for applications
├── server/       # RustCrypto-backed server implementation
└── tests/        # Functional tests (runs in QEMU)
```

## License

Apache-2.0
