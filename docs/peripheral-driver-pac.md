# OpenPRot User Space Driver guide

A guide for writing sound, panic-free Rust peripheral drivers using PAC (Peripheral Access Crate) code generators.

## Table of Contents

1. [Overview](#overview)
2. [Architecture Layers](#architecture-layers)
3. [Safety Principles](#safety-principles)
4. [Register Access Patterns](#register-access-patterns)
5. [Error Handling](#error-handling)
6. [Interrupt Safety](#interrupt-safety)
7. [State Management](#state-management)
8. [Trait Implementations](#trait-implementations)
9. [Type System Patterns](#type-system-patterns)
10. [Testing](#testing)
11. [Common Pitfalls](#common-pitfalls)
12. [Code Review Checklist](#code-review-checklist)

---

## Overview

A PAC-based peripheral driver wraps a vendor-generated Peripheral Access Crate (e.g., `ast1060_pac`) to provide:

- **Type-safe** register access with compile-time field validation
- **Panic-free** operation for embedded/safety-critical targets
- **Interrupt-safe** state management without race conditions
- **Zero-allocation** (`no_std`) operation with fixed-capacity data structures
- **Portable** trait implementations (`embedded_hal`, `embedded_io`) for ecosystem reuse

### Example: UART Driver

The AST10x0 UART peripheral driver (`drivers/usart/` + `target/ast10x0/peripherals/uart/`) demonstrates:

```
┌─────────────────────────────────────────────┐
│  User Application                           │
│  (via embedded_io::Write/Read traits)       │
└────────────────┬────────────────────────────┘
                 │
┌────────────────▼────────────────────────────┐
│  Client Facade (client/lib.rs)              │
│  - Type-safe RPC envelope encoding          │
│  - Error conversion                         │
└────────────────┬────────────────────────────┘
                 │
┌────────────────▼────────────────────────────┐
│  Wire Protocol Definition (api/protocol.rs) │
│  - Request/Response headers (#[repr(C)])    │
│  - Operation opcodes (enum)                 │
│  - zerocopy traits for serialization        │
└────────────────┬────────────────────────────┘
                 │
┌────────────────▼────────────────────────────┐
│  Server Runtime (server/runtime.rs)         │
│  - Event loop + IRQ integration             │
│  - Dispatch to backend operations           │
│  - Error classification                     │
└────────────────┬────────────────────────────┘
                 │
┌────────────────▼────────────────────────────┐
│  Dispatch Logic (server/lib.rs)             │
│  - Protocol decoding (request → operation)  │
│  - Response encoding (Result → wire bytes)  │
│  - Queuing semantics for IRQ completion     │
└────────────────┬────────────────────────────┘
                 │
┌────────────────▼────────────────────────────┐
│  Backend Trait (api/BACKEND_TRAIT)          │
│  - Abstraction over platform instantiation  │
│  - Supports testing + multiple platforms    │
└────────────────┬────────────────────────────┘
                 │
┌────────────────▼────────────────────────────┐
│  Platform Driver (target/ast10x0/uart/)     │
│  - Register access (via ast1060_pac)        │
│  - Hardware-specific error handling         │
│  - Interrupt arms/flags                     │
└─────────────────────────────────────────────┘
```

---

## Architecture Layers

### 1. **Wire Protocol Definition** (`api/protocol.rs`)

Defines the serialization contract between client and server.

```rust
// Wire protocol must use #[repr(C, packed)] for C interoperability
#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub struct UsartRequestHeader {
    pub op_code: u8,
    pub flags: u8,
    pub arg0: u8,
    pub arg1: u8,
    pub payload_len: u16,
}

// zerocopy traits enable zero-copy serialization
#[derive(FromBytes, IntoBytes, Immutable)]
#[repr(C)]
pub struct UartConfig {
    pub parity: u8,
    pub stop_bits: u8,
}
```

**Key patterns:**
- Use `#[repr(C, packed)]` for byte-aligned transmission
- Derive `FromBytes`/`IntoBytes` from `zerocopy` crate for validation
- Never use Rust `bool` (platform-dependent size); use explicit enum or `u8`
- Versioning via header flags field predicts future protocol changes

### 2. **Backend Trait** (`api/BACKEND_TRAIT`)

Abstracts the actual peripheral implementation, enabling testing and multi-platform support.

```rust
pub trait UsartBackend {
    type Error: Into<UsartError> + Debug;

    fn configure(&mut self, parity: Parity, stop_bits: StopBits) 
        -> Result<(), Self::Error>;
    fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error>;
    fn read(&mut self, buf: &mut [u8]) -> nb::Result<usize, Self::Error>;
    fn try_read(&self) -> nb::Result<u8, Self::Error>;
    fn enable_interrupts(&self) -> Result<(), Self::Error>;
    fn disable_interrupts(&self) -> Result<(), Self::Error>;
}
```

**Design considerations:**
- All fallible operations return `Result` or `nb::Result`
- No panicking on error; always return concrete error types
- `enable_interrupts` returns `Result` because it can fail (e.g., IRQ arm timeout)
- `&self` vs `&mut self`: use `&self` for read-only ops that may busy-poll; `&mut self` for state mutations

### 3. **Server Dispatch** (`server/lib.rs`)

Routes incoming protocol requests to backend operations and encodes responses.

```rust
pub fn dispatch_request<B: UsartBackend>(
    backend: &mut B, 
    request: &UsartRequestHeader, 
    payload: &[u8]
) -> DispatchOutcome {
    match request.op_code {
        OP_CONFIGURE => {
            let config = UartConfig::read_from(payload)
                .ok_or(UsartError::InvalidPayload)?;
            match backend.configure(config.parity, config.stop_bits) {
                Ok(()) => DispatchOutcome::Respond { status: 0, len: 0 },
                Err(e) => {
                    let err: UsartError = e.into();
                    DispatchOutcome::Respond { status: err.status_code(), len: 0 }
                }
            }
        }
        OP_DRAIN => {
            // TX drain is IRQ-assisted; queue up for TX_IDLE completion
            match backend.enable_interrupts() {
                Ok(()) => DispatchOutcome::Queued,
                Err(e) => {
                    let err: UsartError = e.into();
                    DispatchOutcome::Respond { status: err.status_code(), len: 0 }
                }
            }
        }
        _ => DispatchOutcome::Respond { status: StatusCode::UnsupportedOp as u8, len: 0 }
    }
}
```

**Key patterns:**
- Never call `.unwrap()`, `.expect()`, or panic
- Use `Result::ok_or()` to convert `Option` to `Result`
- Map backend errors to wire status codes before responding
- Queue operations that require IRQ completion (return `DispatchOutcome::Queued`)

### 4. **Server Runtime** (`server/runtime.rs`)

Manages the event loop: channel wakeups drive dispatch, IRQ completion drives dequeuing.

```rust
fn main_loop<B: UsartBackend>(backend: &mut B) {
    let mut pending_queue: heapless::Deque<PendingRequest, MAX_PENDING> = heapless::Deque::new();
    
    loop {
        // Wait for channel wakeup or IRQ
        let wait_return = sys_wait(&[CHANNEL_WM, IRQ_NUMBER]);
        
        match wait_return.why {
            WaitWhy::Wokenup => {
                // Channel wakeup: dispatch new request
                let request = channel_receive();
                let outcome = dispatch_request(backend, &request.header, &request.payload);
                
                match outcome {
                    DispatchOutcome::Queued => {
                        if pending_queue.push_back(request).is_err() {
                            // Queue full; respond with BackendUnavailable
                            channel_send_response(UsartError::QueueFull);
                        }
                    }
                    DispatchOutcome::Respond { status, len } => {
                        channel_send_response_with_payload(status, &response_buffer[..len]);
                    }
                }
            }
            WaitWhy::Interrupted(irq_num) if irq_num == IRQ_NUMBER => {
                // IRQ completion: dequeue and encode response
                if let Some(pending_req) = pending_queue.pop_front() {
                    // Read TX_IDLE flag or data from backend
                    match backend.try_read() {
                        Ok(byte) => {
                            // Encode response
                            channel_send_response_with_payload(StatusCode::Success, &[byte]);
                        }
                        Err(nb::Error::WouldBlock) => {
                            // Transient; re-queue and re-arm
                            let _ = pending_queue.push_front(pending_req);
                            let _ = backend.enable_interrupts();
                        }
                        Err(nb::Error::Other(e)) => {
                            // Terminal error; respond immediately
                            let err: UsartError = e.into();
                            channel_send_response(err);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
```

**Key patterns:**
- Use fixed-capacity queues (`heapless::Deque<T, N>`) for pending requests
- Distinguish retryable (`WouldBlock`) from terminal errors
- Only re-arm interrupts if operation was retryable
- Respond with error status immediately on terminal failures (don't loop)

### 5. **Platform Driver** (`target/ast10x0/peripherals/uart/`)

Implements the backend trait using PAC register access.

```rust
use ast1060_pac as device;

pub struct Usart {
    usart: *const device::uart::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>, // !Sync marker
}

impl UsartBackend for Usart {
    type Error = UartError;
    
    fn configure(&mut self, parity: Parity, stop_bits: StopBits) 
        -> Result<(), Self::Error> {
        // Implementation details covered in Register Access Patterns
        Ok(())
    }
}
```

---

## Safety Principles

### 1. **No Panic Zones**

Critical rule: **Embedded firmware code must never panic.** Always return `Result` or `Option`, never unwrap.

```rust
// ❌ FORBIDDEN
let status = line_status_register.read().dr().bit_is_set();
let byte = self.regs().uartrbr().read().bits() as u8; // OK: infallible
let config = UartConfig::read_from(payload).unwrap(); // ❌ PANIC

// ✅ CORRECT
let config = UartConfig::read_from(payload)
    .ok_or(UsartError::InvalidPayload)?;
```

### 2. **Checked Arithmetic**

Integer overflow is a silent data corruption vector. Always use checked operations.

```rust
// ❌ FORBIDDEN
let divisor = clock_hz / baud_rate;  // Overflow if baud_rate is huge

// ✅ CORRECT
let divisor = clock_hz.checked_div(baud_rate)
    .ok_or(UartError::InvalidBaudRate)?;

// Saturating for FIFO level (can't exceed 255 bytes)
let level: u8 = rx_bytes.saturating_sub(FIFO_DEPTH);
```

### 3. **Pointer Safety**

PAC drivers require unsafe pointer dereferencing. Mitigate with:

- **Single construction point:** One `unsafe { new() }`
- **Encapsulation:** Private pointer, public safe methods only
- **Validity documentation:** Safety comment documents assumptions
- **Non-Sync marker:** Prevent concurrent access

```rust
pub struct Usart {
    usart: *const device::uart::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>, // Not thread-safe
}

impl Usart {
    /// Create a new USART instance from a raw register-block pointer.
    ///
    /// # Safety
    ///
    /// - `usart` must be a valid, non-null pointer to the AST1060 UART register block.
    /// - The pointed register block must remain valid for the lifetime of this `Usart`.
    /// - Caller must enforce global ownership so concurrent mutable access does not occur.
    pub unsafe fn new(usart: *const device::uart::RegisterBlock) -> Self {
        let this = Self {
            usart,
            _not_sync: PhantomData,
        };
        
        // Safe initialization
        this.regs().uartfcr().write(|w| w.enbl_uartfifo().set_bit());
        this
    }

    #[inline]
    fn regs(&self) -> &device::uart::RegisterBlock {
        // SAFETY: Construction is `unsafe`, so caller upholds validity.
        unsafe { &*self.usart }
    }
}
```

### 4. **Volatile Register Access**

Hardware registers might have side effects on read/write. Always use volatile ops:

```rust
// ✅ CORRECT - Use PAC's .read()/.write()/.modify()
// The PAC generates these with volatile semantics
self.regs().uartlsr().read();  // Volatile read
self.regs().uartthr().write(|w| w.bits(byte as u32));  // Volatile write

// For manual register access (if needed), use volatile crate:
use core::ptr::{read_volatile, write_volatile};
// read_volatile(addr as *const T);
```

### 5. **Error Information Leakage**

Never expose sensitive data in error messages:

```rust
// ❌ FORBIDDEN
pub fn authenticate(password: &[u8]) -> Result<Token, String> {
    if check_password(password)? {
        // ...
    } else {
        Err(format!("Invalid password: {:?}", password))  // Leaks secret!
    }
}

// ✅ CORRECT
pub fn authenticate(password: &[u8]) -> Result<Token, AuthError> {
    if check_password(password)? {
        // ...
    } else {
        Err(AuthError::InvalidCredentials)  // Generic error only
    }
}
```

---

## Register Access Patterns

### 1. **PAC-Generated Register Blocks**

The PAC (e.g., `ast1060_pac`) provides typed register access:

```rust
// From ast1060_pac::uart::RegisterBlock
pub struct RegisterBlock {
    pub uartrbr: RBR,      // Receiver Buffer Register (read-only)
    pub uartthr: THR,      // Transmitter Holding Register (write-only)
    pub uartdll: DLL,      // Divisor Latch Low (DLAB=1)
    pub uartier: IER,      // Interrupt Enable Register
    pub uartiir: IIR,      // Interrupt Identification Register (read-only)
    pub uartfcr: FCR,      // FIFO Control Register
    pub uartlcr: LCR,      // Line Control Register
    pub uartmcr: MCR,      // Modem Control Register
    pub uartlsr: LSR,      // Line Status Register (read-only)
    pub uartmsr: MSR,      // Modem Status Register (read-only)
    // ... more registers
}

// Each register is a proxy with typed accessors
pub struct LSR {
    register: VolatileCell<u32>,
}

impl LSR {
    pub fn read(&self) -> LsrR { /* ... */ }
    pub fn write<F>(&self, f: F) where F: FnOnce(&mut LsrW) { /* ... */ }
    pub fn modify<F>(&self, f: F) where F: FnOnce(&mut LsrR, &mut LsrW) { /* ... */ }
}
```

### 2. **Read Pattern**

Reading is often idempotent for status registers; reading clears interrupt flags:

```rust
// Read Line Status once per operation
let lsr = self.regs().uartlsr().read();

// Check each flag individually
if lsr.dr().bit() {  // Data Ready?
    let byte = self.regs().uartrbr().read().bits() as u8;
    // Process byte
}

if lsr.fe().bit_is_set() {  // Framing Error?
    return Err(UartError::FramingError);
}

// ❌ FORBIDDEN - Read status twice; races with hardware
let lsr1 = self.regs().uartlsr().read();
if lsr1.dr().bit() { /* data */ }
let lsr2 = self.regs().uartlsr().read();
if lsr2.fe().bit_is_set() { /* different bits! */ }
```

**Key lesson:** Read once, use the result multiple times.

### 3. **Write Pattern**

Writing registers typically requires bit field setup via closure:

```rust
// Simple field write
self.regs().uartier().write(|w| {
    w.erbfi().set_bit()   // Enable RX interrupt
        .etbei().set_bit()   // Enable TX interrupt
});

// Write with error checking (if supported by PAC)
self.regs().uartiir().write(|w| {
    w.intid().bits(0b001)  // Set interrupt type
});
```

### 4. **Modify Pattern**

For read-modify-write operations (changing one bit without affecting others):

```rust
// Change DLAB bit without affecting other LCR fields
self.regs().uartlcr().modify(|r, w| {
    // Read current value
    // Modify specific fields
    w.dlab().set_bit();
    // Other fields unchanged
});

// Disable TX interrupt only
self.regs().uartier().modify(|_, w| {
    w.etbei().clear_bit()
});
```

### 5. **Atomic Access for Shared Registers**

Some registers have read-only and write-only fields. Handle carefully:

```rust
// UART IIR (Interrupt Identification) is read-only
// To clear interrupts, we must read it, check the interrupt type, then handle it

let iir = self.regs().uartiir().read();
let int_type = InterruptDecoding::try_from(iir.intdecoding_table().bits() as u8)
    .unwrap_or(InterruptDecoding::Unknown);

match int_type {
    InterruptDecoding::RxDataAvailable => {
        // Read and drain RX FIFO
        while let Ok(byte) = self.try_read_byte() {
            // Process byte
        }
    }
    InterruptDecoding::TxEmpty => {
        // Handle TX completion
    }
    _ => {}
}
```

---

## Error Handling

### 1. **Error Enum Design**

Define a type-safe error hierarchy for your peripheral:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UartError {
    // Hardware faults
    FramingError,
    ParityError,
    OverrunError,
    
    // Configuration errors
    InvalidBaudRate,
    UnsupportedDataSize,
    
    // Protocol/API errors
    InvalidPayload,
    TimeoutExpired,
    
    // Resource exhaustion
    QueueFull,
    BufferOverflow,
}

impl UartError {
    pub fn status_code(&self) -> u8 {
        match self {
            UartError::FramingError => 1,
            UartError::ParityError => 2,
            UartError::OverrunError => 3,
            UartError::InvalidBaudRate => 10,
            UartError::UnsupportedDataSize => 11,
            UartError::InvalidPayload => 20,
            UartError::TimeoutExpired => 21,
            UartError::QueueFull => 30,
            UartError::BufferOverflow => 31,
        }
    }
}

// Implement Into for trait objects
impl From<UartError> for UsartError {
    fn from(e: UartError) -> Self {
        UsartError::Hardware(e.status_code())
    }
}
```

### 2. **Result Type Aliases**

Simplify signatures:

```rust
pub type UartResult<T> = Result<T, UartError>;

impl Usart {
    pub fn configure(&mut self, baud: u32) -> UartResult<()> {
        let divisor = baud.checked_div(16)
            .ok_or(UartError::InvalidBaudRate)?;
        
        self.regs().uartdll().write(|w| unsafe { w.bits(divisor as u32) });
        Ok(())
    }
}
```

### 3. **Non-Blocking Error Handling**

Use `nb::Result` for operations that might retry:

```rust
use nb;

pub fn try_read_byte(&self) -> nb::Result<u8, UartError> {
    let lsr = self.regs().uartlsr().read();
    
    if !lsr.dr().bit() {
        // No data available; caller should retry or arm interrupt
        return Err(nb::Error::WouldBlock);
    }
    
    let byte = self.regs().uartrbr().read().bits() as u8;
    
    if lsr.fe().bit_is_set() {
        Err(nb::Error::Other(UartError::FramingError))
    } else if lsr.pe().bit_is_set() {
        Err(nb::Error::Other(UartError::ParityError))
    } else {
        Ok(byte)
    }
}
```

### 4. **Error Classification**

Distinguish retryable from terminal errors in interrupt contexts:

```rust
fn handle_pending_read(backend: &mut B) {
    match backend.try_read() {
        Ok(byte) => {
            // Success; respond
            send_response(StatusCode::Success, &[byte]);
        }
        Err(nb::Error::WouldBlock) => {
            // No data (yet); re-arm interrupt and re-queue
            let _ = backend.enable_interrupts();
            queue.push_back(pending_read);
        }
        Err(nb::Error::Other(e)) => {
            // Terminal error (framing, parity, etc.); respond now
            let err_code = e.status_code();
            send_response(err_code, &[]);
            // Cancel operation, do NOT re-queue
        }
    }
}
```

---

## Interrupt Safety

### 1. **IRQ-Assisted Completion**

For operations requiring hardware events (RX data, TX done), use IRQ to wake the runtime:

```
Client Thread                Runtime Thread
    │                           │
    │ send(TryRead) ───────────▶│
    │                           │
    │                    dispatch() → Queued
    │                    sys_wait()  ◄─ blocks on channel + IRQ
    │                           │
                           [hardware event occurs]
                                │
                           IRQ fires ─────────────▶ sys_wait() wakes
                                │
                           try_read() ───────────▶ Yes! Data available
                                │
                           send(Response) ◄─────── Responds with data
    │◄──────── receive() ◄────────────────────────┤
    │ complete                                    │
```

**Pattern:**

```rust
// Dispatch "Drain" operation
pub fn dispatch_request<B: UsartBackend>(backend: &mut B, req: Request) 
    -> DispatchOutcome {
    match req.op_code {
        OP_DRAIN => {
            // Operation requires TX_IDLE interrupt
            match backend.enable_interrupts() {
                Ok(()) => DispatchOutcome::Queued,  // Wait for IRQ
                Err(e) => DispatchOutcome::Respond { 
                    status: e.status_code(), 
                    len: 0 
                }
            }
        }
        _ => { /* ... */ }
    }
}

// Runtime's IRQ handler
fn handle_irq(backend: &mut B, pending_queue: &mut Queue<Request>) {
    if let Some(pending) = pending_queue.pop_front() {
        match pending.op_code {
            OP_DRAIN => {
                // Check if TX is now idle
                if backend.is_tx_idle() {
                    channel_send(Response { status: StatusCode::Ok, len: 0 });
                } else {
                    // Still busy; re-queue and re-arm
                    pending_queue.push_front(pending);
                    backend.enable_interrupts();
                }
            }
            _ => { /* ... */ }
        }
    }
}
```

### 2. **Interrupt Arming Must Succeed or Fail Atomically**

Never leave a request queued if interrupt-arm fails:

```rust
// ❌ FORBIDDEN - Race condition
queue.push_back(request);  // Queued...
backend.enable_interrupts()?;  // ...but arm fails; request lost forever!

// ✅ CORRECT - Atomic check
match backend.enable_interrupts() {
    Ok(()) => {
        queue.push_back(request);  // Safe to queue
    }
    Err(e) => {
        // Respond immediately; never queue
        send_response(e.status_code(), &[]);
    }
}
```

### 3. **Interrupt Clear vs Disable**

Understand your hardware's interrupt model:

```rust
// AST1060 UART: IER (Interrupt Enable Register) controls interrupt generation
// IIR (Interrupt Identification) tells you what fired

// Enable RX Data Available interrupt
self.regs().uartier().modify(|_, w| w.erbfi().set_bit());

// Disable RX Data Available interrupt
self.regs().uartier().modify(|_, w| w.erbfi().clear_bit());

// Check what interrupt fired
let iir = self.regs().uartiir().read();
if let Ok(int_type) = InterruptDecoding::try_from(iir.intdecoding_table().bits() as u8) {
    match int_type {
        InterruptDecoding::RxDataAvailable => { /* handle */ }
        InterruptDecoding::TxEmpty => { /* handle */ }
        _ => {}
    }
}
```

### 4. **Data Races in Shared State**

If the same peripheral is accessed from both application thread and IRQ context:

```rust
// ❌ FORBIDDEN - Concurrent access without synchronization
pub static mut UART: Option<Usart> = None;

fn app_task() {
    unsafe {
        if let Some(ref mut uart) = UART {
            uart.write(&[65, 66, 67]);  // May race with IRQ
        }
    }
}

irq_handler! {
    unsafe {
        if let Some(ref mut uart) = UART {
            // ❌ Concurrent mutable reference to same peripheral
        }
    }
}

// ✅ CORRECT - Use a mutex or split concerns
use core::sync::atomic::{AtomicU16, Ordering};

pub struct UartState {
    tx_queue: heapless::Vec<u8, 256>,
    rx_queue: heapless::Deque<u8, 256>,
}

pub static UART_STATE: Mutex<UartState> = Mutex::new(...);

pub fn critical_section_write(data: &[u8]) {
    critical_section::with(|cs| {
        let mut state = UART_STATE.borrow_mut(cs);
        state.tx_queue.extend_from_slice(data);
    });
}
```

---

## State Management

### 1. **Encapsulated Register State**

Don't expose raw pointers; encapsulate all state inside the driver:

```rust
// ✅ CORRECT
pub struct Usart {
    usart: *const device::uart::RegisterBlock,  // Private
    _not_sync: PhantomData<UnsafeCell<()>>,
}

impl Usart {
    pub fn is_tx_idle(&self) -> bool {
        self.regs().uartlsr().read().txter_empty().bit_is_set()
    }
}

// ❌ FORBIDDEN
pub struct Usart {
    pub usart: *const device::uart::RegisterBlock,  // Exposed!
}
```

### 2. **Transient vs Persistent State**

Classify which state persists across operations:

```rust
pub struct Usart {
    usart: *const device::uart::RegisterBlock,  // Persistent (registers)
    _not_sync: PhantomData<UnsafeCell<()>>,
}

pub struct UartConfig {
    baud_rate: u32,      // Transient (computed per operation)
    parity: Parity,      // Transient (queried per operation)
}

impl Usart {
    // No need to cache; register contains truth
    pub fn is_tx_full(&self) -> bool {
        !self.regs().uartlsr().read().thre().bit()
    }
    
    // Caching transient state is wrong; query registers instead
    // ❌ FORBIDDEN
    pub fn get_cached_config(&self) -> UartConfig {
        self.cached_config  // Stale!
    }
}
```

### 3. **Initialization and Cleanup**

Initialize hardware completely in one step; avoid partial-init states:

```rust
impl Usart {
    /// Create a new USART instance from a raw register-block pointer.
    ///
    /// Configures RX/TX FIFO, 8 byte RX trigger level, 1.5 MBaud, 8n1.
    pub unsafe fn new(usart: *const device::uart::RegisterBlock) -> Self {
        let this = Self {
            usart,
            _not_sync: PhantomData,
        };

        // Initialize all critical registers in one go
        unsafe {
            this.regs().uartfcr().write(|w| {
                w.enbl_uartfifo().set_bit();
                w.rx_fiforst().set_bit();
                w.tx_fiforst().set_bit();
                w.define_the_rxr_fifointtrigger_level().bits(0b10)
            });
        }

        // Chain configuration calls for fluency
        this
            .set_rate(Rate::MBaud1_5)
            .set_8n1()
            .interrupt_enable()
    }
}

impl Drop for Usart {
    fn drop(&mut self) {
        // Disable interrupts before drop
        self.regs().uartier().write(|w| w.bits(0));
    }
}
```

---

## Trait Implementations

### 1. **embedded_io Traits**

Implement standard read/write interfaces for ecosystem compatibility:

```rust
use embedded_io::{Read, Write};

impl embedded_io::ErrorType for Usart {
    type Error = UartError;
}

impl embedded_io::Error for UartError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

impl Write for Usart {
    fn write(&mut self, buf: &[u8]) -> Result<usize, UartError> {
        for (n, byte) in buf.iter().enumerate() {
            if !self.is_tx_full() {
                self.regs().uartthr().write(|w| unsafe { w.bits(*byte as u32) });
            } else {
                if n == 0 {
                    // UART spec: must write at least one byte (block until FIFO ready)
                    continue;
                }
                return Ok(n);  // Partial write
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), UartError> {
        while !self.is_tx_idle() {}
        Ok(())
    }
}

impl Read for Usart {
    fn read(&mut self, out: &mut [u8]) -> Result<usize, UartError> {
        if out.is_empty() {
            return Ok(0);
        }

        let mut count = 0;
        
        // Block until at least one byte is available
        while count == 0 {
            match self.try_read_byte() {
                Ok(byte) => {
                    out[count] = byte;
                    count += 1;
                }
                Err(nb::Error::WouldBlock) => continue,
                Err(nb::Error::Other(e)) => return Err(e),
            }
        }

        // Drain what's immediately available
        while count < out.len() {
            match self.try_read_byte() {
                Ok(byte) => {
                    out[count] = byte;
                    count += 1;
                }
                Err(nb::Error::WouldBlock) => break,
                Err(nb::Error::Other(e)) => return Err(e),
            }
        }

        Ok(count)
    }
}
```

### 2. **embedded_hal_nb Traits**

Implement non-blocking serial traits:

```rust
use embedded_hal_nb::serial as serial_nb;

impl serial_nb::ErrorType for Usart {
    type Error = UartError;
}

impl serial_nb::Error for UartError {
    fn kind(&self) -> serial_nb::ErrorKind {
        match self {
            UartError::FramingError => serial_nb::ErrorKind::FrameFormat,
            UartError::ParityError => serial_nb::ErrorKind::Parity,
            UartError::OverrunError => serial_nb::ErrorKind::Overrun,
            _ => serial_nb::ErrorKind::Other,
        }
    }
}

impl serial_nb::Write<u8> for Usart {
    fn write(&mut self, word: u8) -> nb::Result<(), UartError> {
        if !self.is_tx_full() {
            self.regs().uartthr().write(|w| unsafe { w.bits(word as u32) });
            Ok(())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }

    fn flush(&mut self) -> nb::Result<(), UartError> {
        if self.is_tx_idle() {
            Ok(())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }
}

impl serial_nb::Read<u8> for Usart {
    fn read(&mut self) -> nb::Result<u8, UartError> {
        self.try_read_byte()
    }
}
```

---

## Type System Patterns

### 1. **Type-State Pattern for Configuration**

Enforce configuration order at compile time:

```rust
// Builder pattern with type safety
pub struct Unconfigured;
pub struct Configured;

pub struct Usart<State = Unconfigured> {
    usart: *const device::uart::RegisterBlock,
    _state: PhantomData<State>,
    _not_sync: PhantomData<UnsafeCell<()>>,
}

impl Usart<Unconfigured> {
    pub unsafe fn new(usart: *const device::uart::RegisterBlock) -> Self {
        Self {
            usart,
            _state: PhantomData,
            _not_sync: PhantomData,
        }
    }
    
    pub fn configure(mut self, baud: u32) -> Result<Usart<Configured>, ConfigError> {
        self.set_rate(baud)?;
        Ok(Usart {
            usart: self.usart,
            _state: PhantomData,  // Changed to Configured
            _not_sync: PhantomData,
        })
    }
}

impl Usart<Configured> {
    pub fn write(&mut self, buf: &[u8]) -> Result<usize, UartError> {
        // Only callable after configuration
    }
}

// Usage: compile error if you try to write without configure()
let mut uart = unsafe { Usart::new(UART_BASE) };
uart.write(&[65])?;  // ❌ Compile error: no Write impl for Usart<Unconfigured>
uart.configure(115200)?;
uart.write(&[65])?;  // ✅ Works: Usart<Configured>
```

### 2. **Phantom Types for Compile-Time Constraints**

```rust
use core::marker::PhantomData;

// Ensure type is not Send + Sync (prevents accidental use in concurrent contexts)
pub struct Usart {
    usart: *const device::uart::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>,
}

// Verify at compile time
#[test]
fn uart_is_not_send() {
    fn assert_send<T: Send>() {}
    // assert_send::<Usart>();  // ❌ Compile error; good!
}
```

### 3. **Generic Over Backend for Testing**

```rust
pub trait UsartBackend {
    type Error: Into<UartError>;
    fn configure(&mut self, baud: u32) -> Result<(), Self::Error>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error>;
}

pub struct MockUsart {
    tx_buffer: heapless::Vec<u8, 256>,
    rx_buffer: heapless::Deque<u8, 256>,
}

impl UsartBackend for MockUsart {
    type Error = MockError;
    
    fn configure(&mut self, baud: u32) -> Result<(), MockError> {
        Ok(())  // No-op in mock
    }
    
    fn write(&mut self, buf: &[u8]) -> Result<usize, MockError> {
        self.tx_buffer.extend_from_slice(buf).ok();
        Ok(buf.len())
    }
}

pub fn dispatch_request<B: UsartBackend>(
    backend: &mut B,
    request: Request,
) -> Result<Response, B::Error> {
    // Works with both real and mock backends
    match request.op_code {
        OP_WRITE => backend.write(&request.payload),
        _ => Ok(0),
    }
}

#[test]
fn test_dispatch_with_mock() {
    let mut backend = MockUsart::default();
    let request = Request { op_code: OP_WRITE, payload: &[65, 66, 67] };
    let result = dispatch_request(&mut backend, request);
    assert!(result.is_ok());
}
```

---

## Testing

### 1. **Unit Tests with Mock Objects**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    struct MockUsart {
        write_called: bool,
        written_byte: u8,
    }
    
    impl MockUsart {
        fn new() -> Self {
            Self {
                write_called: false,
                written_byte: 0,
            }
        }
    }
    
    #[test]
    fn test_configure_invalid_baud() {
        // Can't use real hardware in tests; spawn mock instead
        let mut uart = MockUsart::new();
        let result = dispatch_request(&mut uart, Request {
            op_code: OP_CONFIGURE,
            payload: &[0, 0, 0, 0]  // Huge baud rate
        });
        assert!(result.is_err());
    }
    
    #[test]
    fn test_write_to_full_fifo() {
        let mut uart = MockUsart::new();
        // Simulate FIFO full by injecting mock behavior
        let result = dispatch_request(&mut uart, Request {
            op_code: OP_WRITE,
            payload: &[65]
        });
        assert_eq!(result.unwrap(), 1);
    }
}
```

### 2. **Hardware Integration Tests**

Real hardware tests use the actual driver:

```bash
# Run with specific UART hardware available
$ bazel test //target/ast10x0/tests/usart:usart_test --config=virt_ast10x0

# Result summary:
# PASSED: test_write_and_read_echo
# PASSED: test_interrupt_enable_disable
# FAILED: test_tx_drain_timeout (timeout after 5s)
```

### 3. **Error Path Coverage**

Always test error conditions:

```rust
#[test]
fn test_frame_error_handling() {
    let lsr = LineStatus::FramingError;
    assert_eq!(try_read_with_status(lsr), Err(UartError::FramingError));
}

#[test]
fn test_queue_full_response() {
    let mut queue = heapless::Deque::<Request, 1>::new();
    queue.push_back(Request { /* ... */ }).ok();
    queue.push_back(Request { /* ... */ }).ok();  // Fails
    
    // Verify response code is QueueFull, not panic
    assert_eq!(queue.push_back(Request { /* ... */ }), Err(()));
}
```

---

## Common Pitfalls

### Pitfall 1: **Assuming Data is Always Available**

```rust
// ❌ WRONG
pub fn read_byte(&self) -> u8 {
    self.regs().uartrbr().read().bits() as u8  // May read garbage if !DR
}

// ✅ CORRECT
pub fn try_read_byte(&self) -> nb::Result<u8, UartError> {
    let lsr = self.regs().uartlsr().read();
    if !lsr.dr().bit() {
        return Err(nb::Error::WouldBlock);
    }
    Ok(self.regs().uartrbr().read().bits() as u8)
}
```

### Pitfall 2: **Reading Status Twice**

```rust
// ❌ WRONG - TOCTOU race
let lsr1 = self.regs().uartlsr().read();
if !lsr1.dr().bit() {
    return Err(...);
}
let lsr2 = self.regs().uartlsr().read();  // May have different flags!
if lsr2.fe().bit_is_set() { /* ? */ }

// ✅ CORRECT - Single read
let lsr = self.regs().uartlsr().read();
if !lsr.dr().bit() {
    return Err(...);
}
if lsr.fe().bit_is_set() {  // Use same snapshot
    return Err(...);
}
```

### Pitfall 3: **Not Distinguishing Retryable from Terminal Errors**

```rust
// ❌ WRONG - Loops forever on terminal errors
match backend.try_read() {
    Ok(byte) => { /* success */ }
    Err(e) => {
        // Could be WouldBlock (retryable) or FramingError (not)
        queue.push_front(request);
        backend.enable_interrupts();  // Re-arm for both cases ❌
    }
}

// ✅ CORRECT - Check error type
match backend.try_read() {
    Ok(byte) => { /* success */ }
    Err(nb::Error::WouldBlock) => {
        queue.push_front(request);  // Re-queue
        backend.enable_interrupts();  // Re-arm
    }
    Err(nb::Error::Other(e)) => {
        // Terminal error; respond immediately, don't re-queue
        send_response(e.status_code(), &[]);
    }
}
```

### Pitfall 4: **Volatile Semantics on Reads**

```rust
// ❌ WRONG - Compiler optimizes away reads
loop {
    if self.regs().uartlsr().read().dr().bit() {
        break;  // ✅ Compiler sees .read() and won't optimize
    }
}

// ✅ STILL CORRECT - read() is volatile internally
// PAC ensures volatility; don't use manual loops
```

### Pitfall 5: **Integer Overflow on Divisor**

```rust
// ❌ WRONG
let divisor = 24_000_000 / baud_rate;  // Overflows if baud_rate < 0.5?

// ✅ CORRECT
let divisor = 24_000_000
    .checked_div(baud_rate)
    .ok_or(UartError::InvalidBaudRate)?;
```

### Pitfall 6: **Forgetting Volatile on Writes**

```rust
// ❌ WRONG - Non-volatile write (compiler could optimize away)
// (Never use this; use PAC accessors instead)
*(addr as *mut u32) = value;

// ✅ CORRECT - Use PAC's write() which is volatile
self.regs().uartthr().write(|w| w.bits(value));
```

---

## Code Review Checklist

When reviewing peripheral driver code, verify:

### Safety & Correctness

- [ ] No `.unwrap()`, `.expect()`, or `panic!()` calls
- [ ] All fallible operations return `Result` or `Option`
- [ ] Integer arithmetic uses `checked_*` or `saturating_*` where applicable
- [ ] No direct array indexing; use `get()` or pattern matching
- [ ] `unsafe` blocks have safety comments explaining why they're safe
- [ ] `unsafe` functions document their safety contracts clearly
- [ ] Register reads are performed once and result reused (no TOCTOU)

### Register Access

- [ ] All register access uses PAC-provided accessors (`.read()`, `.write()`, `.modify()`)
- [ ] No manual pointer dereferences except in unsafe blocks with safety comments
- [ ] Volatile semantics preserved (PAC handles this automatically)
- [ ] Register fields are extracted with typed accessors (`.bits()`, `.bit()`, etc.)

### Error Handling

- [ ] Custom error types are defined (enum with descriptive variants)
- [ ] Errors are mapped to wire status codes (for protocol serialization)
- [ ] Retryable errors (`WouldBlock`) are distinguished from terminal errors
- [ ] Error information doesn't leak sensitive data
- [ ] IRQ context errors are classified correctly (retryable vs terminal)

### Interrupt Safety

- [ ] Interrupt-arm operations return `Result` and are checked before queuing
- [ ] No race between queuing and interrupt-arm; atomic semantics enforced
- [ ] IRQ handlers use only `&self` or bounded mutable references
- [ ] Shared state between IRQ and task context is protected (mutex, atomic, etc.)
- [ ] IRQ handlers don't call blocking operations

### State Management

- [ ] Driver encapsulates all register access (private fields)
- [ ] No exposed raw pointers or `UnsafeCell` to external callers
- [ ] Type state or builder pattern enforces initialization order if needed
- [ ] No caching of transient register values; query registers instead

### Testing

- [ ] Happy path covered with unit tests using mock objects
- [ ] Error conditions tested (not just success cases)
- [ ] Mock backend implements the backend trait for testing
- [ ] Integration tests run on real hardware (or QEMU simulation)
- [ ] Test names describe what they verify, not just "test_foo"

### Documentation

- [ ] Safety comments on `unsafe` blocks explain the safety assumptions
- [ ] Function-level doc comments explain pre/post conditions
- [ ] `Safety` section in `unsafe fn` docs lists all caller obligations
- [ ] Register access patterns documented (why read once, etc.)
- [ ] Error enum variants documented with causes

### No_std Compliance

- [ ] No `std` crate imports (only `core`)
- [ ] No heap allocation (no `Vec`, `HashMap`, `String`, `Box`)
- [ ] Fixed-capacity collections used instead (`heapless`, arrays, `heapless::Deque`)
- [ ] No FFI without `unsafe` and safety comments

---

## Best Practices Summary

| Practice | Benefit | Example |
|----------|---------|---------|
| **One-time initialization** | Prevents partial-state bugs | `unsafe fn new()` performs all setup atomically |
| **Error classification** | Enables correct recovery | `WouldBlock` vs `Other(Error)` in IRQ context |
| **Single register reads** | Avoids TOCTOU bugs | `let lsr = regs.uartlsr().read(); if lsr.dr() { ... }` |
| **Phantom types** | Static safety guarantees | `PhantomData<UnsafeCell<()>>` prevents concurrent access |
| **Builder pattern** | Type-safe configuration | `Usart::new().configure(...).enable_interrupts()` |
| **Test with mocks** | Fast feedback without hardware | `MockUsart` impl `UsartBackend` |
| **Result everywhere** | Panic-free in interrupt contexts | `fn set_rate(rate: u32) -> Result<...>` |
| **Split concerns** | Clear responsibility boundaries | Protocol ≠ dispatch ≠ runtime ≠ backend |

---

## References

- [rust-embedded/embedded-hal](https://github.com/rust-embedded/embedded-hal) — Standard traits
- [AST1060 UART Register Manual](https://www.aspeedtech.com/nc/en/uSite_docs/files/ast1060_uart_guide.pdf) — Hardware specification
- [Rust Book § 19.1: Unsafe Code](https://doc.rust-lang.org/book/ch19_01_unsafe_rust.html)
- [Rustonomicon § Concurrency](https://doc.rust-lang.org/nomicon/concurrency.html)
- [heapless](https://docs.rs/heapless/) — Fixed-capacity collections
- [zerocopy](https://docs.rs/zerocopy/) — Safe zero-copy serialization
