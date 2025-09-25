# SPDM-lib to Hubris IPC Integration Guide

## Overview

This guide explains how to integrate the existing `spdm-lib` SPDM protocol implementation with Hubris IPC, bridging between the comprehensive spdm-lib types and the simplified IPC-compatible types required for inter-task communication.

## Integration Architecture

```
┌─────────────────┐    ┌──────────────────────────┐    ┌─────────────────────┐
│ SPDM Client     │    │ Hubris IPC Boundary      │    │ SPDM Server         │
│ (Other Task)    │    │                          │    │ (spdm-lib context)  │
│                 │    │ - Simple IPC Types       │    │                     │
│ - SpdmResponder │◄──►│ - Serializable           │◄──►│ - Full spdm-lib     │
│ - SpdmVersion   │    │ - no_std compatible      │    │ - SpdmContext       │
│ - SpdmError     │    │                          │    │ - Platform traits   │
└─────────────────┘    └──────────────────────────┘    └─────────────────────┘
```

## Type Mapping Strategy

### 1. IPC Types (Client-Side)
Located in `drv/spdm-responder-api/src/lib.rs`:

```rust
// Simple, serializable IPC types
#[derive(Serialize, Deserialize, SerializedSize)]
pub enum SpdmVersion {
    V1_0 = 0x10,
    V1_1 = 0x11,
    V1_2 = 0x12,
    V1_3 = 0x13,
}

#[derive(Serialize, Deserialize, SerializedSize)]
pub struct SpdmCapabilities {
    pub ct_exponent: u8,
    pub flags: u32, // Flattened from bitfield
    pub data_transfer_size: u32,
    pub max_spdm_msg_size: u32,
}

#[derive(IdolError, Serialize, Deserialize, SerializedSize)]
pub enum SpdmError {
    UnsupportedVersion = 1,
    InvalidParam = 2,
    // ... flattened error hierarchy
}
```

### 2. spdm-lib Types (Server-Side)
Located in `spdm-lib/src/protocol/`:

```rust
// Rich, protocol-complete spdm-lib types
pub enum SpdmVersion {
    V10, V11, V12, V13
}

pub struct DeviceCapabilities {
    pub ct_exponent: u8,
    pub flags: CapabilityFlags, // Complex bitfield
    pub data_transfer_size: u32,
    pub max_spdm_msg_size: u32,
}

pub enum SpdmError {
    UnsupportedVersion,
    InvalidParam,
    Codec(CodecError),     // Nested errors
    Transport(TransportError),
    Command(CommandError),
    // ...
}
```

### 3. Server-Side Conversion Layer
Located in `drv/spdm-responder-server/src/main.rs`:

```rust
use spdm_lib::protocol::version::SpdmVersion as LibSpdmVersion;
use spdm_lib::protocol::capabilities::DeviceCapabilities as LibCapabilities;
use spdm_lib::error::SpdmError as LibSpdmError;

impl InOrderSpdmResponderImpl for SpdmResponderServer {
    fn get_version(&mut self, _msg: &RecvMessage) -> Result<SpdmVersionResponse, RequestError<SpdmError>> {
        // 1. Use spdm-lib to get actual supported versions
        let lib_versions = self.spdm_context.supported_versions;

        // 2. Convert spdm-lib types to IPC types
        let ipc_versions: heapless::Vec<SpdmVersion, 4> = lib_versions
            .iter()
            .map(|&v| convert_version(v))
            .collect();

        // 3. Return IPC-compatible response
        Ok(SpdmVersionResponse {
            version_count: ipc_versions.len() as u8,
            versions: ipc_versions.into_array().unwrap_or_default(),
        })
    }

    fn get_capabilities(&mut self, _msg: &RecvMessage) -> Result<SpdmCapabilities, RequestError<SpdmError>> {
        // 1. Get capabilities from spdm-lib
        let lib_caps = self.spdm_context.local_capabilities;

        // 2. Convert to IPC format
        Ok(SpdmCapabilities {
            ct_exponent: lib_caps.ct_exponent,
            flags: lib_caps.flags.0, // Extract bitfield value
            data_transfer_size: lib_caps.data_transfer_size,
            max_spdm_msg_size: lib_caps.max_spdm_msg_size,
        })
    }
}

// Conversion functions
fn convert_version(lib_ver: LibSpdmVersion) -> SpdmVersion {
    match lib_ver {
        LibSpdmVersion::V10 => SpdmVersion::V1_0,
        LibSpdmVersion::V11 => SpdmVersion::V1_1,
        LibSpdmVersion::V12 => SpdmVersion::V1_2,
        LibSpdmVersion::V13 => SpdmVersion::V1_3,
    }
}

fn convert_error(lib_err: LibSpdmError) -> SpdmError {
    match lib_err {
        LibSpdmError::UnsupportedVersion => SpdmError::UnsupportedVersion,
        LibSpdmError::InvalidParam => SpdmError::InvalidParam,
        LibSpdmError::Codec(_) => SpdmError::CodecError,
        LibSpdmError::Transport(_) => SpdmError::TransportError,
        LibSpdmError::Command(_) => SpdmError::CommandError,
        LibSpdmError::BufferTooSmall => SpdmError::BufferTooSmall,
        LibSpdmError::UnsupportedRequest => SpdmError::UnsupportedRequest,
        LibSpdmError::CertStore(_) => SpdmError::CertStoreError,
    }
}
```

## Implementation Steps

### Phase 1: API Crate (IPC Types Only)
**Location**: `drv/spdm-responder-api/`
**Dependencies**: No spdm-lib dependency
**Responsibility**: Define simple, serializable types for IPC

```toml
# drv/spdm-responder-api/Cargo.toml
[dependencies]
# Standard Hubris IPC dependencies only
hubpack = { workspace = true }
serde = { workspace = true }
derive-idol-err = { path = "../../lib/derive-idol-err" }
# NO spdm-lib dependency
```

### Phase 2: Server Crate (Integration Layer)
**Location**: `drv/spdm-responder-server/`
**Dependencies**: Both IPC types AND spdm-lib
**Responsibility**: Convert between type systems

```toml
# drv/spdm-responder-server/Cargo.toml
[dependencies]
drv-spdm-responder-api = { path = "../spdm-responder-api" }
spdm-lib = { path = "../../spdm-lib" }
# ... other server dependencies
```

### Phase 3: Platform Integration
**Location**: `drv/spdm-responder-server/src/platform/`
**Responsibility**: Implement spdm-lib platform traits for Hubris

```rust
// Hubris-specific platform implementations
struct HubrisTransport {
    // Use Hubris IPC for SPDM transport
}

impl spdm_lib::platform::transport::SpdmTransport for HubrisTransport {
    // Implement using Hubris communication primitives
}

struct HubrisRng {
    rng_client: RngClient,
}

impl spdm_lib::platform::rng::SpdmRng for HubrisRng {
    fn fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), SpdmRngError> {
        self.rng_client.fill_bytes(dest)
            .map_err(|_| SpdmRngError::GenerationFailed)
    }
}

struct HubrisHash {
    hash_client: HashClient,
}

impl spdm_lib::platform::hash::SpdmHash for HubrisHash {
    fn hash_sha256(&mut self, data: &[u8], output: &mut [u8]) -> Result<(), SpdmHashError> {
        self.hash_client.digest_sha256(data, output)
            .map_err(|_| SpdmHashError::HashFailed)
    }
}
```

## Key Benefits of This Approach

### 1. **Separation of Concerns**
- **API Crate**: Simple IPC types, no complex dependencies
- **Server Crate**: Full protocol implementation with conversions
- **Clear Boundary**: Type conversion happens at server boundary

### 2. **Dependency Management**
- Client tasks only depend on lightweight IPC types
- Server task handles complex spdm-lib dependencies
- No memory allocator issues in client code

### 3. **Standards Compliance**
- Server uses proven spdm-lib implementation
- Full SPDM protocol support with proper state management
- Platform abstractions allow Hubris-specific optimizations

### 4. **Incremental Implementation**
- Start with basic operations (get_version, get_capabilities)
- Add complex operations (certificates, measurements) progressively
- Maintain IPC interface stability while enhancing server

## Error Handling Strategy

### IPC Error Flattening
spdm-lib has rich, nested error types that must be flattened for IPC:

```rust
// spdm-lib: Rich error hierarchy
pub enum SpdmError {
    Command(CommandError),
    Codec(CodecError),
    Transport(TransportError(IoError)),
}

// IPC: Flattened error codes
pub enum SpdmError {
    CommandError = 5,
    CodecError = 3,
    TransportError = 4,
}
```

### Server-Side Error Conversion
The server performs lossy but sufficient error conversion:

```rust
fn handle_spdm_operation(&mut self) -> Result<Response, RequestError<SpdmError>> {
    match self.spdm_context.some_operation() {
        Ok(result) => Ok(convert_response(result)),
        Err(lib_error) => Err(RequestError::from(convert_error(lib_error))),
    }
}
```

## Certificate and Measurement Integration

### Large Data Handling
SPDM certificates and measurements require special handling:

```rust
fn get_certificate(
    &mut self,
    slot: u8,
    offset: u16,
    length: u16,
    buffer: Leased<W, [u8]>,
) -> Result<u16, RequestError<SpdmError>> {
    // 1. Get certificate from spdm-lib cert store
    let cert_data = self.spdm_context.cert_store.get_certificate(slot)?;

    // 2. Handle chunked transfer using IPC lease
    let available = cert_data.len().saturating_sub(offset as usize);
    let copy_len = core::cmp::min(length as usize, available);

    if copy_len > 0 {
        buffer.write_range(0..copy_len, &cert_data[offset as usize..])
            .map_err(|_| RequestError::went_away())?;
    }

    // 3. Return actual bytes copied
    Ok(copy_len as u16)
}
```

## Testing Strategy

### Unit Tests
- Test type conversions in isolation
- Verify error code mappings
- Test edge cases (empty responses, large data)

### Integration Tests
- End-to-end IPC communication
- spdm-lib integration with Hubris platform traits
- Performance testing with real hardware acceleration

### Compliance Tests
- SPDM protocol conformance using spdm-lib test suite
- Interoperability with external SPDM requesters

## Migration from Current State

### Current Implementation
We currently have placeholder types in the IPC interface.

### Migration Steps
1. **Keep IPC Types**: Maintain current simplified types in API crate
2. **Add Server Integration**: Import spdm-lib only in server crate
3. **Implement Conversions**: Add type conversion functions
4. **Replace Mocks**: Replace mock implementations with real spdm-lib calls
5. **Add Platform Traits**: Implement Hubris-specific platform integration

This approach allows us to leverage the full power of spdm-lib while maintaining clean IPC boundaries and avoiding dependency issues in client code.