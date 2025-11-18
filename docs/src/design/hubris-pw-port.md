# Porting Hubris Applications to Pigweed Kernel

## Executive Summary

This guide provides a practical roadmap for porting existing Oxide Hubris applications to Pigweed Kernel, focusing on **translating existing Hubris code** to work with Pigweed's current capabilities as-is, without waiting for new tooling or infrastructure.

**Key Challenge:** Hubris applications rely heavily on Idol IDL code generation, while Pigweed kernel uses manual syscall APIs. This guide shows how to bridge that gap.

**Target Audience:** Developers with existing Hubris applications who want to evaluate or migrate to Pigweed kernel.

**Pigweed Kernel Resources:**
- Documentation: `pw_kernel/docs.rst`
- Quickstart: `pw_kernel/quickstart.rst`
- API Reference: `pw_kernel/userspace/syscall.rs` (syscall API)
- Channel Implementation: `pw_kernel/kernel/object/channel.rs`
- System Configuration: `pw_kernel/kernel/system_config.rs`

---

## Table of Contents

1. [Understanding the Gap](#understanding-the-gap)
   - [Terminology Mapping](#terminology-mapping)
2. [Component-by-Component Mapping](#component-by-component-mapping)
3. [Idol to Manual IPC Translation](#idol-to-manual-ipc-translation)
4. [Configuration Translation](#configuration-translation)
5. [Build System Integration](#build-system-integration)
6. [Complete Porting Example](#complete-porting-example)
7. [Testing and Validation](#testing-and-validation)
8. [Known Limitations](#known-limitations)
9. [Decision Matrix](#decision-matrix)
10. [Migration Strategies](#migration-strategies)

---

## Understanding the Gap

### What You Have (Hubris)

```
app.toml                    → System configuration
idl/*.idol                  → Service interface definitions (repository root)
drv/*/src/main.rs           → Driver task implementations (servers)
task/*/src/main.rs          → Application task implementations (clients)
build.rs                    → Idol code generation in driver/API crates
Cargo.toml                  → Dependencies, features
```

### What You Need (Pigweed)

```
system.json5                → System configuration
processes/*/src/main.rs     → Process implementation (manual syscalls)
BUILD.bazel                 → Build configuration
No IDL files                → Manual protocol implementation
No code generation          → Hand-written client/server code
```

### Terminology Mapping

**Hubris** and **Pigweed** use different terms for the same concepts:

| Concept | Hubris Term | Pigweed Term | Description |
|---------|-------------|--------------|-------------|
| **Execution Unit** | Task | Process | Isolated unit of execution with MPU protection |
| **Service Provider** | Server | Handler | Receives and processes IPC requests |
| **Service Consumer** | Client | Initiator | Sends IPC requests to services |
| **IPC Operation** | Call (via Idol stub) | Transaction (via channel_transact) | Synchronous request-response |
| **IPC Endpoint** | Task ID | Channel ID | Identifies communication endpoint |

**Throughout this document:**
- When discussing Hubris code: "task", "client", "server"
- When discussing Pigweed code: "process", "initiator", "handler"
- These terms are **functionally equivalent** in their respective systems

### Critical Differences

| Aspect | Hubris | Pigweed Kernel |
|--------|--------|----------------|
| **Task/Process Isolation** | MPU regions (ARMv7-M) | MPU regions (ARMv8-M only) |
| **IPC Mechanism** | Idol-generated stubs over kernel IPC via task-slots | Manual channel syscalls |
| **Service Definitions** | .idol files → generated Rust code | No IDL, hand-written protocols |
| **Error Handling** | Idol error types (auto-generated) | Manual Result<> types |
| **Shared Memory** | Leases (temporary IPC memory grants) | Static memory regions in system.json5 |
| **Task Communication** | Task-slots (compile-time IPC handles) | Channel IDs (runtime configuration) |
| **Notifications** | Async notification bits (timers, IRQs) | No built-in async notification system |
| **Peripheral Access** | `uses = ["periph"]` in app.toml (MPU-enforced) | `device_memory[]` in system.json5 (MPU-enforced) |
| **Supervisor/Recovery** | task-jefe supervisor (standard, priority 0) | No built-in supervisor (DIY) |
| **Build System** | Cargo with xtask | Bazel |
| **Configuration Format** | app.toml | system.json5 |

---

---

## Component-by-Component Mapping

### 1. Configuration Files

#### Hubris app.toml
```toml
[kernel]
name = "demo"
target = "thumbv8m.main-none-eabihf"
board = "stm32h753-nucleo"
stacksize = 2048

[tasks.jefe]
name = "task-jefe"
priority = 0
max-sizes = {flash = 16384, ram = 2048}
stacksize = 1536
start = true

[tasks.uart_driver]
name = "drv-stm32h7-uart"
priority = 3
max-sizes = {flash = 8192, ram = 1024}
stacksize = 1024
start = true
uses = ["usart1"]
notifications = ["usart-irq"]
interrupts = {"usart1.irq" = "usart-irq"}

[tasks.echo_server]
name = "task-echo"
priority = 5
max-sizes = {flash = 4096, ram = 512}
stacksize = 768
start = true
task-slots = ["uart_driver"]  # IPC handle to uart_driver (uphill: priority 5 → 3)
```

#### Pigweed system.json5 (Translation)

**Reference:** See `pw_kernel/kernel/system_config.rs` for schema details.

```json5
{
  "system": {
    "name": "demo",
    "memory_layout": {
      "flash_start": "0x08000000",
      "flash_size": "0x200000",
      "ram_start": "0x20000000",
      "ram_size": "0x20000"
    }
  },
  "processes": [
    {
      "name": "supervisor",
      "priority": 0,
      "memory": {
        "flash_size": "0x4000",
        "ram_size": "0x800",
        "stack_size": "0x600"
      },
      "channels": [
        {"id": 1, "role": "handler", "name": "supervisor_service"}
      ]
    },
    {
      "name": "uart_driver",
      "priority": 3,
      "memory": {
        "flash_size": "0x2000",
        "ram_size": "0x400",
        "stack_size": "0x400",
        "device_memory": [
          {
            "name": "usart1",
            "address": "0x40011000",
            "size": "0x400",
            "writable": true
          }
        ]
      },
      "channels": [
        {"id": 2, "role": "handler", "name": "uart_service"}
      ],
      "interrupts": [37]
    },
    {
      "name": "echo_server",
      "priority": 5,
      "memory": {
        "flash_size": "0x1000",
        "ram_size": "0x200",
        "stack_size": "0x300"
      },
      "channels": [
        {"id": 3, "role": "initiator", "target": 2, "name": "uart_client"}
      ]
    }
  ]
}
```

**Key Translation Rules:**
- `tasks.*` → `processes[]`
- `priority` → `priority` (same semantics: 0 = highest)
- `max-sizes.{flash,ram}` → `memory.{flash_size,ram_size}`
- `stacksize` → `memory.stack_size`
- `uses = ["peripheral"]` → `memory.device_memory[]`
- `task-slots = ["other_task"]` → `channels[].role = "initiator"` (IPC dependencies)
- `notifications = ["name"]` → No direct equivalent (manual implementation)
- `interrupts = {"periph.irq" = "notif-name"}` → `interrupts[]` (numeric vector IDs)

**Important:** Hubris enforces the "uphill rule" - tasks can only IPC to equal/higher priority.
Pigweed has no such restriction, so you must manually verify priority relationships.

---

### Key Hubris Concepts You Must Understand

Before translating Idol interfaces, understand these core Hubris mechanisms:

#### Task-Slots: Compile-Time IPC Handles

Task-slots are **not channel IDs** - they are compile-time IPC capabilities:

```toml
# In app.toml
[tasks.hmac_client]
priority = 4
task-slots = ["digest_server"]  # Request IPC capability at build time

[tasks.digest_server]
priority = 2  # Higher priority (uphill rule: 4 → 2 allowed)
```

The kernel allocates a task-slot index at build time. The client code receives:
```rust
// Generated by build system, not runtime
const DIGEST_SERVER: TaskId = TaskId(3);  // Kernel-assigned slot
```

**Translation challenge:** Pigweed uses runtime channel IDs configured in system.json5.

#### Leases: Temporary IPC Memory Grants

Leases are **NOT static shared memory**. They are per-call memory grants:

```rust
// Hubris client code
let mut output = [0u32; 8];
let lease = Lease::read_write(&mut output)?;
digest_client.finalize_sha256(session_id, lease)?;
// After call completes, kernel revokes access
```

**How it works:**
1. Client creates a lease from its own memory
2. Kernel validates the memory region during IPC
3. Server receives temporary access via the lease parameter
4. After IPC completes, server loses access (MPU enforced)

**Translation challenge:** Pigweed has no lease mechanism. Use:
- Copy data in/out of messages (simple but slower)
- Static shared memory regions (requires careful synchronization)

#### Notifications: Async Signaling Without IPC

Notifications are separate from IPC - they're lightweight async signals:

```toml
[tasks.uart_driver]
notifications = ["uart-irq", "timer"]
interrupts = {"usart1.irq" = "uart-irq"}  # IRQ → notification mapping
```

```rust
// Server waits for IPC OR notifications
let notification_bits = idol_runtime::dispatch_with_notification(
    &mut server, 
    &mut buffer,
    NOTIFICATIONS
);

if notification_bits & UART_IRQ != 0 {
    // Handle interrupt
}
```

**Translation challenge:** Pigweed has no notification system. Must poll or use IPC for async events.

#### Uphill Rule: Compile-Time Priority Safety

The kernel enforces at **build time** that tasks only IPC to equal/higher priority:

```
Priority 4 → Priority 2  ✓ Allowed (uphill)
Priority 2 → Priority 4  ✗ Rejected at build time (downhill)
```

This prevents priority inversion. Pigweed has no such enforcement.

---

### 2. Idol Interface to Manual Protocol

#### Hubris: Idol Interface Definition

```rust
// uart.idol
Interface(
    name: "Uart",
    ops: {
        "write": (
            args: {
                "data": (type: "u8", recv: Borrow(max_size: 256)),
            },
            reply: Result(
                ok: "()",
                err: CLike("UartError"),
            ),
        ),
        "read": (
            args: {
                "buffer": (type: "u8", recv: BorrowMut(max_size: 256)),
            },
            reply: Result(
                ok: "usize",
                err: CLike("UartError"),
            ),
        ),
    },
)
```

#### Generated Hubris Client Code (What you currently use)

```rust
// Auto-generated by Idol in drv/uart-api/build.rs
// Client uses task-slot handle, not raw task ID
pub struct UartClient {
    task_slot: TaskId,  // Actually a task-slot handle from app.toml
}

impl UartClient {
    pub fn write(&self, data: &[u8]) -> Result<(), UartError> {
        // Generated IPC uses userlib::sys_send_stub with proper serialization
        let (request_code, request_payload) = /* idol-generated marshalling */;
        let mut response_buf = [0u8; MAX_RESPONSE_SIZE];
        
        userlib::sys_send_stub(
            self.task_slot,
            request_code,
            request_payload,
            &mut response_buf,
            &[/* leases if needed */]
        ).map_err(|e| match e {
            RequestError::Idol(idol_err) => UartError::from(idol_err),
            _ => UartError::IpcFailure,
        })
    }

    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, UartError> {
        // Similar pattern with proper hubpack serialization/deserialization
        // Uses leases for zero-copy buffer access
        let lease = Lease::read_write(buffer)?;
        /* idol-generated call with lease */
    }
}
```

#### Generated Hubris Server Code (What you currently use)

```rust
// Auto-generated server traits
pub trait UartServer {
    fn write(&mut self, data: &[u8]) -> Result<(), UartError>;
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, UartError>;
}

// Your implementation
struct MyUartDriver {
    hardware: stm32h7::usart::Usart1,
}

impl UartServer for MyUartDriver {
    fn write(&mut self, data: &[u8]) -> Result<(), UartError> {
        self.hardware.write_bytes(data)
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, UartError> {
        self.hardware.read_bytes(buffer)
    }
}

// Auto-generated dispatch loop with notification support
fn main() -> ! {
    let mut driver = MyUartDriver::new();
    let mut buffer = [0u8; idol_runtime::INCOMING_SIZE];
    
    loop {
        // Waits for IPC or notifications
        idol_runtime::dispatch_with_notification(
            &mut driver, 
            &mut buffer,
            NOTIFICATIONS  // From app.toml notifications array
        );
    }
}
```

---

#### Pigweed: Manual Protocol Translation

**Step 1: Define Binary Message Format**

```rust
// protocol.rs - Manual protocol definition

/// UART service protocol opcodes
const UART_OP_WRITE: u8 = 1;
const UART_OP_READ: u8 = 2;

/// Maximum data size for operations
const MAX_UART_DATA: usize = 256;

/// Error codes
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum UartError {
    BufferTooLarge = 1,
    HardwareError = 2,
    Timeout = 3,
}

/// Request message layout
#[repr(C)]
struct WriteRequest {
    opcode: u8,      // UART_OP_WRITE
    length: u8,      // Data length
    data: [u8; MAX_UART_DATA],
}

#[repr(C)]
struct ReadRequest {
    opcode: u8,      // UART_OP_READ
    max_length: u8,  // Maximum bytes to read
}

/// Response message layout
#[repr(C)]
struct WriteResponse {
    status: u8,      // 0 = success, else error code
}

#[repr(C)]
struct ReadResponse {
    status: u8,      // 0 = success, else error code
    length: u8,      // Actual bytes read
    data: [u8; MAX_UART_DATA],
}
```

**Step 2: Implement Manual Client**

**Reference:** Channel syscalls documented in `pw_kernel/userspace/syscall.rs`

```rust
// client.rs - Manual implementation (replaces Idol-generated client)

use pw_kernel_api as kernel;
use userspace::time::Instant;

pub struct UartClient {
    channel_id: u32,  // Channel to UART server
}

impl UartClient {
    pub fn new(channel_id: u32) -> Self {
        Self { channel_id }
    }

    /// Write data to UART
    pub fn write(&self, data: &[u8]) -> Result<(), UartError> {
        if data.len() > MAX_UART_DATA {
            return Err(UartError::BufferTooLarge);
        }

        // Construct request message
        let mut request = WriteRequest {
            opcode: UART_OP_WRITE,
            length: data.len() as u8,
            data: [0u8; MAX_UART_DATA],
        };
        request.data[..data.len()].copy_from_slice(data);

        // Send request and receive response
        let mut response_buf = [0u8; core::mem::size_of::<WriteResponse>()];
        let request_bytes = unsafe {
            core::slice::from_raw_parts(
                &request as *const _ as *const u8,
                core::mem::size_of::<WriteRequest>(),
            )
        };

        kernel::channel_transact(self.channel_id, request_bytes, &mut response_buf, Instant::MAX)
            .map_err(|_| UartError::HardwareError)?;

        // Parse response
        let response = unsafe {
            &*(response_buf.as_ptr() as *const WriteResponse)
        };

        if response.status == 0 {
            Ok(())
        } else {
            Err(match response.status {
                1 => UartError::BufferTooLarge,
                2 => UartError::HardwareError,
                3 => UartError::Timeout,
                _ => UartError::HardwareError,
            })
        }
    }

    /// Read data from UART
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, UartError> {
        if buffer.len() > MAX_UART_DATA {
            return Err(UartError::BufferTooLarge);
        }

        // Construct request
        let request = ReadRequest {
            opcode: UART_OP_READ,
            max_length: buffer.len() as u8,
        };

        let mut response_buf = [0u8; core::mem::size_of::<ReadResponse>()];
        let request_bytes = unsafe {
            core::slice::from_raw_parts(
                &request as *const _ as *const u8,
                core::mem::size_of::<ReadRequest>(),
            )
        };

        kernel::channel_transact(self.channel_id, request_bytes, &mut response_buf, Instant::MAX)
            .map_err(|_| UartError::HardwareError)?;

        // Parse response
        let response = unsafe {
            &*(response_buf.as_ptr() as *const ReadResponse)
        };

        if response.status == 0 {
            let len = response.length as usize;
            buffer[..len].copy_from_slice(&response.data[..len]);
            Ok(len)
        } else {
            Err(match response.status {
                1 => UartError::BufferTooLarge,
                2 => UartError::HardwareError,
                3 => UartError::Timeout,
                _ => UartError::HardwareError,
            })
        }
    }
}
```

**Step 3: Implement Manual Server**

**Reference:** Channel implementation details in `pw_kernel/kernel/object/channel.rs`

```rust
// server.rs - Manual implementation (replaces Idol-generated server)

use pw_kernel_api as kernel;

const UART_CHANNEL_ID: u32 = 2;  // From system.json5

pub struct UartServer {
    hardware: stm32h7::Usart1,  // Hardware abstraction
}

impl UartServer {
    pub fn new(hardware: stm32h7::Usart1) -> Self {
        Self { hardware }
    }

    /// Main server loop - replaces idol_runtime::dispatch()
    pub fn run(&mut self) -> ! {
        let mut request_buf = [0u8; core::mem::size_of::<WriteRequest>()];
        
        loop {
            // Block waiting for IPC request
            let msg_len = match kernel::channel_read(UART_CHANNEL_ID, 0, &mut request_buf) {
                Ok(len) => len,
                Err(_) => continue,  // Ignore errors, keep serving
            };

            // Dispatch based on opcode
            let response = match request_buf[0] {
                UART_OP_WRITE => self.handle_write(&request_buf[..msg_len]),
                UART_OP_READ => self.handle_read(&request_buf[..msg_len]),
                _ => {
                    // Unknown opcode, send error
                    let err_response = WriteResponse { status: 2 };
                    self.send_response(&err_response)
                }
            };

            // Send response back to client
            if let Err(_) = response {
                // Log error? In embedded context, may just continue
            }
        }
    }

    fn handle_write(&mut self, request_bytes: &[u8]) -> Result<(), ()> {
        let request = unsafe {
            &*(request_bytes.as_ptr() as *const WriteRequest)
        };

        let data = &request.data[..request.length as usize];
        
        // Perform actual hardware write
        let result = self.hardware.write_bytes(data);

        let response = WriteResponse {
            status: match result {
                Ok(()) => 0,
                Err(e) => match e {
                    HwError::BufferTooLarge => 1,
                    HwError::Timeout => 3,
                    _ => 2,
                }
            }
        };

        self.send_response(&response)
    }

    fn handle_read(&mut self, request_bytes: &[u8]) -> Result<(), ()> {
        let request = unsafe {
            &*(request_bytes.as_ptr() as *const ReadRequest)
        };

        let mut data_buf = [0u8; MAX_UART_DATA];
        let result = self.hardware.read_bytes(&mut data_buf[..request.max_length as usize]);

        let response = match result {
            Ok(bytes_read) => ReadResponse {
                status: 0,
                length: bytes_read as u8,
                data: data_buf,
            },
            Err(e) => ReadResponse {
                status: match e {
                    HwError::Timeout => 3,
                    _ => 2,
                },
                length: 0,
                data: [0u8; MAX_UART_DATA],
            }
        };

        self.send_response(&response)
    }

    fn send_response<T>(&self, response: &T) -> Result<(), ()> {
        let response_bytes = unsafe {
            core::slice::from_raw_parts(
                response as *const _ as *const u8,
                core::mem::size_of::<T>(),
            )
        };

        kernel::channel_respond(UART_CHANNEL_ID, response_bytes)
            .map_err(|_| ())
    }
}

// Process entry point
#[no_mangle]
fn main() -> ! {
    // Initialize hardware (address from system.json5 device_memory)
    let usart1 = unsafe { stm32h7::Usart1::new(0x40011000) };
    let mut server = UartServer::new(usart1);
    
    // Enter service loop
    server.run()
}
```

**Step 4: Client Usage Example**

```rust
// echo_server process main.rs

use pw_kernel_api as kernel;

const UART_CLIENT_CHANNEL: u32 = 3;  // From system.json5

#[no_mangle]
fn main() -> ! {
    let uart = UartClient::new(UART_CLIENT_CHANNEL);

    loop {
        // Read from UART
        let mut buffer = [0u8; 64];
        match uart.read(&mut buffer) {
            Ok(len) => {
                // Echo back
                if let Err(e) = uart.write(&buffer[..len]) {
                    // Handle error
                }
            }
            Err(e) => {
                // Handle error
            }
        }
    }
}
```

---

### 3. Shared Memory / Leases

#### Hubris Approach (Leases)

```rust
// Hubris task using leases for zero-copy
use idol_runtime::{Borrow, BorrowMut};

fn process_data(data: Borrow<[u8]>) -> Result<(), Error> {
    // data is borrowed from caller's memory
    // No copy needed
    hardware.dma_transfer(data.as_slice())
}

fn fill_buffer(buffer: BorrowMut<[u8]>) -> Result<usize, Error> {
    // buffer is mutable borrow from caller
    let bytes_read = hardware.read_into(buffer.as_mut_slice())?;
    Ok(bytes_read)
}
```

#### Pigweed Translation (Shared Memory Regions)

**system.json5 Configuration:**

```json5
{
  "shared_memory": [
    {
      "name": "dma_buffer",
      "size": "0x1000",
      "processes": [
        {"name": "app", "writable": true},
        {"name": "dma_driver", "writable": true}
      ]
    }
  ]
}
```

**Manual Implementation:**

```rust
use pw_kernel_api as kernel;
use userspace::time::Instant;

// Define shared memory address (generated from system.json5)
const DMA_BUFFER_ADDR: usize = 0x20010000;
const DMA_BUFFER_SIZE: usize = 0x1000;

// Client side
pub struct DmaClient {
    channel_id: u32,
    shared_buffer: &'static mut [u8],
}

impl DmaClient {
    pub fn new(channel_id: u32) -> Self {
        let shared_buffer = unsafe {
            core::slice::from_raw_parts_mut(
                DMA_BUFFER_ADDR as *mut u8,
                DMA_BUFFER_SIZE
            )
        };
        Self { channel_id, shared_buffer }
    }

    pub fn transfer_data(&mut self, data: &[u8]) -> Result<(), Error> {
        // Copy to shared memory
        self.shared_buffer[..data.len()].copy_from_slice(data);

        // Send message with length (not the data itself)
        let request = DmaRequest {
            opcode: DMA_OP_TRANSFER,
            offset: 0,
            length: data.len() as u32,
        };

        let mut response_buf = [0u8; 4];
        kernel::channel_transact(self.channel_id, request.as_bytes(), &mut response_buf, Instant::MAX)?;

        Ok(())
    }
}

// Server side
pub struct DmaServer {
    hardware: DmaController,
    shared_buffer: &'static [u8],
}

impl DmaServer {
    pub fn new(hardware: DmaController) -> Self {
        let shared_buffer = unsafe {
            core::slice::from_raw_parts(
                DMA_BUFFER_ADDR as *const u8,
                DMA_BUFFER_SIZE
            )
        };
        Self { hardware, shared_buffer }
    }

    fn handle_transfer(&mut self, request: &DmaRequest) -> Result<(), Error> {
        let data = &self.shared_buffer[
            request.offset as usize..
            (request.offset + request.length) as usize
        ];
        
        // Use DMA hardware to transfer from shared memory
        self.hardware.transfer(data)?;
        Ok(())
    }
}
```

**Translation Rules:**
- `Borrow<[T]>` → Shared memory region + offset/length in message
- `BorrowMut<[T]>` → Shared memory region + offset/length in message
- Idol automatic lease tracking → Manual synchronization (semaphores or protocol states)
- Compile-time borrow checking → Runtime protocol enforcement

---

## Configuration Translation

### Complete app.toml → system.json5 Example

#### Hubris app.toml (Complex Example)

```toml
[kernel]
name = "environmental-monitor"
target = "thumbv8m.main-none-eabihf"
board = "stm32h753-nucleo"
stacksize = 2048

[tasks.jefe]
name = "task-jefe"
priority = 0
max-sizes = {flash = 16384, ram = 4096}
stacksize = 2048
start = true

[tasks.i2c_driver]
name = "drv-stm32h7-i2c"
priority = 2
max-sizes = {flash = 8192, ram = 2048}
stacksize = 1536
start = true
uses = ["i2c1"]
interrupts = {"i2c1.event" = 31, "i2c1.error" = 32}

[tasks.sensor_task]
name = "task-sensor"
priority = 4
max-sizes = {flash = 12288, ram = 2048}
stacksize = 1024
start = true
notifications = ["sensor.timer"]

[tasks.net_task]
name = "task-net"
priority = 5
max-sizes = {flash = 32768, ram = 8192}
stacksize = 2048
start = true

[tasks.net_task.task-slots]
i2c = "i2c_driver"
sensor = "sensor_task"

[peripherals.i2c1]
address = 0x40005400
size = 0x400
```

#### Pigweed system.json5 (Translation)

```json5
{
  "system": {
    "name": "environmental-monitor",
    "memory_layout": {
      "flash_start": "0x08000000",
      "flash_size": "0x200000",
      "ram_start": "0x20000000",
      "ram_size": "0x20000"
    }
  },
  
  "processes": [
    {
      "name": "supervisor",
      "priority": 0,
      "memory": {
        "flash_size": "0x4000",
        "ram_size": "0x1000",
        "stack_size": "0x800"
      },
      "channels": [
        {"id": 1, "role": "handler", "name": "supervisor_service"}
      ]
    },
    
    {
      "name": "i2c_driver",
      "priority": 2,
      "memory": {
        "flash_size": "0x2000",
        "ram_size": "0x800",
        "stack_size": "0x600",
        "device_memory": [
          {
            "name": "i2c1",
            "address": "0x40005400",
            "size": "0x400",
            "writable": true
          }
        ]
      },
      "channels": [
        {"id": 2, "role": "handler", "name": "i2c_service"}
      ],
      "interrupts": [31, 32]
    },
    
    {
      "name": "sensor_task",
      "priority": 4,
      "memory": {
        "flash_size": "0x3000",
        "ram_size": "0x800",
        "stack_size": "0x400"
      },
      "channels": [
        {"id": 3, "role": "handler", "name": "sensor_service"},
        {"id": 4, "role": "initiator", "target": 2, "name": "i2c_client"}
      ]
    },
    
    {
      "name": "net_task",
      "priority": 5,
      "memory": {
        "flash_size": "0x8000",
        "ram_size": "0x2000",
        "stack_size": "0x800"
      },
      "channels": [
        {"id": 5, "role": "initiator", "target": 2, "name": "i2c_client"},
        {"id": 6, "role": "initiator", "target": 3, "name": "sensor_client"}
      ]
    }
  ],
  
  "shared_memory": [
    {
      "name": "sensor_data",
      "size": "0x400",
      "processes": [
        {"name": "sensor_task", "writable": true},
        {"name": "net_task", "writable": false}
      ]
    }
  ]
}
```

**Translation Key Points:**

1. **Task Slots → Channels:**
   - Hubris: `[tasks.net_task.task-slots] i2c = "i2c_driver"`
   - Pigweed: `{"role": "initiator", "target": 2, "name": "i2c_client"}`

2. **Peripherals → Device Memory:**
   - Hubris: `[peripherals.i2c1]` + `uses = ["i2c1"]`
   - Pigweed: `memory.device_memory[{name: "i2c1", address: "0x40005400", ...}]`

3. **Interrupts:**
   - Hubris: `interrupts = {"i2c1.event" = 31, "i2c1.error" = 32}`
   - Pigweed: `interrupts: [31, 32]`

4. **Notifications:**
   - Hubris: `notifications = ["sensor.timer"]`
   - Pigweed: Implement via timer interrupt + signal mechanism (manual)

---

## Build System Integration

### Hubris Build (Cargo + xtask)

```toml
# Cargo.toml for Hubris task
[package]
name = "task-sensor"
version = "0.1.0"
edition = "2021"

[dependencies]
idol-runtime = "0.2"
userlib = { path = "../../sys/userlib" }

[build-dependencies]
idol = "0.2"

# build.rs
use idol::Generator;

fn main() {
    Generator::new()
        .with_idol_file("sensor.idol")
        .generate()
        .unwrap();
}
```

### Pigweed Build (Bazel)

**Reference:** See `pw_kernel/BUILD.bazel` for build configuration examples.

```python
# BUILD.bazel

load("//pw_build:pigweed.bzl", "pw_rust_binary")

pw_rust_binary(
    name = "sensor_task",
    srcs = [
        "src/main.rs",
        "src/protocol.rs",  # Manual protocol definitions
        "src/client.rs",    # Manual client implementation
    ],
    deps = [
        "//pw_kernel:pw_kernel_api",
        "//third_party/stm32h7:pac",
    ],
    crate_features = ["no_std"],
    target = "thumbv8m.main-none-eabihf",
)

# System configuration target
filegroup(
    name = "system_config",
    srcs = ["system.json5"],
)

# Complete kernel image
pw_kernel_image(
    name = "environmental_monitor",
    system_config = ":system_config",
    processes = [
        ":supervisor",
        ":i2c_driver",
        ":sensor_task",
        ":net_task",
    ],
)
```

**Key Differences:**
- No build.rs code generation (manual protocol.rs instead)
- Bazel targets instead of Cargo packages
- pw_kernel_image rule combines all processes
- Manual dependency management

---

## Complete Porting Example

### Original Hubris Application

**Directory Structure:**
```
hubris-app/
├── app.toml
├── tasks/
│   ├── sensor/
│   │   ├── sensor.idol
│   │   ├── Cargo.toml
│   │   ├── build.rs
│   │   └── src/
│   │       └── main.rs
│   └── controller/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
```

**sensor.idol:**
```rust
Interface(
    name: "Sensor",
    ops: {
        "read_temperature": (
            reply: Result(ok: "f32", err: CLike("SensorError")),
        ),
        "read_humidity": (
            reply: Result(ok: "f32", err: CLike("SensorError")),
        ),
    },
)
```

**tasks/sensor/src/main.rs (Hubris):**
```rust
#![no_std]
#![no_main]

use idol_runtime::ClientError;
use userlib::*;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/sensor_impl.rs"));
}

struct SensorImpl {
    i2c: i2c::I2cClient,
    address: u8,
}

impl generated::SensorServer for SensorImpl {
    fn read_temperature(&mut self) -> Result<f32, SensorError> {
        let mut buf = [0u8; 2];
        self.i2c.read(self.address, 0x00, &mut buf)?;
        let raw = u16::from_be_bytes(buf);
        Ok((raw as f32) * 0.01 - 40.0)
    }

    fn read_humidity(&mut self) -> Result<f32, SensorError> {
        let mut buf = [0u8; 2];
        self.i2c.read(self.address, 0x01, &mut buf)?;
        let raw = u16::from_be_bytes(buf);
        Ok((raw as f32) * 0.1)
    }
}

#[export_name = "main"]
fn main() -> ! {
    let i2c = i2c::I2cClient::from_task_slot(TASK_I2C);
    let mut sensor = SensorImpl {
        i2c,
        address: 0x44,
    };

    loop {
        generated::dispatch(&mut sensor);
    }
}
```

**tasks/controller/src/main.rs (Hubris Client):**
```rust
#![no_std]
#![no_main]

use userlib::*;

mod sensor_client {
    include!(concat!(env!("OUT_DIR"), "/sensor_client.rs"));
}

#[export_name = "main"]
fn main() -> ! {
    let sensor = sensor_client::SensorClient::from_task_slot(TASK_SENSOR);

    loop {
        match sensor.read_temperature() {
            Ok(temp) => {
                // Use temperature
            }
            Err(e) => {
                // Handle error
            }
        }

        sys_sleep(1000);  // Sleep 1 second
    }
}
```

---

### Ported Pigweed Application

**Directory Structure:**
```
pigweed-app/
├── system.json5
├── processes/
│   ├── sensor/
│   │   ├── BUILD.bazel
│   │   └── src/
│   │       ├── main.rs
│   │       ├── protocol.rs
│   │       └── server.rs
│   └── controller/
│       ├── BUILD.bazel
│       └── src/
│           ├── main.rs
│           ├── protocol.rs
│           └── client.rs
```

**system.json5:**
```json5
{
  "system": {
    "name": "sensor-system",
    "memory_layout": {
      "flash_start": "0x08000000",
      "flash_size": "0x100000",
      "ram_start": "0x20000000",
      "ram_size": "0x20000"
    }
  },
  "processes": [
    {
      "name": "sensor",
      "priority": 3,
      "memory": {
        "flash_size": "0x2000",
        "ram_size": "0x800",
        "stack_size": "0x400"
      },
      "channels": [
        {"id": 1, "role": "handler", "name": "sensor_service"},
        {"id": 2, "role": "initiator", "target": 10, "name": "i2c_client"}
      ]
    },
    {
      "name": "controller",
      "priority": 5,
      "memory": {
        "flash_size": "0x1000",
        "ram_size": "0x400",
        "stack_size": "0x300"
      },
      "channels": [
        {"id": 3, "role": "initiator", "target": 1, "name": "sensor_client"}
      ]
    }
  ]
}
```

**processes/sensor/src/protocol.rs (Manual Protocol):**
```rust
#![no_std]

/// Sensor service opcodes
pub const SENSOR_OP_READ_TEMPERATURE: u8 = 1;
pub const SENSOR_OP_READ_HUMIDITY: u8 = 2;

/// Error codes
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum SensorError {
    I2cError = 1,
    InvalidData = 2,
    NotReady = 3,
}

/// Request message
#[repr(C)]
pub struct SensorRequest {
    pub opcode: u8,
}

/// Response message
#[repr(C)]
pub struct SensorResponse {
    pub status: u8,      // 0 = success, else error code
    pub value: [u8; 4],  // f32 as bytes
}

impl SensorResponse {
    pub fn ok(value: f32) -> Self {
        Self {
            status: 0,
            value: value.to_le_bytes(),
        }
    }

    pub fn err(error: SensorError) -> Self {
        Self {
            status: error as u8,
            value: [0; 4],
        }
    }

    pub fn parse(&self) -> Result<f32, SensorError> {
        if self.status == 0 {
            Ok(f32::from_le_bytes(self.value))
        } else {
            Err(match self.status {
                1 => SensorError::I2cError,
                2 => SensorError::InvalidData,
                3 => SensorError::NotReady,
                _ => SensorError::I2cError,
            })
        }
    }
}
```

**processes/sensor/src/server.rs (Manual Server):**
```rust
#![no_std]

use pw_kernel_api as kernel;
use crate::protocol::*;

const SENSOR_CHANNEL: u32 = 1;
const I2C_CLIENT_CHANNEL: u32 = 2;
const SENSOR_I2C_ADDR: u8 = 0x44;

pub struct SensorServer {
    i2c_channel: u32,
}

impl SensorServer {
    pub fn new() -> Self {
        Self {
            i2c_channel: I2C_CLIENT_CHANNEL,
        }
    }

    pub fn run(&mut self) -> ! {
        let mut request_buf = [0u8; core::mem::size_of::<SensorRequest>()];

        loop {
            // Wait for request
            if kernel::channel_read(SENSOR_CHANNEL, 0, &mut request_buf).is_err() {
                continue;
            }

            let request = unsafe {
                &*(request_buf.as_ptr() as *const SensorRequest)
            };

            // Dispatch
            let response = match request.opcode {
                SENSOR_OP_READ_TEMPERATURE => self.read_temperature(),
                SENSOR_OP_READ_HUMIDITY => self.read_humidity(),
                _ => SensorResponse::err(SensorError::InvalidData),
            };

            // Send response
            let response_bytes = unsafe {
                core::slice::from_raw_parts(
                    &response as *const _ as *const u8,
                    core::mem::size_of::<SensorResponse>(),
                )
            };

            let _ = kernel::channel_respond(SENSOR_CHANNEL, response_bytes);
        }
    }

    fn read_temperature(&self) -> SensorResponse {
        // Call I2C service (simplified)
        let mut buf = [0u8; 2];
        match self.i2c_read(SENSOR_I2C_ADDR, 0x00, &mut buf) {
            Ok(()) => {
                let raw = u16::from_be_bytes(buf);
                let temp = (raw as f32) * 0.01 - 40.0;
                SensorResponse::ok(temp)
            }
            Err(e) => SensorResponse::err(SensorError::I2cError),
        }
    }

    fn read_humidity(&self) -> SensorResponse {
        let mut buf = [0u8; 2];
        match self.i2c_read(SENSOR_I2C_ADDR, 0x01, &mut buf) {
            Ok(()) => {
                let raw = u16::from_be_bytes(buf);
                let humidity = (raw as f32) * 0.1;
                SensorResponse::ok(humidity)
            }
            Err(e) => SensorResponse::err(SensorError::I2cError),
        }
    }

    fn i2c_read(&self, addr: u8, reg: u8, buf: &mut [u8]) -> Result<(), ()> {
        // Manual I2C protocol implementation
        // (Similar pattern as UART example above)
        // ... I2C request/response handling ...
        Ok(())
    }
}
```

**processes/sensor/src/main.rs:**
```rust
#![no_std]
#![no_main]

mod protocol;
mod server;

use server::SensorServer;

#[no_mangle]
fn main() -> ! {
    let mut server = SensorServer::new();
    server.run()
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
```

**processes/controller/src/client.rs (Manual Client):**
```rust
#![no_std]

use pw_kernel_api as kernel;
use userspace::time::Instant;
use crate::protocol::*;

pub struct SensorClient {
    channel: u32,
}

impl SensorClient {
    pub fn new(channel: u32) -> Self {
        Self { channel }
    }

    pub fn read_temperature(&self) -> Result<f32, SensorError> {
        let request = SensorRequest {
            opcode: SENSOR_OP_READ_TEMPERATURE,
        };

        let mut response_buf = [0u8; core::mem::size_of::<SensorResponse>()];
        let request_bytes = unsafe {
            core::slice::from_raw_parts(
                &request as *const _ as *const u8,
                core::mem::size_of::<SensorRequest>(),
            )
        };

        kernel::channel_transact(self.channel, request_bytes, &mut response_buf, Instant::MAX)
            .map_err(|_| SensorError::I2cError)?;

        let response = unsafe {
            &*(response_buf.as_ptr() as *const SensorResponse)
        };

        response.parse()
    }

    pub fn read_humidity(&self) -> Result<f32, SensorError> {
        let request = SensorRequest {
            opcode: SENSOR_OP_READ_HUMIDITY,
        };

        let mut response_buf = [0u8; core::mem::size_of::<SensorResponse>()];
        let request_bytes = unsafe {
            core::slice::from_raw_parts(
                &request as *const _ as *const u8,
                core::mem::size_of::<SensorRequest>(),
            )
        };

        kernel::channel_transact(self.channel, request_bytes, &mut response_buf, Instant::MAX)
            .map_err(|_| SensorError::I2cError)?;

        let response = unsafe {
            &*(response_buf.as_ptr() as *const SensorResponse)
        };

        response.parse()
    }
}
```

**processes/controller/src/main.rs:**
```rust
#![no_std]
#![no_main]

mod protocol;
mod client;

use pw_kernel_api as kernel;
use client::SensorClient;

const SENSOR_CLIENT_CHANNEL: u32 = 3;

#[no_mangle]
fn main() -> ! {
    let sensor = SensorClient::new(SENSOR_CLIENT_CHANNEL);

    loop {
        match sensor.read_temperature() {
            Ok(temp) => {
                // Use temperature value
                // (In real code, might send to logger, display, etc.)
            }
            Err(e) => {
                // Handle error
            }
        }

        // Sleep for 1 second (Pigweed timer API)
        kernel::sleep_ms(1000);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
```

**processes/sensor/BUILD.bazel:**
```python
load("//pw_build:pigweed.bzl", "pw_rust_binary")

pw_rust_binary(
    name = "sensor",
    srcs = [
        "src/main.rs",
        "src/protocol.rs",
        "src/server.rs",
    ],
    deps = [
        "//pw_kernel:pw_kernel_api",
    ],
    crate_features = ["no_std"],
)
```

**processes/controller/BUILD.bazel:**
```python
load("//pw_build:pigweed.bzl", "pw_rust_binary")

pw_rust_binary(
    name = "controller",
    srcs = [
        "src/main.rs",
        "src/protocol.rs",
        "src/client.rs",
    ],
    deps = [
        "//pw_kernel:pw_kernel_api",
    ],
    crate_features = ["no_std"],
)
```

---

## Testing and Validation

### Testing Strategy

1. **Unit Testing (Per-Process)**
   ```rust
   // tests/protocol_test.rs
   #[test]
   fn test_sensor_response_serialization() {
       let response = SensorResponse::ok(25.5);
       assert_eq!(response.status, 0);
       assert_eq!(response.parse().unwrap(), 25.5);
   }

   #[test]
   fn test_error_response() {
       let response = SensorResponse::err(SensorError::I2cError);
       assert_eq!(response.status, 1);
       assert!(response.parse().is_err());
   }
   ```

2. **Integration Testing (IPC Flows)**

   **Reference:** See `pw_kernel/tests/` for test examples.
   
   ```rust
   // On host target (k_host config)
   #[test]
   fn test_sensor_read_temperature() {
       // Spawn sensor server process
       // Spawn controller client process
       // Verify IPC communication
       // Check temperature value within range
   }
   ```

3. **Hardware Testing**

   **Reference:** Available targets in `pw_kernel/target/` (mps2_an505, pw_rp2350, qemu_virt_riscv32)
   
   ```bash
   # Build for actual hardware (example, adjust for your target)
   bazelisk build --config k_qemu_mps2_an505 //:sensor_system
   
   # Flash to device
   openocd -f board/st_nucleo_h753zi.cfg \
       -c "program bazel-bin/sensor_system.elf verify reset exit"
   
   # Monitor via UART
   picocom /dev/ttyACM0 -b 115200
   ```

### Validation Checklist

- [ ] All Idol interfaces documented with manual protocols
- [ ] Request/response message formats specified
- [ ] Client implementations match Hubris API signatures
- [ ] Server dispatch loops implemented
- [ ] Error handling covers all Idol error types
- [ ] Shared memory regions configured (if used)
- [ ] Interrupt handlers mapped correctly
- [ ] Process priorities match Hubris task priorities
- [ ] Memory sizes sufficient (check stack usage)
- [ ] IPC channel IDs consistent across system.json5
- [ ] Build system generates correct binary layout
- [ ] Unit tests pass for protocol serialization
- [ ] Integration tests verify IPC flows
- [ ] Hardware testing on target device successful

---

## Known Limitations

### What Works Well

✅ **Process Isolation:** Pigweed MPU protection equivalent to Hubris
✅ **Synchronous IPC:** channel_transact matches Hubris call semantics
✅ **Static Configuration:** system.json5 similar to app.toml
✅ **Zero-Copy via Shared Memory:** Achievable with careful design
✅ **Peripheral Ownership:** MPU-enforced device memory regions
✅ **Priority-Based Scheduling:** Same as Hubris

### What Requires Manual Work

⚠️ **No IDL Code Generation:** Must hand-write all client/server stubs
⚠️ **Protocol Versioning:** No automatic compatibility checking
⚠️ **Type Safety:** Binary message formats error-prone
⚠️ **Documentation:** Must manually document protocols (no auto-generated docs)
⚠️ **Refactoring:** Changes to interfaces require manual updates everywhere

### What's Missing

❌ **Supervisor/Recovery:** No supervisor pattern or examples (DIY required)
❌ **Idol-Style Leases:** Manual shared memory management
❌ **Notifications:** No kernel notification primitive (use interrupts + signals)
❌ **Debug Tooling:** Limited compared to Hubris's task inspection

### Performance Considerations

| Aspect | Hubris | Pigweed | Impact |
|--------|--------|---------|--------|
| **IPC Latency** | ~2-5 μs | ~2-5 μs | ✅ Equivalent |
| **Context Switch** | ~1-2 μs | ~1-2 μs | ✅ Equivalent |
| **Code Size** | Generated stubs compact | Hand-written can be verbose | ⚠️ Potentially larger |
| **RAM Usage** | Optimized by compiler | Depends on manual implementation | ⚠️ Needs careful design |

---

## Decision Matrix

### Should You Port to Pigweed?

| Factor | Port Now | Wait for Tooling | Stay with Hubris |
|--------|----------|------------------|------------------|
| **Application Size** | Small (<10 tasks) | Medium (10-50 tasks) | Large (50+ tasks) |
| **IDL Complexity** | Simple (<5 ops/interface) | Medium (5-20 ops) | Complex (20+ ops) |
| **Development Timeline** | Long (>6 months) | Medium (3-6 months) | Short (<3 months) |
| **Team Familiarity** | Knows Pigweed well | Learning both | Hubris experts |
| **Existing Hubris Code** | None/minimal | Moderate | Extensive |
| **Need Pigweed Features** | Critical | Desired | Not needed |
| **Maintenance Burden** | Can handle manual updates | Prefer automation | Must have automation |

### Recommendation Algorithm

**Score each factor:** -2 (strongly favors Hubris) to +2 (strongly favors Pigweed)

- Application Size: Small = +2, Medium = 0, Large = -2
- IDL Complexity: Simple = +2, Medium = 0, Complex = -2
- Development Timeline: Long = +1, Medium = 0, Short = -2
- Team: Pigweed experts = +2, Equal = 0, Hubris experts = -1
- Existing Code: Minimal = +2, Moderate = -1, Extensive = -2
- Need Pigweed: Critical = +2, Desired = +1, Not needed = -2
- Maintenance: Can handle = +1, Prefer auto = 0, Must have = -2

**Total Score:**
- **+5 or higher:** Port to Pigweed now
- **0 to +4:** Consider porting with gradual migration
- **-1 to -4:** Wait for Pigweed tooling improvements
- **-5 or lower:** Stay with Hubris

---

## Migration Strategies

### Strategy 1: Incremental Port (Recommended)

**Risk:** Low to Medium

1. Port one simple leaf task (no dependencies)
2. Port its client task
3. Port core driver tasks (I2C, UART, etc.)
4. Port application logic tasks
5. Integration testing and refinement

**Benefits:**
- Validate approach early with simple tasks
- Learn Pigweed conventions incrementally
- Can abort if issues discovered
- Maintain working Hubris version in parallel

### Strategy 2: Big Bang Port

**Risk:** High

1. Map all Idol interfaces to manual protocols
2. Port all tasks simultaneously
3. Integration and debugging
4. Testing and refinement

**Benefits:**
- Faster completion if successful
- No hybrid maintenance
- Forces complete understanding upfront

**Risks:**
- High debugging complexity
- Difficult to isolate issues
- All-or-nothing approach

### Strategy 3: Hybrid Approach

**Risk:** Low

1. Keep Hubris running on production hardware
2. Port to Pigweed on separate development boards
3. Run both in parallel for extended validation period
4. Cut over only when Pigweed version fully validated

**Benefits:**
- Lowest risk
- Production system unaffected
- Ample time for optimization

**Drawbacks:**
- Longest timeline
- Maintaining two codebases
- Resource intensive

---

## Tooling Recommendations

### Code Generation Helper (Optional)

Consider building a simple Idol → Pigweed protocol generator:

```bash
# Usage
idol2pw sensor.idol --output src/protocol.rs

# Generates:
# - Protocol constants and opcodes
# - Request/response structures
# - Client stub skeleton
# - Server dispatch skeleton
```

**ROI:** High for applications with many interfaces

### Build Script Template

```python
# generate_process.bzl

def pigweed_process(name, protocol_file, server_impl, client_deps = []):
    """Helper to reduce boilerplate"""
    
    native.genrule(
        name = name + "_protocol",
        srcs = [protocol_file],
        outs = [name + "_protocol.rs"],
        cmd = "cp $< $@",  # Or run code generator
    )
    
    pw_rust_binary(
        name = name + "_server",
        srcs = [
            server_impl,
            ":" + name + "_protocol",
        ],
        deps = ["//pw_kernel:pw_kernel_api"],
    )
    
    rust_library(
        name = name + "_client",
        srcs = [":" + name + "_protocol"],
        deps = ["//pw_kernel:pw_kernel_api"] + client_deps,
    )
```

### Documentation Template

```markdown
# Protocol: {Service Name}

## Overview
Brief description of service

## Channel Configuration
- Server Channel ID: {N}
- Client Channel IDs: {M1, M2, ...}

## Operations

### {Operation Name}
**Opcode:** 0x{XX}

**Request:**
| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0      | 1    | u8   | Opcode      |
| 1      | 2    | u16  | Parameter 1 |
| ...    | ...  | ...  | ...         |

**Response:**
| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0      | 1    | u8   | Status      |
| ...    | ...  | ...  | ...         |

**Errors:**
- 0x01: {Error Name} - {Description}
- ...

## Example Usage
```rust
// Client code
let client = {Service}Client::new(CHANNEL_ID);
let result = client.{operation}(params)?;
```
```

---

## Conclusion

Porting Hubris applications to Pigweed kernel is **feasible but labor-intensive** without IDL tooling. The core kernel primitives (MPU, IPC, scheduling) are comparable, but the lack of code generation means significant manual work to replicate Idol-generated functionality.

### Key Takeaways

1. **Manual Protocol Design Is Critical:** Document message formats meticulously
2. **Start Small:** Port simple tasks first to validate approach
3. **Consider Building Tools:** Even basic code generation helps significantly
4. **Test Extensively:** Without compile-time guarantees, runtime validation is essential
5. **Weigh Costs vs. Benefits:** Ensure Pigweed-specific features justify porting effort


### Future Outlook

If Pigweed kernel adds IDL tooling in the future, porting would become significantly easier. Until then, evaluate carefully whether manual porting effort aligns with project goals.

---

## Appendix: Quick Reference

### Hubris → Pigweed Mapping Cheat Sheet

| Hubris Concept | Pigweed Equivalent | Notes |
|----------------|-------------------|-------|
| Task | Process | Same semantics |
| app.toml | system.json5 | Different syntax |
| .idol file | protocol.rs (manual) | No code generation |
| idol_runtime::dispatch() | Manual loop + channel_read/respond | Hand-written |
| Client stub | Manual client.rs | Hand-written |
| sys_send() | channel_transact() | Similar API |
| Lease (Borrow) | Shared memory region | Manual coordination |
| Supervisor task | DIY supervisor process | Pattern, not built-in |
| Notification | Signal/interrupt | Different mechanism |
| TaskId | ProcessId | Same concept |
| TASK_SLOT | Channel ID from config | Static binding |

### Common Pitfalls

1. **Confusing terminology:** Remember Client=Initiator, Server=Handler in Pigweed
2. **Forgetting to reverse channels:** Client/initiator → Server/handler mapping
3. **Mismatched message sizes:** Buffer overruns in channel_read
3. **Incorrect opcode dispatch:** Missing cases in server loop
4. **Shared memory races:** Without leases, need manual synchronization
5. **Priority inversion:** Map Hubris priorities carefully to Pigweed
6. **Stack overflow:** Pigweed stack sizes need tuning
7. **Interrupt conflicts:** Vector numbers differ across MCUs

### Useful Commands

**See also:** `pw_kernel/quickstart.rst` for detailed build instructions.

```bash
# Build host target for testing
# Targets defined in pw_kernel/target/
bazelisk build --config k_host //processes/...

# Build for ARM Cortex-M33 (MPS2-AN505)
bazelisk build --config k_qemu_mps2_an505 //:system_image

# Build for RP2350 (Cortex-M33)
bazelisk build --config k_rp2350 //:system_image

# Build for RISC-V
bazelisk build --config k_qemu_virt_riscv32 //:system_image

# Run on QEMU
bazelisk run --config k_qemu_mps2_an505 //:system_image

# Check binary sizes
bazelisk build --config k_host //processes:sensor
ls -lh bazel-bin/processes/sensor

# View system configuration (requires jq)
cat system.json5 | jq '.processes[] | {name, priority, channels}'
```

---

**Document Version:** 1.0
**Last Updated:** November 18, 2025
**Author:** Pigweed/Hubris Expert

**Related Pigweed Documentation:**
- `pw_kernel/docs.rst` - Main pw_kernel documentation
- `pw_kernel/quickstart.rst` - Getting started guide
- `pw_kernel/kernel/object/channel.rs` - Channel IPC implementation
- `pw_kernel/userspace/syscall.rs` - Syscall API definitions
- `pw_kernel/kernel/system_config.rs` - System configuration schema
- `pw_kernel/tests/README.md` - Testing guide
- `IPC_CHANNELS_GUIDE.md` - IPC usage guide (workspace root)
