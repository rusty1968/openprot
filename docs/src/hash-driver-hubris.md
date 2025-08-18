# Generic Digest Server Design Document

## Requirements

### Primary Requirement

**Enable SPDM and PLDM Protocol Support**: The digest server must provide cryptographic hash services to support both SPDM (Security Protocol and Data Model) and PLDM (Platform Level Data Model) protocol implementations in Hubris OS.

### Derived Requirements

#### R1: Algorithm Support
- **R1.1**: Support SHA-256 for basic SPDM operations and PLDM firmware integrity validation
- **R1.2**: Support SHA-384 for enhanced security profiles in both SPDM and PLDM  
- **R1.3**: Support SHA-512 for maximum security assurance
- **R1.4**: Reject unsupported algorithms (SHA-3) with clear error codes

#### R2: Session Management
- **R2.1**: Support incremental hash computation for large certificate chains and firmware images
- **R2.2**: Support multiple concurrent digest sessions (≥8 concurrent operations)
- **R2.3**: Provide session isolation between different SPDM and PLDM protocol flows
- **R2.4**: Automatic session cleanup to prevent resource exhaustion
- **R2.5**: Session timeout mechanism for abandoned operations

#### R3: SPDM and PLDM Use Cases
- **R3.1**: Certificate chain verification (hash large X.509 certificate data)
- **R3.2**: Measurement verification (hash firmware measurement data)
- **R3.3**: Challenge-response authentication (compute transcript hashes)
- **R3.4**: Session key derivation (hash key exchange material)
- **R3.5**: Message authentication (hash SPDM message sequences)
- **R3.6**: PLDM firmware image integrity validation (hash received firmware chunks)
- **R3.7**: PLDM component image verification (validate assembled image against manifest digest)
- **R3.8**: PLDM signature verification support (hash image data for signature validation)

#### R4: Performance and Resource Constraints
- **R4.1**: Memory-efficient operation suitable for embedded systems
- **R4.2**: Zero-copy data processing using Hubris leased memory
- **R4.3**: Deterministic resource allocation (no dynamic allocation)
- **R4.4**: Bounded execution time for real-time guarantees

#### R5: Hardware Abstraction
- **R5.1**: Generic interface supporting any hardware digest accelerator
- **R5.2**: Mock implementation for testing and development
- **R5.3**: Type-safe hardware abstraction with compile-time verification
- **R5.4**: Consistent API regardless of underlying hardware

#### R6: Error Handling and Reliability
- **R6.1**: Comprehensive error reporting for SPDM protocol diagnostics
- **R6.2**: Graceful handling of hardware failures
- **R6.3**: Session state validation and corruption detection
- **R6.4**: Clear error propagation to SPDM layer

#### R7: Integration Requirements
- **R7.1**: Synchronous IPC interface compatible with Hubris task model
- **R7.2**: Idol-generated API stubs for type-safe inter-process communication
- **R7.3**: Integration with Hubris memory management and scheduling
- **R7.4**: No dependency on async runtime or futures

#### R8: Supervisor Integration Requirements
- **R8.1**: Configure appropriate task disposition (Restart recommended for production)
- **R8.2**: SPDM clients handle task generation changes transparently (no complex recovery logic needed)
- **R8.3**: Digest server fails fast on unrecoverable hardware errors rather than returning complex error states
- **R8.4**: Support debugging via jefe external interface during development

## Implementation Overview

This digest server has been successfully converted to a generic implementation that can work with any device implementing the required digest traits from `openprot-hal-blocking`.

## Architecture

### System Context

```mermaid
graph LR
    subgraph "SPDM Client Task"
        SC[SPDM Client]
        SCV[• Certificate verification<br/>• Transcript hashing<br/>• Challenge-response<br/>• Key derivation]
    end
    
    subgraph "PLDM Client Task"
        PC[PLDM Firmware Update]
        PCV[• Image integrity validation<br/>• Component verification<br/>• Signature validation<br/>• Running digest computation]
    end
    
    subgraph "Digest Server"
        DS[ServerImpl&lt;D&gt;]
        DSV[• Session management<br/>• Generic implementation<br/>• Resource management<br/>• Error handling]
    end
    
    subgraph "Hardware Backend"
        HW[Hardware Device]
        HWV[• MockDigestDevice<br/>• Actual HW accelerator<br/>• Any device with traits]
    end
    
    SC ---|Synchronous<br/>IPC/Idol| DS
    PC ---|Synchronous<br/>IPC/Idol| DS
    DS ---|HAL Traits| HW
    
    SC -.-> SCV
    PC -.-> PCV
    DS -.-> DSV
    HW -.-> HWV
```

### Component Architecture

```
ServerImpl<D>
├── Generic Type Parameter D
│   └── Trait Bounds: DigestInit<Sha2_256/384/512>
├── Session Management
│   ├── Static session storage (MAX_SESSIONS = 8)
│   ├── Session lifecycle (init → update → finalize)
│   └── Automatic timeout and cleanup
└── Hardware Abstraction
    ├── Static dispatch (no runtime polymorphism)
    ├── Algorithm-specific methods
    └── Error translation layer
```

### Data Flow

```
SPDM Client Request
        ↓
   Idol-generated stub
        ↓
   ServerImpl<D> method
        ↓
   Session validation/allocation
        ↓
   Hardware abstraction call
        ↓
   Result processing
        ↓
   Response to client
```

## Detailed Design

### Session Model

#### Session Lifecycle
```
┌─────────┐    init_sha256/384/512()    ┌─────────┐
│  FREE   │ ────────────────────────→   │ ACTIVE  │
│         │                             │         │
└─────────┘                             └─────────┘
     ↑                                       │
     │ finalize_sha256/384/512()             │ update(data)
     │ reset()                               │ (accumulate)
     │ timeout_cleanup()                     │
     └───────────────────────────────────────┘
```

#### Session Data Structure
```rust
pub struct SessionData {
    algorithm: SessionAlgorithm,      // Algorithm type (Free/Sha256/Sha384/Sha512)
    buffer: [u8; SESSION_BUFFER_SIZE], // Accumulated data buffer (512 bytes)
    length: usize,                     // Current data length
    timeout: Option<u64>,              // Expiration timestamp
}
```

### Generic Hardware Abstraction

#### Trait Requirements
The server is generic over type `D` where:
```rust
D: DigestInit<Sha2_256, Output = Digest<8>> 
 + DigestInit<Sha2_384, Output = Digest<12>> 
 + DigestInit<Sha2_512, Output = Digest<16>>
```

#### Static Dispatch Pattern
- **Compile-time algorithm selection**: No runtime algorithm switching
- **Type safety**: Associated type constraints ensure output size compatibility
- **Zero-cost abstraction**: No virtual function calls or dynamic dispatch
- **Hardware flexibility**: Any device implementing the traits can be used

### Memory Management

#### Static Allocation Strategy
```rust
static mut SESSION_STORAGE: [SessionData; MAX_SESSIONS] = [...];
```
- **Deterministic memory usage**: No dynamic allocation
- **Stack overflow prevention**: Large session data not on stack
- **Real-time guarantees**: Bounded memory access patterns
- **Resource limits**: Fixed maximum number of concurrent sessions

#### Data Flow Optimization
- **Zero-copy IPC**: Uses Hubris leased memory system
- **Bounded updates**: Maximum 1024 bytes per update call
- **Incremental processing**: Large data processed in chunks
- **Memory safety**: All buffer accesses bounds-checked

### Error Handling Strategy

#### Layered Error Model
```
Hardware Layer Error → DigestError → RequestError<DigestError> → SPDM Client
```

#### Error Categories
- **Hardware failures**: `DigestError::HardwareFailure`
- **Session management**: `DigestError::InvalidSession`, `DigestError::TooManySessions`
- **Input validation**: `DigestError::InvalidInputLength`
- **Algorithm support**: `DigestError::UnsupportedAlgorithm`

### Concurrency Model

#### Session Isolation
- Each session operates independently
- No shared mutable state between sessions
- Session IDs provide access control
- Timeout mechanism prevents resource leaks

#### SPDM and PLDM Integration Points
1. **SPDM Certificate Verification**: Hash certificate chains incrementally
2. **SPDM Transcript Computation**: Hash sequences of SPDM messages
3. **SPDM Challenge Processing**: Compute authentication hashes
4. **SPDM Key Derivation**: Hash key exchange material
5. **PLDM Firmware Integrity**: Hash received firmware image chunks during transfer
6. **PLDM Component Validation**: Verify assembled components against manifest digests
7. **PLDM Multi-Component Updates**: Concurrent digest computation for multiple firmware components

## Failure Scenarios

### Session Management Failures

#### Session Exhaustion Scenario
```mermaid
sequenceDiagram
    participant S1 as SPDM Client 1
    participant S2 as SPDM Client 2
    participant DS as Digest Server
    participant HW as Hardware

    Note over DS: MAX_SESSIONS = 8, all sessions active
    
    S2->>DS: init_sha256()
    DS->>DS: find_free_session()
    DS-->>S2: Error: TooManySessions
    
    Note over S2: Client must wait or use one-shot operations
    S2->>DS: digest_oneshot_sha256(data)
    DS->>HW: compute hash directly
    HW-->>DS: result
    DS-->>S2: Success: hash result
```

#### Session Timeout Recovery
```mermaid
sequenceDiagram
    participant SC as SPDM Client
    participant DS as Digest Server
    participant T as Timer

    SC->>DS: init_sha256()
    DS-->>SC: session_id = 3
    
    Note over T: 10,000 ticks pass
    T->>DS: timer_tick
    DS->>DS: cleanup_expired_sessions()
    DS->>DS: session[3].timeout expired
    DS->>DS: session[3] = FREE
    
    SC->>DS: update(session_id=3, data)
    DS->>DS: validate_session(3)
    DS-->>SC: Error: InvalidSession
    
    Note over SC: Client must reinitialize
    SC->>DS: init_sha256()
    DS-->>SC: session_id = 3 (reused)
```

### Hardware Failure Scenarios

#### Hardware Device Failure
```mermaid
flowchart TD
    A[SPDM/PLDM Client Request] --> B[Digest Server]
    B --> C{Hardware Available?}
    
    C -->|Yes| D[Call hardware.init]
    C -->|No| E[panic! - Hardware unavailable]
    
    D --> F{Hardware Response}
    F -->|Success| G[Process normally]
    F -->|Error| H[panic! - Hardware failure]
    
    G --> I[Return result to client]
    E --> J[Task fault → Jefe supervision]
    H --> J
    
    style E fill:#ffcccc
    style H fill:#ffcccc
    style J fill:#fff2cc
```


### Resource Exhaustion Scenarios

#### Memory Pressure Handling
```mermaid
flowchart LR
    A[Large Data Update] --> B{Buffer Space Available?}
    
    B -->|Yes| C[Accept data into session buffer]
    B -->|No| D[Return InvalidInputLength]
    
    C --> E{Session Buffer Full?}
    E -->|No| F[Continue accepting updates]
    E -->|Yes| G[Client must finalize before more updates]
    
    D --> H[Client must use smaller chunks]
    G --> I[finalize_sha256/384/512]
    H --> J[Retry with smaller data]
    
    style D fill:#ffcccc
    style G fill:#fff2cc
    style H fill:#ccffcc
```

#### Session Lifecycle Error States
```mermaid
stateDiagram-v2
    [*] --> FREE
    FREE --> ACTIVE_SHA256: init_sha256()
    FREE --> ACTIVE_SHA384: init_sha384()
    FREE --> ACTIVE_SHA512: init_sha512()
    
    ACTIVE_SHA256 --> ACTIVE_SHA256: update(data)
    ACTIVE_SHA384 --> ACTIVE_SHA384: update(data)
    ACTIVE_SHA512 --> ACTIVE_SHA512: update(data)
    
    ACTIVE_SHA256 --> FREE: finalize_sha256()
    ACTIVE_SHA384 --> FREE: finalize_sha384()
    ACTIVE_SHA512 --> FREE: finalize_sha512()
    
    ACTIVE_SHA256 --> FREE: reset()
    ACTIVE_SHA384 --> FREE: reset()
    ACTIVE_SHA512 --> FREE: reset()
    
    ACTIVE_SHA256 --> FREE: timeout
    ACTIVE_SHA384 --> FREE: timeout
    ACTIVE_SHA512 --> FREE: timeout
    
    state ERROR_STATES {
        [*] --> InvalidSession: Wrong session ID
        [*] --> WrongAlgorithm: finalize_sha384() on SHA256 session
        [*] --> BufferOverflow: update() exceeds buffer
        [*] --> HardwareError: Hardware failure
    }
    
    ACTIVE_SHA256 --> ERROR_STATES: Error conditions
    ACTIVE_SHA384 --> ERROR_STATES: Error conditions
    ACTIVE_SHA512 --> ERROR_STATES: Error conditions
```

### SPDM Protocol Impact Analysis

#### Certificate Verification Failure Recovery
```mermaid
sequenceDiagram
    participant SPDM as SPDM Protocol
    participant DS as Digest Server
    participant POL as Security Policy

    SPDM->>DS: verify_certificate_chain()
    DS->>DS: init_sha256()
    DS-->>SPDM: Error: TooManySessions
    
    SPDM->>SPDM: Fallback strategy decision
    
    alt Retry with backoff
        Note over SPDM: Wait for sessions to free up
        SPDM->>DS: verify_certificate_chain() (retry)
        DS-->>SPDM: Success
    else Use one-shot operation
        SPDM->>DS: digest_oneshot_sha256()
        DS-->>SPDM: Success (if cert < 1024 bytes)
    else Fail authentication
        SPDM->>POL: Report authentication failure
        POL-->>SPDM: Security policy decision
    end
```

#### Transcript Hash Failure Impact
```mermaid
flowchart TD
    A[SPDM Message Exchange] --> B[Compute Transcript Hash]
    B --> C{Digest Server Available?}
    
    C -->|Yes| D[Normal transcript computation]
    C -->|No| E[Digest server failure]
    
    E --> F{Failure Type}
    F -->|Session Exhausted| G[Retry with backoff]
    F -->|Hardware Failure| H[Abort authentication]
    F -->|Timeout| I[Reinitialize session]
    
    G --> J{Retry Successful?}
    J -->|Yes| D
    J -->|No| K[Authentication failure]
    
    H --> K
    I --> L{Reinit Successful?}
    L -->|Yes| D
    L -->|No| K
    
    D --> M[Continue SPDM protocol]
    K --> N[Report to security policy]
    
    style E fill:#ffcccc
    style K fill:#ff9999
    style N fill:#ffcccc
```

### Failure Recovery Strategies

#### Error Propagation Chain
```mermaid
flowchart LR
    HW[Hardware Layer] -->|Any Error| PANIC[Task Panic]
    
    DS[Digest Server] -->|Recoverable DigestError| RE[RequestError wrapper]
    RE -->|IPC| CLIENTS[SPDM/PLDM Clients]
    CLIENTS -->|Simple Retry| POL[Security Policy]
    
    PANIC -->|Task Fault| JEFE[Jefe Supervisor]
    JEFE -->|Task Restart| DS_NEW[Fresh Digest Server]
    DS_NEW -->|Next IPC| CLIENTS
    
    subgraph "Recoverable Error Types"
        E1[InvalidSession]
        E2[TooManySessions]
        E3[InvalidInputLength]
    end
    
    subgraph "Simple Client Recovery"
        R1[Session Cleanup]
        R2[Retry with Backoff]
        R3[Use One-shot API]
        R4[Authentication Failure]
    end
    
    DS --> E1
    DS --> E2
    DS --> E3
    
    CLIENTS --> R1
    CLIENTS --> R2
    CLIENTS --> R3
    CLIENTS --> R4
    
    style PANIC fill:#ffcccc
    style DS_NEW fill:#ccffcc
```

#### System-Level Failure Handling
```mermaid
graph TB
    subgraph "Digest Server Internal Failures"
        F1[Session Exhaustion]
        F2[Recoverable Hardware Failure]
        F3[Input Validation Errors]
    end
    
    subgraph "Task-Level Failures"
        T1[Unrecoverable Hardware Failure]
        T2[Memory Corruption]
        T3[Syscall Faults]
        T4[Explicit Panics]
    end
    
    subgraph "SPDM Client Responses"
        S1[Retry with Backoff]
        S2[Fallback to One-shot]
        S3[Graceful Degradation]
        S4[Abort Authentication]
    end
    
    subgraph "Jefe Supervisor Actions"
        J1[Task Restart - Restart Disposition]
        J2[Hold for Debug - Hold Disposition]
        J3[Log Fault Information]
        J4[External Debug Interface]
    end
    
    subgraph "System-Level Responses"
        R1[Continue with Fresh Task Instance]
        R2[Debug Analysis Mode]
        R3[System Reboot - Jefe Fault]
    end
    
    F1 --> S1
    F2 --> S1
    F3 --> S4
    
    T1 --> J1
    T2 --> J1
    T3 --> J1
    T4 --> J1
    
    J1 --> R1
    J2 --> R2
    
    S1 --> R1
    S2 --> R1
    S3 --> R1
    
    R2 --> R3
    R1 --> R4
    R2 --> R4
```

## Supervisor Integration and System-Level Failure Handling

### Jefe Supervisor Role

The digest server operates under the supervision of Hubris OS's supervisor task ("jefe"), which provides system-level failure management beyond the server's internal error handling.

#### Supervisor Architecture
```mermaid
graph TB
    subgraph "Supervisor Domain (Priority 0)"
        JEFE[Jefe Supervisor Task]
        JEFE_FEATURES[• Fault notification handling<br/>• Task restart decisions<br/>• Debugging interface<br/>• System restart capability]
    end
    
    subgraph "Application Domain"
        DS[Digest Server]
        SPDM[SPDM Client]
        OTHER[Other Tasks]
    end
    
    KERNEL[Hubris Kernel] -->|Fault Notifications| JEFE
    JEFE -->|reinit_task| KERNEL
    JEFE -->|system_restart| KERNEL
    
    DS -.->|Task Fault| KERNEL
    SPDM -.->|Task Fault| KERNEL
    OTHER -.->|Task Fault| KERNEL
    
    JEFE -.-> JEFE_FEATURES
```

#### Task Disposition Management

Each task, including the digest server, has a configured disposition that determines jefe's response to failures:

- **Restart Disposition**: Automatic recovery via `kipc::reinit_task()`
- **Hold Disposition**: Task remains faulted for debugging inspection

#### Failure Escalation Hierarchy

```mermaid
sequenceDiagram
    participant HW as Hardware
    participant DS as Digest Server
    participant SPDM as SPDM Client
    participant K as Kernel
    participant JEFE as Jefe Supervisor

    Note over DS: Fail immediately on any hardware failure
    HW->>DS: Hardware fault
    DS->>DS: panic!("Hardware failure detected")
    DS->>K: Task fault occurs
    K->>JEFE: Fault notification (bit 0)
    
    JEFE->>K: find_faulted_task()
    K-->>JEFE: task_index (digest server)
    
    alt Restart disposition (production)
        JEFE->>K: reinit_task(digest_server, true)
        K->>DS: Task reinitialized with fresh hardware state
        Note over SPDM: Next IPC gets fresh task, no special handling needed
    else Hold disposition (debug)
        JEFE->>JEFE: Mark holding_fault = true
        Note over DS: Task remains faulted for debugging
        Note over SPDM: IPC returns generation mismatch error
    end
```

### System Failure Categories and Responses

#### Recoverable Failures (Handled by Digest Server)
- **Session Management**: `TooManySessions`, `InvalidSession` → Return error to client
- **Input Validation**: `InvalidInputLength` → Return error to client  

#### Task-Level Failures (Handled by Jefe)
- **Any Hardware Failure**: Hardware errors of any kind → Task panic → Jefe restart
- **Hardware Resource Exhaustion**: Hardware cannot allocate resources → Task panic → Jefe restart  
- **Memory Corruption**: Stack overflow, heap corruption → Task fault → Jefe restart
- **Syscall Faults**: Invalid kernel IPC usage → Task fault → Jefe restart
- **Explicit Panics**: `panic!()` in digest server code → Task fault → Jefe restart

#### System-Level Failures (Handled by Kernel)
- **Supervisor Fault**: Jefe task failure → System reboot
- **Kernel Panic**: Critical kernel failure → System reset
- **Watchdog Timeout**: System hang detection → Hardware reset

**Key Design Principle**: The digest server fails immediately on any hardware error without attempting recovery. This maximally simplifies the implementation and ensures consistent system behavior through jefe's supervision.

### External Debugging Interface

Jefe provides an external interface for debugging digest server failures:

```rust
// External control commands available via debugger (Humility)
enum JefeRequest {
    Hold,     // Stop automatic restart of digest server
    Start,    // Manually restart digest server  
    Release,  // Resume automatic restart behavior
    Fault,    // Force digest server to fault for testing
}
```

This enables development workflows like:
1. **Hold faulting server**: Examine failure state without automatic restart
2. **Analyze dump data**: Extract task memory and register state
3. **Test recovery**: Manually trigger restart after fixes
4. **Fault injection**: Test SPDM client resilience

### Integration Requirements Update

#### R8: Supervisor Integration Requirements
- **R8.1**: Configure appropriate task disposition (Restart recommended for production)
- **R8.2**: SPDM clients handle task generation changes transparently (no complex recovery logic needed)
- **R8.3**: Digest server fails fast on unrecoverable hardware errors rather than returning complex error states
- **R8.4**: Support debugging via jefe external interface during development

## SPDM Integration Examples

### Certificate Chain Verification (Requirement R3.1)
```rust
// SPDM task verifying a certificate chain
fn verify_certificate_chain(&mut self, cert_chain: &[u8]) -> Result<bool, SpdmError> {
    let digest = Digest::from(DIGEST_SERVER_TASK_ID);
    
    // Create session for certificate hash (R2.1: incremental computation)
    let session_id = digest.init_sha256()?;  // R1.1: SHA-256 support
    
    // Process certificate data incrementally (R4.2: zero-copy processing)
    for chunk in cert_chain.chunks(512) {
        digest.update(session_id, chunk.len() as u32, chunk)?;
    }
    
    // Get final certificate hash
    let mut cert_hash = [0u32; 8];
    digest.finalize_sha256(session_id, &mut cert_hash)?;
    
    // Verify against policy
    self.verify_hash_against_policy(&cert_hash)
}
```

### SPDM Transcript Hash Computation (Requirement R3.3)
```rust
// Computing hash of SPDM message sequence for authentication
fn compute_transcript_hash(&mut self, messages: &[SpdmMessage]) -> Result<[u32; 8], SpdmError> {
    let digest = Digest::from(DIGEST_SERVER_TASK_ID);
    let session_id = digest.init_sha256()?;  // R2.3: session isolation
    
    // Hash all messages in the SPDM transcript (R3.5: message authentication)
    for msg in messages {
        let msg_bytes = msg.serialize()?;
        digest.update(session_id, msg_bytes.len() as u32, &msg_bytes)?;
    }
    
    let mut transcript_hash = [0u32; 8];
    digest.finalize_sha256(session_id, &mut transcript_hash)?;  // R7.1: synchronous IPC
    Ok(transcript_hash)
}
```

### Concurrent SPDM Operations (Requirement R2.2)
```rust
// Multiple SPDM operations running simultaneously
impl SpdmResponder {
    fn handle_multiple_requests(&mut self) -> Result<(), SpdmError> {
        let digest = Digest::from(DIGEST_SERVER_TASK_ID);
        
        // Session 1: Certificate verification
        let cert_session = digest.init_sha256()?;
        
        // Session 2: Measurement hashing  
        let measure_session = digest.init_sha384()?;  // R1.2: SHA-384 support
        
        // Session 3: Key derivation
        let key_session = digest.init_sha512()?;      // R1.3: SHA-512 support
        
        // Process all three concurrently (up to 8 sessions total - R2.2)
        // Each session maintains independent state (R2.3: isolation)
        
        // ... process data in each session ...
        
        Ok(())
    }
}
```

## PLDM Integration Examples

### PLDM Firmware Image Integrity Validation (Requirement R3.6)
```rust
// PLDM task validating received firmware chunks
fn validate_firmware_image(&mut self, image_chunks: &[&[u8]], expected_digest: &[u32; 8]) -> Result<bool, PldmError> {
    let digest = Digest::from(DIGEST_SERVER_TASK_ID);
    
    // Create session for running digest computation (R2.1: incremental computation)
    let session_id = digest.init_sha256()?;  // R1.1: SHA-256 commonly used in PLDM
    
    // Process firmware image incrementally as chunks are received (R4.2: zero-copy processing)
    for chunk in image_chunks {
        digest.update(session_id, chunk.len() as u32, chunk)?;
    }
    
    // Get final image digest
    let mut computed_digest = [0u32; 8];
    digest.finalize_sha256(session_id, &mut computed_digest)?;
    
    // Compare with manifest digest
    Ok(computed_digest == *expected_digest)
}
```

### PLDM Component Verification During Transfer (Requirement R3.7)
```rust
// PLDM task computing running digest during TransferFirmware
fn transfer_firmware_with_validation(&mut self, component_id: u16) -> Result<(), PldmError> {
    let digest = Digest::from(DIGEST_SERVER_TASK_ID);
    
    // Initialize digest session for this component transfer (R2.3: session isolation)
    let session_id = digest.init_sha384()?;  // R1.2: SHA-384 for enhanced security
    
    // Store session for this component transfer
    self.component_sessions.insert(component_id, session_id);
    
    // Firmware chunks will be processed via update() calls as they arrive
    // This enables real-time validation during transfer rather than after
    
    Ok(())
}

fn process_firmware_chunk(&mut self, component_id: u16, chunk: &[u8]) -> Result<(), PldmError> {
    let digest = Digest::from(DIGEST_SERVER_TASK_ID);
    
    // Retrieve session for this component
    let session_id = self.component_sessions.get(&component_id)
        .ok_or(PldmError::InvalidComponent)?;
    
    // Add chunk to running digest (R3.6: firmware image integrity)
    digest.update(*session_id, chunk.len() as u32, chunk)?;
    
    Ok(())
}
```

### PLDM Multi-Component Concurrent Updates (Requirement R2.2)
```rust
// PLDM task handling multiple concurrent firmware updates
impl PldmFirmwareUpdate {
    fn handle_concurrent_updates(&mut self) -> Result<(), PldmError> {
        let digest = Digest::from(DIGEST_SERVER_TASK_ID);
        
        // Component 1: Main firmware using SHA-256
        let main_fw_session = digest.init_sha256()?;
        
        // Component 2: Boot loader using SHA-384  
        let bootloader_session = digest.init_sha384()?;  // R1.2: SHA-384 support
        
        // Component 3: FPGA bitstream using SHA-512
        let fpga_session = digest.init_sha512()?;        // R1.3: SHA-512 support
        
        // All components can be updated concurrently (up to 8 total - R2.2)
        // Each maintains independent digest state (R2.3: isolation)
        
        // Store sessions for component tracking
        self.component_sessions.insert(MAIN_FW_COMPONENT, main_fw_session);
        self.component_sessions.insert(BOOTLOADER_COMPONENT, bootloader_session);
        self.component_sessions.insert(FPGA_COMPONENT, fpga_session);
        
        Ok(())
    }
}
```

## Requirements Validation

### ✅ Requirements Satisfied

| Requirement | Status | Implementation |
|-------------|--------|----------------|
| **R1.1** SHA-256 support | ✅ | `init_sha256()`, `finalize_sha256()` |
| **R1.2** SHA-384 support | ✅ | `init_sha384()`, `finalize_sha384()` |  
| **R1.3** SHA-512 support | ✅ | `init_sha512()`, `finalize_sha512()` |
| **R1.4** Reject unsupported algorithms | ✅ | SHA-3 functions return `UnsupportedAlgorithm` |
| **R2.1** Incremental hash computation | ✅ | `update()` method for chunk processing |
| **R2.2** Multiple concurrent sessions | ✅ | `MAX_SESSIONS = 8` concurrent operations |
| **R2.3** Session isolation | ✅ | Independent session state and IDs |
| **R2.4** Automatic cleanup | ✅ | `cleanup_expired_sessions()` |
| **R2.5** Session timeout | ✅ | `SESSION_TIMEOUT_TICKS` mechanism |
| **R3.1-R3.5** SPDM use cases | ✅ | All supported via session-based API |
| **R3.6-R3.8** PLDM use cases | ✅ | Firmware validation, component verification, signature support |
| **R4.1** Memory efficient | ✅ | Static allocation, fixed buffers |
| **R4.2** Zero-copy processing | ✅ | Hubris leased memory system |
| **R4.3** Deterministic allocation | ✅ | No dynamic memory allocation |
| **R4.4** Bounded execution | ✅ | Fixed session limits, timeouts |
| **R5.1** Generic hardware interface | ✅ | `ServerImpl<D>` with trait bounds |
| **R5.2** Mock implementation | ✅ | `MockDigestDevice` available |
| **R5.3** Type-safe abstraction | ✅ | Associated type constraints |
| **R5.4** Consistent API | ✅ | Same interface regardless of hardware |
| **R6.1** Comprehensive errors | ✅ | Full `DigestError` enumeration |
| **R6.2** Hardware failure handling | ✅ | `HardwareFailure` error propagation |
| **R6.3** Session state validation | ✅ | `validate_session()` checks |
| **R6.4** Clear error propagation | ✅ | `RequestError<DigestError>` wrapper |
| **R7.1** Synchronous IPC | ✅ | No async/futures dependencies |
| **R7.2** Idol-generated stubs | ✅ | Type-safe IPC interface |
| **R7.3** Hubris integration | ✅ | Uses userlib, leased memory |
| **R7.4** No async runtime | ✅ | Pure synchronous implementation |
| **R8.1** Task disposition configuration | ✅ | Configured in app.toml |
| **R8.2** Transparent task generation handling | ✅ | SPDM clients get fresh task transparently |
| **R8.3** Fail-fast hardware error handling | ✅ | Task panic on unrecoverable hardware errors |
| **R8.4** Debugging support | ✅ | Jefe external interface available |

## Generic Design Summary

The `ServerImpl<D>` struct is now generic over any device `D` that implements:

## Key Features

1. **Hardware Agnostic**: Can work with any compatible digest hardware device
2. **Type Safety**: Associated type constraints ensure digest output sizes match expectations
3. **Zero Runtime Cost**: Uses static dispatch for optimal performance
4. **Memory Efficient**: Static session storage allocated at compile time

## Usage Example

To use with a custom hardware device:

```rust
// Your hardware device must implement the required traits
struct MyDigestDevice {
    // Your hardware-specific fields
}

impl DigestInit<Sha2_256> for MyDigestDevice {
    type Output = Digest<8>;
    // Implementation...
}

impl DigestInit<Sha2_384> for MyDigestDevice {
    type Output = Digest<12>;
    // Implementation...
}

impl DigestInit<Sha2_512> for MyDigestDevice {
    type Output = Digest<16>;
    // Implementation...
}

// Then use it with the server
let server = ServerImpl::new(MyDigestDevice::new());
```
