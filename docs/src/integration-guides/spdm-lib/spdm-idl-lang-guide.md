# SPDM Responder IDL Language Guide

This guide shows how to use Hubris IDL language constructs to design an `idl/spdm-responder.idol`, serving as both a reference for the SPDM interface and a practical guide to IDL language features.

## IDL Structure Overview

The SPDM responder interface demonstrates a complete Hubris IDL definition with all major language constructs:

```idl
Interface(
    name: "SpdmResponder",
    ops: { /* operation definitions */ }
)
```

## Core Language Constructs

### 1. Interface Declaration

```idl
Interface(
    name: "SpdmResponder",
    ops: { ... }
)
```

**Purpose**: Defines a named IPC interface that generates client stubs and server traits.
**Generated Code**:
- Client: `SpdmResponder` struct with methods
- Server: `InOrderSpdmResponderImpl` trait to implement

### 2. Operation Definitions

Each operation in the `ops` block defines an IPC method:

```idl
"get_version": (
    doc: "Get supported SPDM protocol versions in priority order",
    reply: Result(ok: "SpdmVersionResponse", err: CLike("SpdmError")),
    encoding: Hubpack,
    idempotent: true,
),
```

**Components**:
- **name**: Method name (becomes Rust function name)
- **doc**: Documentation string for generated code
- **reply**: Return type specification
- **encoding**: Serialization format (always `Hubpack` in Hubris)
- **idempotent**: Whether operation can be safely retried

### 3. Result Types

```idl
reply: Result(
    ok: "SpdmVersionResponse",
    err: CLike("SpdmError"),
)
```

**Purpose**: Defines fallible operations that return `Result<T, E>`
**Components**:
- **ok**: Success type (references a Rust type)
- **err**: Error type using `CLike` enum wrapper

### 4. CLike Error Types

```idl
err: CLike("SpdmError")
```

**Purpose**: Wraps a C-like enum for error handling
**Generated**: `RequestError<SpdmError>` in server implementations
**Requirement**: Error type must implement `IdolError` trait

### 5. Arguments Block

```idl
args: {
    "slot": "u8",
    "offset": "u16",
    "length": "u16",
}
```

**Purpose**: Defines method parameters passed by value
**Encoding**: Serialized with specified encoding (Hubpack)
**Types**: Supports primitive types, structs, enums, and arrays

### 6. Leases Block

```idl
leases: {
    "certificate": (type: "[u8]", write: true, max_len: Some(4096)),
    "nonce": (type: "[u8]", read: true, max_len: Some(64)),
}
```

**Purpose**: Defines zero-copy buffer passing between tasks
**Components**:
- **type**: Buffer element type (usually `[u8]` for byte arrays)
- **read/write**: Access direction from server perspective
- **max_len**: Optional size limit for safety

**Lease Types**:
- `read: true` → Server reads from client buffer
- `write: true` → Server writes to client buffer
- Both → Bidirectional access

### 7. Idempotency Markers

```idl
idempotent: true   // Safe to retry, no side effects
idempotent: false  // May change state, avoid retries
```

**Purpose**: Indicates whether operations can be safely retried
**Usage**: Critical for fault tolerance and transaction semantics

## Advanced Type Specifications

### Optional Types

```idl
"nonce": "Option<[u8; 32]>"
```

**Purpose**: Parameters that may or may not be present
**Generated**: Standard Rust `Option<T>` type

### Fixed Arrays

```idl
"nonce": "[u8; 32]"
```

**Purpose**: Fixed-size arrays with compile-time known length
**Serialization**: Efficient direct memory copy

### Unit Type

```idl
reply: Result(ok: "()", err: CLike("SpdmError"))
```

**Purpose**: Operations that return no data on success
**Usage**: Status operations, void functions

## SPDM-Specific Patterns

### Certificate Operations

```idl
"get_certificate": (
    args: {
        "slot": "u8",
        "offset": "u16",
        "length": "u16",
    },
    leases: {
        "certificate": (type: "[u8]", write: true, max_len: Some(4096)),
    },
    reply: Result(ok: "u16", err: CLike("SpdmError")),
)
```

**Pattern**: Streaming large data with offset/length parameters
**Return**: Actual bytes written (partial read support)

### Challenge-Response Authentication

```idl
"challenge_auth": (
    args: {
        "slot": "u8",
        "measurement_summary": "u8",
    },
    leases: {
        "nonce": (type: "[u8]", read: true, max_len: Some(64)),
        "signature": (type: "[u8]", write: true, max_len: Some(512)),
    },
    reply: Result(ok: "ChallengeAuthResponse", err: CLike("SpdmError")),
)
```

**Pattern**: Input challenge → cryptographic response
**Security**: Separates input (nonce) from output (signature)

### Key Exchange Operations

```idl
"key_exchange": (
    leases: {
        "req_random": (type: "[u8]", read: true, max_len: Some(64)),
        "rsp_random": (type: "[u8]", write: true, max_len: Some(64)),
        "exchange_data": (type: "[u8]", read: true, max_len: Some(512)),
        "rsp_exchange_data": (type: "[u8]", write: true, max_len: Some(512)),
    },
)
```

**Pattern**: Bidirectional cryptographic data exchange
**Security**: Clear separation of request/response buffers

## Code Generation Results

### Client Side (API Crate)

```rust
// Generated from IDL
pub struct SpdmResponder { /* task_id */ }

impl SpdmResponder {
    pub fn get_version(&self) -> Result<SpdmVersionResponse, RequestError<SpdmError>> {
        // IPC call implementation
    }

    pub fn get_certificate(
        &self,
        slot: u8,
        offset: u16,
        length: u16,
        certificate: Leased<W, [u8]>
    ) -> Result<u16, RequestError<SpdmError>> {
        // IPC call with lease
    }
}
```

### Server Side (Server Crate)

```rust
// Generated trait to implement
pub trait InOrderSpdmResponderImpl: NotificationHandler {
    fn get_version(
        &mut self,
        msg: &RecvMessage,
    ) -> Result<SpdmVersionResponse, RequestError<SpdmError>>;

    fn get_certificate(
        &mut self,
        msg: &RecvMessage,
        slot: u8,
        offset: u16,
        length: u16,
        certificate: LenLimit<Leased<W, [u8]>, 4096>,
    ) -> Result<u16, RequestError<SpdmError>>;
}
```

## Best Practices Demonstrated

### 1. Consistent Error Handling
- All operations return `Result` with same error type
- Enables uniform error handling across interface

### 2. Appropriate Idempotency
- Read operations: `idempotent: true`
- State-changing operations: `idempotent: false`
- Critical for reliable distributed systems

### 3. Zero-Copy Buffer Management
- Large data uses leases (certificates, signatures)
- Small data uses args (slot numbers, flags)
- Optimal memory usage and performance

### 4. Security Boundaries
- Input/output buffers clearly separated
- Buffer size limits prevent overflow
- Cryptographic operations isolated

### 5. Comprehensive Documentation
- Every operation has descriptive `doc` string
- Generated API includes full documentation
- Self-documenting interface design

## Integration with Hubris

### Task Slots
```toml
# In app.toml
[tasks.spdm_responder]
name = "drv-spdm-responder-server"

[tasks.client]
task-slots = ["spdm_responder"]
```

### Client Usage
```rust
task_slot!(SPDM, spdm_responder);

let spdm = SpdmResponder::from(SPDM.get_task_id());
let version = spdm.get_version()?;
```

This IDL interface demonstrates modern IPC design with security, performance, and maintainability as core principles.