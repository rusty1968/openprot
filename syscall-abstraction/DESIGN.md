# Syscall Abstraction Design Document

**Version:** 1.0  
**Date:** July 7, 2025  
**Author:** Rust Developer

## Overview

This document describes the design and architecture of the OS-agnostic syscall abstraction library, focusing on how the various traits and components interoperate to provide a unified interface across different operating systems and execution models.

## Architecture Overview

The syscall abstraction is built as a layered architecture with clear separation of concerns:

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│                   (User Code)                              │
├─────────────────────────────────────────────────────────────┤
│              Execution Adapters                            │
│    SyncAdapter │ AsyncAdapter │ PollAdapter                │
├─────────────────────────────────────────────────────────────┤
│                  Operation Trait                           │
│           execute(ExecutionContext) → Result               │
├─────────────────────────────────────────────────────────────┤
│                 GenericSyscalls Trait                      │
│  command_immediate() │ setup_callback() │ poll_callback()  │
├─────────────────────────────────────────────────────────────┤
│                    OS Adapters                             │
│  TockAdapter │ LinuxAdapter │ HubrisAdapter │
├─────────────────────────────────────────────────────────────┤
│                 Operating System                           │
│    Tock │ Linux │ Hubris         │
└─────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. GenericSyscalls Trait

The `GenericSyscalls` trait is the foundation of the abstraction, providing a unified interface for all syscall operations.

```rust
pub trait GenericSyscalls: Clone {
    // Immediate operations
    fn command_immediate(&self, driver_id: u32, command_id: u32, arg0: u32, arg1: u32) -> Result<u32, ErrorCode>;
    
    // Asynchronous operations
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode>;
    fn poll_callback(&self, handle: &CallbackHandle) -> CallbackStatus;
    fn wait_callback(&self, handle: &CallbackHandle) -> Result<CallbackData, ErrorCode>;
    
    // Buffer management
    fn setup_buffer(&self, driver_id: u32, buffer_id: u32, buffer: &[u8]) -> Result<BufferHandle, ErrorCode>;
    fn setup_mutable_buffer(&self, driver_id: u32, buffer_id: u32, buffer: &mut [u8]) -> Result<BufferHandle, ErrorCode>;
    fn get_buffer_info(&self, handle: &BufferHandle) -> Result<BufferInfo, ErrorCode>;
    
    // Resource cleanup
    fn cleanup_buffer(&self, handle: BufferHandle) -> Result<(), ErrorCode>;
    fn cleanup_callback(&self, handle: CallbackHandle) -> Result<(), ErrorCode>;
    
    // Console-specific convenience methods
    fn write_buffer(&self, data: &[u8]) -> Result<CallbackHandle, ErrorCode>;
    fn read_buffer(&self, handle: BufferHandle, size: usize) -> Result<CallbackHandle, ErrorCode>;
    fn allocate_buffer(&self, size: usize) -> Result<BufferHandle, ErrorCode>;
    fn free_buffer(&self, handle: BufferHandle) -> Result<(), ErrorCode>;
    fn read_from_buffer(&self, handle: BufferHandle) -> Result<Vec<u8>, ErrorCode>;
    fn input_available(&self) -> Result<bool, ErrorCode>;
    fn clear_console(&self) -> Result<(), ErrorCode>;
}
```

**Key Design Decisions:**

- **Handle-based Resource Management**: Uses opaque handles (`CallbackHandle`, `BufferHandle`) to manage resources safely across OS boundaries
- **Clone Requirement**: All implementations must be `Clone` to support sharing across threads and contexts
- **Unified Error Handling**: All operations return `Result<T, ErrorCode>` for consistent error handling
- **Immediate vs Async Operations**: Clear separation between operations that complete immediately and those that require callbacks

### 2. Operation Trait

The `Operation` trait defines how individual operations execute within different execution contexts.

```rust
pub trait Operation<T> {
    fn execute(&mut self, ctx: ExecutionContext) -> OperationResult<T>;
    fn can_complete_immediately(&self) -> bool;
    fn cancel(&mut self) -> Result<(), ErrorCode>;
}
```

**Key Design Decisions:**

- **Execution Context Awareness**: Operations receive context about their execution environment
- **Mutable Self**: Operations can maintain state across execution attempts
- **Cancellation Support**: All operations support cancellation for clean resource management
- **Immediate Completion Optimization**: Operations can indicate if they can complete without blocking

### 3. Execution Context

The `ExecutionContext` provides information about the current execution environment.

```rust
pub struct ExecutionContext {
    pub syscalls: Box<dyn GenericSyscalls>,
    pub max_retries: u32,
    pub timeout_ms: Option<u32>,
}
```

**Key Design Decisions:**

- **Syscall Provider**: Operations receive the syscall implementation to use
- **Retry Policy**: Built-in retry mechanism for transient failures
- **Timeout Support**: Optional timeout for long-running operations

### 4. Execution Adapters

The execution adapters provide different execution models for the same underlying operations.

#### SyncAdapter

```rust
pub trait SyncAdapter {
    fn execute_blocking<T, O: Operation<T>>(operation: O) -> Result<T, ErrorCode>;
    fn execute_with_retry<T, O: Operation<T>>(operation: O, max_retries: u32) -> Result<T, ErrorCode>;
    fn execute_with_timeout<T, O: Operation<T>>(operation: O, timeout_ms: u32) -> Result<T, ErrorCode>;
}
```

#### AsyncAdapter

```rust
pub trait AsyncAdapter {
    fn execute_async<T, O: Operation<T>>(operation: O) -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>>;
    fn execute_with_timeout<T, O: Operation<T>>(operation: O, timeout_ms: u32) -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>>;
}
```

#### PollAdapter

```rust
pub trait PollAdapter {
    fn try_execute<T, O: Operation<T>>(operation: &mut O) -> OperationResult<T>;
    fn try_execute_with_retries<T, O: Operation<T>>(operation: &mut O, max_retries: u32) -> OperationResult<T>;
}
```

## Trait Interoperation

### 1. Data Flow

The typical flow of data through the system:

```
Application Code
       │
       ▼
Execution Adapter (Sync/Async/Poll)
       │
       ▼
Operation::execute(ExecutionContext)
       │
       ▼
GenericSyscalls implementation
       │
       ▼
OS-specific adapter
       │
       ▼
Operating System
```

### 2. Handle Management

Resource handles flow through the system to ensure proper cleanup:

```rust
// 1. Create handle through GenericSyscalls
let handle = syscalls.setup_callback(driver_id, callback_id)?;

// 2. Use handle for operations
let status = syscalls.poll_callback(&handle);

// 3. Clean up handle
syscalls.cleanup_callback(handle)?;
```

### 3. Error Propagation

Errors propagate up through the layers:

```rust
// OS-specific error → ErrorCode → Operation Result → Execution Adapter → Application
```

## Execution Models

### Synchronous Execution

```rust
impl SyncAdapter for DefaultSyncAdapter {
    fn execute_blocking<T, O: Operation<T>>(mut operation: O) -> Result<T, ErrorCode> {
        let syscalls = get_syscalls_for_platform();
        let ctx = ExecutionContext {
            syscalls: Box::new(syscalls),
            max_retries: 3,
            timeout_ms: None,
        };
        
        loop {
            match operation.execute(ctx) {
                OperationResult::Ready(result) => return Ok(result),
                OperationResult::Pending => {
                    // Block until ready
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                OperationResult::WouldBlock => {
                    return Err(ErrorCode::Busy);
                }
            }
        }
    }
}
```

### Asynchronous Execution

```rust
impl AsyncAdapter for DefaultAsyncAdapter {
    fn execute_async<T, O: Operation<T>>(mut operation: O) -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>> {
        Box::pin(async move {
            let syscalls = get_syscalls_for_platform();
            let ctx = ExecutionContext {
                syscalls: Box::new(syscalls),
                max_retries: 3,
                timeout_ms: None,
            };
            
            loop {
                match operation.execute(ctx) {
                    OperationResult::Ready(result) => return Ok(result),
                    OperationResult::Pending => {
                        // Yield to async runtime
                        tokio::task::yield_now().await;
                    }
                    OperationResult::WouldBlock => {
                        return Err(ErrorCode::Busy);
                    }
                }
            }
        })
    }
}
```

### Polling Execution

```rust
impl PollAdapter for DefaultPollAdapter {
    fn try_execute<T, O: Operation<T>>(operation: &mut O) -> OperationResult<T> {
        let syscalls = get_syscalls_for_platform();
        let ctx = ExecutionContext {
            syscalls: Box::new(syscalls),
            max_retries: 0, // No retries in polling mode
            timeout_ms: None,
        };
        
        operation.execute(ctx)
    }
}
```

## OS Adapter Implementation

Each OS adapter implements `GenericSyscalls` to provide platform-specific behavior:

### Tock Adapter

```rust
pub struct TockSyscallsAdapter {
    // Tock-specific state
}

impl GenericSyscalls for TockSyscallsAdapter {
    fn command_immediate(&self, driver_id: u32, command_id: u32, arg0: u32, arg1: u32) -> Result<u32, ErrorCode> {
        // Use libtock-rs to make syscall
        libtock_platform::syscalls::command(driver_id, command_id, arg0, arg1)
            .map_err(|e| ErrorCode::from(e))
    }
    
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode> {
        // Use libtock-rs to subscribe to callback
        // Return handle that can be used to poll status
    }
    
    // ... other methods
}
```

### Hubris Adapter

```rust
pub struct HubrisSyscallsAdapter {
    // Hubris-specific state
}

impl GenericSyscalls for HubrisSyscallsAdapter {
    fn command_immediate(&self, driver_id: u32, command_id: u32, arg0: u32, arg1: u32) -> Result<u32, ErrorCode> {
        // Use Hubris syscall interface
        // Map to appropriate Hubris IPC send/receive operations
        // or direct syscalls where applicable
        unsafe {
            hubris::syscalls::send(driver_id, command_id, arg0, arg1)
                .map_err(|e| ErrorCode::from(e))
        }
    }
    
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode> {
        // Use Hubris IPC to set up message reception
        // Return handle that can be used to poll for messages
        let handle = hubris::ipc::setup_receive(driver_id, callback_id)?;
        Ok(CallbackHandle::new(driver_id, callback_id, handle as u64))
    }
    
    // ... other methods
}
```

### Linux Adapter

```rust
pub struct LinuxSyscallsAdapter {
    // Linux-specific state (epoll, eventfd, etc.)
}

impl GenericSyscalls for LinuxSyscallsAdapter {
    fn command_immediate(&self, driver_id: u32, command_id: u32, arg0: u32, arg1: u32) -> Result<u32, ErrorCode> {
        // Use Linux syscalls directly or through device files
        // Map driver_id to device file paths
        // Execute appropriate ioctl or read/write operations
    }
    
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode> {
        // Use epoll or io_uring to set up async notifications
        // Return handle that can be polled
    }
    
    // ... other methods
}
```

## Design Patterns

### 1. Builder Pattern for Operations

```rust
pub struct WriteOperation {
    data: Vec<u8>,
    driver_id: u32,
    callback_handle: Option<CallbackHandle>,
}

impl WriteOperation {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            driver_id: 0x01, // Console driver
            callback_handle: None,
        }
    }
}

impl Operation<usize> for WriteOperation {
    fn execute(&mut self, ctx: ExecutionContext) -> OperationResult<usize> {
        if self.callback_handle.is_none() {
            // First call - set up the operation
            match ctx.syscalls.write_buffer(&self.data) {
                Ok(handle) => {
                    self.callback_handle = Some(handle);
                    OperationResult::Pending
                }
                Err(e) => OperationResult::WouldBlock,
            }
        } else {
            // Subsequent calls - check status
            let handle = self.callback_handle.as_ref().unwrap();
            match ctx.syscalls.poll_callback(handle) {
                CallbackStatus::Completed(CallbackData::Number(bytes_written)) => {
                    OperationResult::Ready(bytes_written as usize)
                }
                CallbackStatus::Pending => OperationResult::Pending,
                CallbackStatus::Error(e) => OperationResult::WouldBlock,
            }
        }
    }
    
    fn can_complete_immediately(&self) -> bool {
        false // Write operations typically require callbacks
    }
    
    fn cancel(&mut self) -> Result<(), ErrorCode> {
        if let Some(handle) = self.callback_handle.take() {
            // Cancel the operation through syscalls
            // This would be implemented by the specific syscall provider
            Ok(())
        } else {
            Ok(())
        }
    }
}
```

### 2. Factory Pattern for Platform Selection

```rust
pub fn get_syscalls_for_platform() -> impl GenericSyscalls {
    #[cfg(feature = "tock")]
    return TockSyscallsAdapter::new();
    
    #[cfg(feature = "tock")]
    return TockSyscallsAdapter::new();
    
    #[cfg(all(feature = "linux", not(feature = "tock"), not(feature = "hubris")))]
    return LinuxSyscallsAdapter::new();
    
    #[cfg(all(feature = "hubris", not(feature = "tock"), not(feature = "linux")))]
    return HubrisSyscallsAdapter::new();
    
    #[cfg(all(not(feature = "tock"), not(feature = "linux"), not(feature = "hubris")))]
    {
        #[cfg(any(test, feature = "mock"))]
        return MockSyscalls::new();
        
        #[cfg(not(any(test, feature = "mock")))]
        compile_error!("No syscall implementation selected");
    }
}
```

## Hubris Integration

Hubris presents unique challenges and opportunities for syscall abstraction due to its distinctive architecture:

### Hubris Architecture Overview

Hubris is a small operating system designed for deeply-embedded systems with the following characteristics:

- **Task-based**: All user code runs in isolated tasks with memory protection
- **Minimal kernel**: Very small kernel with limited syscall interface
- **IPC-centric**: Most operations happen through Inter-Process Communication between tasks
- **Synchronous messaging**: Uses rendezvous-style message passing
- **Physically addressed**: Single address space with memory regions for each task

### Hubris Syscall Interface

Hubris provides a minimal set of syscalls:

```rust
// Core Hubris syscalls (from documentation)
const SEND: u32 = 0;      // Send message to task
const RECV: u32 = 1;      // Receive message from task
const REPLY: u32 = 2;     // Reply to message sender
const IRQ_CONTROL: u32 = 3; // Control interrupt mask
const REFRESH_TASK_ID: u32 = 4; // Get current task generation
const PANIC: u32 = 5;     // Crash current task
const GET_TIMER: u32 = 6; // Get timer value
const SET_TIMER: u32 = 7; // Set timer deadline
```

### Mapping to Generic Interface

The challenge is mapping Hubris's IPC-centric model to our generic syscall interface:

```rust
impl GenericSyscalls for HubrisSyscallsAdapter {
    fn command_immediate(&self, driver_id: u32, command_id: u32, arg0: u32, arg1: u32) -> Result<u32, ErrorCode> {
        // Map to Hubris SEND syscall
        // driver_id becomes target task ID
        // command_id and args become message payload
        
        let target_task = self.map_driver_to_task(driver_id)?;
        let message = HubrisMessage::new(command_id, arg0, arg1);
        
        match hubris_syscall::send(target_task, &message) {
            Ok(reply) => Ok(reply.as_u32()),
            Err(e) => Err(ErrorCode::from(e)),
        }
    }
    
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode> {
        // Set up to receive messages from a specific task
        // This maps to Hubris RECV syscall preparation
        
        let source_task = self.map_driver_to_task(driver_id)?;
        let handle_id = self.next_handle_id();
        
        // Store expectation for future polling
        self.pending_receives.insert(handle_id, PendingReceive {
            source_task,
            callback_id,
            status: ReceiveStatus::Pending,
        });
        
        Ok(CallbackHandle::new(driver_id, callback_id, handle_id))
    }
    
    fn poll_callback(&self, handle: &CallbackHandle) -> CallbackStatus {
        // Check if a message has arrived
        // This might involve a non-blocking RECV attempt
        
        if let Some(pending) = self.pending_receives.get(&handle.internal_id) {
            match hubris_syscall::recv_nonblocking(pending.source_task) {
                Ok(message) => {
                    CallbackStatus::Completed(CallbackData::Structured {
                        arg0: message.arg0,
                        arg1: message.arg1,
                        arg2: message.arg2,
                        user_data: message.user_data,
                    })
                }
                Err(HubrisError::WouldBlock) => CallbackStatus::Pending,
                Err(e) => CallbackStatus::Error(ErrorCode::from(e)),
            }
        } else {
            CallbackStatus::Error(ErrorCode::InvalidHandle)
        }
    }
}
```

### Hubris-Specific Challenges

1. **Task ID Mapping**: Hubris uses task IDs instead of driver IDs. The adapter needs to maintain a mapping between generic driver concepts and specific Hubris tasks.

2. **Message Size Limits**: Hubris messages are small (typically 4-8 words). Large data transfers require memory borrowing through the lease system.

3. **Synchronous Nature**: Hubris IPC is inherently synchronous, which needs careful handling in async contexts.

4. **Memory Borrowing**: For buffer operations, we need to use Hubris's memory lease system to allow tasks to access each other's memory safely.

### Buffer Management in Hubris

```rust
impl GenericSyscalls for HubrisSyscallsAdapter {
    fn setup_buffer(&self, driver_id: u32, buffer_id: u32, buffer: &[u8]) -> Result<BufferHandle, ErrorCode> {
        // Use Hubris memory lease system
        let target_task = self.map_driver_to_task(driver_id)?;
        
        // Create lease entry for the buffer
        let lease_id = hubris_syscall::create_lease(
            target_task,
            buffer.as_ptr() as u32,
            buffer.len() as u32,
            LeasePermissions::ReadOnly,
        )?;
        
        let handle_id = self.next_handle_id();
        self.active_leases.insert(handle_id, lease_id);
        
        Ok(BufferHandle::new(driver_id, buffer_id, handle_id))
    }
    
    fn cleanup_buffer(&self, handle: BufferHandle) -> Result<(), ErrorCode> {
        // Revoke the memory lease
        if let Some(lease_id) = self.active_leases.remove(&handle.internal_id) {
            hubris_syscall::revoke_lease(lease_id)?;
            Ok(())
        } else {
            Err(ErrorCode::InvalidHandle)
        }
    }
}
```

## Benefits of This Design

### 1. OS Independence
- Same application code works across all supported operating systems
- OS-specific details are encapsulated in adapters
- Easy to add support for new operating systems

### 2. Execution Model Flexibility
- Applications can choose sync, async, or polling execution
- Can switch execution models without changing core logic
- Adapters handle the complexity of different execution patterns

### 3. Resource Safety
- Handle-based resource management prevents leaks
- Automatic cleanup through Drop implementations
- Type-safe resource tracking

### 4. Zero-Cost Abstractions
- Traits compile to direct function calls
- No runtime overhead for the abstraction layer
- Monomorphization eliminates virtual dispatch

### 5. Testability
- Mock implementation for unit testing
- Easy to test different execution scenarios
- Isolated testing of individual components

## Future Extensions

### 1. Advanced Error Handling
```rust
pub trait RetryableOperation<T>: Operation<T> {
    fn should_retry(&self, error: &ErrorCode) -> bool;
    fn backoff_strategy(&self) -> BackoffStrategy;
}
```

### 3. Performance Monitoring
```rust
pub trait InstrumentedSyscalls: GenericSyscalls {
    fn get_metrics(&self) -> SyscallMetrics;
    fn reset_metrics(&self);
}
```

## Conclusion

This design provides a robust, flexible, and extensible foundation for OS-agnostic syscall abstraction. The trait-based architecture enables clean separation of concerns while maintaining zero-cost abstractions and type safety. The design supports multiple execution models and operating systems while providing a unified programming interface for applications.
