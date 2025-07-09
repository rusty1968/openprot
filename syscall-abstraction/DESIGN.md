# Syscall Abstraction Design Document

**Version:** 1.0  
**Date:** July 7, 2025  
**Author:** rusty1968

## Overview

This document describes the design and architecture of the OS-agnostic syscall abstraction library, focusing on how the various traits and components interoperate to provide a unified interface across different embedded operating systems and execution models.

**Target Systems**: This library is designed primarily for embedded systems running Tock and Hubris. Linux support is provided solely for prototyping and development purposes, not for production use.

**Design Constraints**: The library must support zero-allocation environments (Hubris), stateless callback models (Tock), and provide efficient abstractions for resource-constrained embedded systems.

## Architecture Overview

The syscall abstraction is built as a layered architecture with clear separation of concerns:

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│            (Embedded Applications)                          │
├─────────────────────────────────────────────────────────────┤
│              Execution Adapters                             │
│    SyncAdapter │ AsyncAdapter │ PollAdapter                 │
├─────────────────────────────────────────────────────────────┤
│                  Operation Trait                            │
│           execute(ExecutionContext) → Result                │
├─────────────────────────────────────────────────────────────┤
│                  SysCall Abstractions                       │
│ GenericSyscalls │ MemoryAllocator │ CallbackManager         │
│  (Core Syscalls)│  (Dynamic Alloc) │ (Async Callbacks)      │
├─────────────────────────────────────────────────────────────┤
│                    OS Adapters                              │
│  TockAdapter │ HubrisAdapter │ LinuxAdapter*                │
│  (Core+CB)   │   (Core+CB)   │ (Prototyping)                │
├─────────────────────────────────────────────────────────────┤
│              Embedded Operating Systems                     │
│    Tock │ Hubris │ Linux* (Development Only)               │
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
    
    // Generic I/O convenience methods (work with any buffered driver)
    fn write_buffer(&self, data: &[u8]) -> Result<CallbackHandle, ErrorCode>;
    fn read_buffer(&self, handle: BufferHandle, size: usize) -> Result<CallbackHandle, ErrorCode>;
    fn input_available(&self) -> Result<bool, ErrorCode>;
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

The `ExecutionContext` is a fundamental abstraction that enables the same operation to work across different execution models. This design solves the core challenge of writing code that can work in blocking, non-blocking, and polling environments without sacrificing performance or correctness.

#### Conceptual Foundation

The key insight behind `ExecutionContext` is that **the same logical operation has fundamentally different constraints depending on the execution environment**:

- **Blocking contexts** can wait indefinitely for I/O completion
- **Async contexts** must yield control when waiting, using cooperative scheduling
- **Polling contexts** must return immediately, leaving retry logic to the caller

Rather than forcing operations to choose one execution model, `ExecutionContext` allows operations to **adapt their behavior** based on the runtime environment.

#### Design Philosophy

**Separation of Concerns**: The operation logic is separate from execution strategy. An operation describes *what* to do, while ExecutionContext describes *how* to do it in the current environment.

**Zero-Cost Abstraction**: The abstraction compiles away - there's no runtime overhead for supporting multiple execution models. Each execution path is optimized independently.

**Composability**: Operations can be combined and nested while preserving execution context awareness throughout the call stack.

#### ExecutionContext Definition

```rust
#[derive(Debug)]
pub enum ExecutionContext {
    /// Synchronous execution - blocks until completion
    Sync,
    /// Asynchronous execution with a waker for notifications  
    Async(Waker),
    /// Polling execution - returns immediately with status
    Poll,
}
```

#### Execution Context Semantics

**`ExecutionContext::Sync`**
- **Constraint**: Operations may block indefinitely
- **Expectation**: Complete the operation or return an error
- **Use Case**: Traditional blocking I/O, single-threaded embedded firmware
- **Example**: Sensor initialization, configuration writes to peripherals

**`ExecutionContext::Async(Waker)`**
- **Constraint**: Operations must not block the thread
- **Expectation**: Return `Pending` if not ready, wake the task when ready
- **Use Case**: Async/await programming, cooperative multitasking
- **Example**: Concurrent sensor readings, parallel device communications

**`ExecutionContext::Poll`**
- **Constraint**: Operations must return immediately
- **Expectation**: Return current status without waiting
- **Use Case**: Real-time systems, event loops, embedded systems
- **Example**: Hardware polling in firmware, control systems

#### OperationResult

Operations return `OperationResult<T>` to communicate their status:

```rust
#[derive(Debug)]
pub enum OperationResult<T> {
    /// Operation completed immediately with result
    Ready(Result<T, ErrorCode>),
    /// Operation is pending, will complete later via callback
    Pending,
    /// Operation would block in polling context
    WouldBlock,
}
```

**State Meanings**:
- **`Ready(result)`**: Operation completed, result available immediately
- **`Pending`**: Operation started but needs time to complete (async context)
- **`WouldBlock`**: Operation cannot proceed without blocking (polling context)

#### Design Benefits

**1. Execution Model Independence**: Operations work in any context without modification

**2. Performance Optimization**: Each execution path can be optimized for its specific constraints

**3. Composability**: Operations can call other operations while preserving context

**4. Testing Flexibility**: Same operation can be tested in sync mode for simplicity and async mode for correctness

**5. Gradual Migration**: Applications can migrate between execution models incrementally

#### Implementation Pattern

```rust
impl Operation<Data> for MyOperation {
    fn execute(&mut self, ctx: ExecutionContext) -> OperationResult<Data> {
        match ctx {
            ExecutionContext::Sync => {
                // Safe to block - wait for completion
                let result = self.perform_blocking_operation();
                OperationResult::Ready(result)
            }
            ExecutionContext::Async(waker) => {
                // Check if ready without blocking
                if let Some(result) = self.try_get_result() {
                    OperationResult::Ready(result)
                } else {
                    // Store waker for later notification
                    self.register_waker(waker);
                    OperationResult::Pending
                }
            }
            ExecutionContext::Poll => {
                // Must return immediately
                if let Some(result) = self.check_immediate_result() {
                    OperationResult::Ready(result)
                } else {
                    OperationResult::WouldBlock
                }
            }
        }
    }
}
```

**Key Design Decisions:**

- **Context-Aware Execution**: Operations adapt their behavior based on the execution environment
- **Waker Integration**: Direct support for async runtime integration via `Waker`
- **Immediate Status**: Clear indication when operations cannot proceed without blocking
- **Composability**: Context flows through operation hierarchies naturally

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

### 1. Data Flow with ExecutionContext

The ExecutionContext is a crucial component that flows through the system, enabling context-aware execution at every level:

```
Application Code
       │
       ▼
Execution Adapter (creates ExecutionContext)
       │
       ▼
Operation::execute(ExecutionContext) ← Context flows here
       │
       ▼
GenericSyscalls implementation (context-aware)
       │
       ▼
OS-specific adapter
       │
       ▼
Operating System
```

**Context Flow Example:**
1. **SyncAdapter** creates `ExecutionContext::Sync`
2. **Operation** receives context and adapts behavior (can block)
3. **GenericSyscalls** implementations can optionally use context for optimization
4. **Result flows back** with appropriate `OperationResult` variant

**Context Propagation:**
- ExecutionContext is **immutable** and **Copy** - no ownership transfer
- Operations can **clone** context when calling sub-operations
- Context **never changes** during an operation execution tree
- Each execution model creates its **own context type**

### 2. ExecutionContext Creation by Adapters

Each execution adapter creates the appropriate context:

```rust
// SyncAdapter implementation
impl SyncAdapter for DefaultSyncAdapter {
    fn execute_blocking<T, O: Operation<T>>(mut operation: O) -> Result<T, ErrorCode> {
        let ctx = ExecutionContext::Sync;  // Create sync context
        
        loop {
            match operation.execute(ctx) {  // Pass context to operation
                OperationResult::Ready(result) => return result,
                OperationResult::Pending => {
                    // In sync context, pending means "try again"
                    std::thread::sleep(Duration::from_millis(1));
                }
                OperationResult::WouldBlock => {
                    return Err(ErrorCode::Busy);
                }
            }
        }
    }
}

// AsyncAdapter implementation  
impl AsyncAdapter for DefaultAsyncAdapter {
    fn execute_async<T, O: Operation<T>>(mut operation: O) 
        -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>> 
    {
        Box::pin(async move {
            // Context includes waker for notifications
            let waker = futures::task::current().waker();
            let ctx = ExecutionContext::Async(waker);
            
            loop {
                match operation.execute(ctx.clone()) {
                    OperationResult::Ready(result) => return result,
                    OperationResult::Pending => {
                        // Yield to async runtime
                        futures::task::yield_now().await;
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

### 3. Data Flow

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

### 4. Handle Management

Resource handles flow through the system to ensure proper cleanup:

```rust
// 1. Create handle through GenericSyscalls
let handle = syscalls.setup_callback(driver_id, callback_id)?;

// 2. Use handle for operations
let status = syscalls.poll_callback(&handle);

// 3. Clean up handle
syscalls.cleanup_callback(handle)?;
```

### 5. Error Propagation

Errors propagate up through the layers:

```rust
// OS-specific error → ErrorCode → Operation Result → Execution Adapter → Application
```

## Execution Models

### Synchronous Execution

```rust
impl SyncAdapter for DefaultSyncAdapter {
    fn execute_blocking<T, O: Operation<T>>(mut operation: O) -> Result<T, ErrorCode> {
        let ctx = ExecutionContext::Sync;
        
        loop {
            match operation.execute(ctx) {
                OperationResult::Ready(result) => return result,
                OperationResult::Pending => {
                    // Block until ready - safe in sync context
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
            let waker = futures::task::current().waker();
            let ctx = ExecutionContext::Async(waker);
            
            loop {
                match operation.execute(ctx.clone()) {
                    OperationResult::Ready(result) => return result,
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

## Design Principles

This syscall abstraction library is designed specifically for embedded systems with the following constraints and principles:

### Embedded-First Design
- **Primary Targets**: Tock microkernel on microcontrollers, Hubris microkernel for embedded systems
- **Resource Constraints**: Support for systems with limited memory, no heap allocation (Hubris)
- **Real-Time Requirements**: Predictable execution times, no blocking in critical paths
- **Hardware Interaction**: Direct access to peripherals, sensors, and embedded hardware

### Development Support
- **Linux Adapter**: Provided for development, testing, and prototyping only
- **Not for Production**: Linux support is not intended for production embedded systems
- **Testing Framework**: Enables unit testing and development workflows on standard machines

### Design Constraints
- **Zero Allocation**: Must work in no-heap environments (Hubris)
- **Deterministic**: Predictable memory usage and execution times
- **Minimal Dependencies**: Reduced attack surface and resource usage
- **Type Safety**: Compile-time guarantees for embedded safety requirements

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

## Trait Separation Architecture

The syscall abstraction uses a **modular trait design** to provide flexibility and support different operating system capabilities and execution environments. This separation allows implementations to provide only the functionality that makes sense for their platform.

#### Core Trait Separation

The abstraction is divided into three main trait categories:

1. **Core Syscalls** (`GenericSyscalls`) - Essential syscall operations available on all platforms
2. **Memory Allocation** (`MemoryAllocator`) - Dynamic memory allocation services  
3. **Callback Management** (`CallbackManager`) - Asynchronous callback handling

```rust
// Core syscall interface - required for all implementations
pub trait GenericSyscalls: Clone {
    fn command_immediate(&self, driver_id: u32, command_id: u32, arg0: u32, arg1: u32) -> Result<u32, ErrorCode>;
    fn setup_buffer(&self, driver_id: u32, buffer_id: u32, buffer: &[u8]) -> Result<BufferHandle, ErrorCode>;
    // ... other essential methods
}

// Optional memory allocation - not available in no_alloc environments
pub trait MemoryAllocator {
    fn allocate_buffer(&self, size: usize) -> Result<BufferHandle, ErrorCode>;
    fn free_buffer(&self, handle: BufferHandle) -> Result<(), ErrorCode>;
    fn read_from_buffer(&self, handle: BufferHandle) -> Result<Vec<u8>, ErrorCode>;
}

// Optional callback management - platform-specific async handling
pub trait CallbackManager {
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode>;
    fn poll_callback(&self, handle: &CallbackHandle) -> CallbackStatus;
    fn wait_callback(&self, handle: &CallbackHandle) -> Result<CallbackData, ErrorCode>;
    fn cleanup_callback(&self, handle: CallbackHandle) -> Result<(), ErrorCode>;
}

// Combined interface for full-featured implementations
pub trait SyscallsWithMemory: GenericSyscalls + MemoryAllocator {}
```

#### Design Rationale

**1. No-Alloc Compatibility**
- Embedded systems like Tock don't support dynamic allocation
- `GenericSyscalls` can be implemented without heap allocation
- `MemoryAllocator` is optional and requires `std` or `alloc`

**2. Platform-Specific Capabilities**
- Different operating systems have fundamentally different callback models
- Some platforms may not support certain types of asynchronous operations
- Traits allow implementations to provide only what makes sense for their platform

**3. Separation of Concerns**
- Memory management is conceptually separate from syscall execution
- Callback handling has different complexity and requirements than immediate operations
- Each trait can be tested and reasoned about independently

**4. Incremental Implementation**
- Implementations can start with just `GenericSyscalls`
- Additional functionality can be added by implementing more traits
- Applications can depend on only the traits they need

#### Platform Implementation Patterns

**Tock (No-Alloc Environment)**
```rust
impl GenericSyscalls for TockSyscallsAdapter {
    // Implements core syscalls using libtock-rs
    // Uses only static buffers and stack allocation
}

impl CallbackManager for TockSyscallsAdapter {
    // Uses Tock's yield-based upcall mechanism
    // No dynamic allocation required
}

// Note: TockSyscallsAdapter does NOT implement MemoryAllocator
// This prevents usage of heap-allocating methods in no_alloc environments
```

**Linux (Full-Featured Environment)**
```rust
impl GenericSyscalls for LinuxSyscallsAdapter {
    // Implements core syscalls using Linux system calls
    // Maps driver concepts to device files and ioctls
}

impl MemoryAllocator for LinuxSyscallsAdapter {
    // Provides dynamic allocation using Linux memory management
    // Uses mmap, malloc, or other allocation strategies
}

impl CallbackManager for LinuxSyscallsAdapter {
    // Uses epoll, io_uring, or signals for async notifications
    // Full async/await integration
}

// LinuxSyscallsAdapter automatically implements SyscallsWithMemory
```

**Hubris (Task-Based Environment)**
```rust
impl GenericSyscalls for HubrisSyscallsAdapter {
    // Uses Hubris IPC and task messaging
    // Maps driver operations to task communications
}

impl CallbackManager for HubrisSyscallsAdapter {
    // Uses Hubris message passing for async operations
    // Task-based notification system
}

// May or may not implement MemoryAllocator depending on Hubris capabilities
```

#### Usage Patterns

**1. Core Functionality Only**
```rust
fn basic_operation(syscalls: &impl GenericSyscalls) -> Result<(), ErrorCode> {
    // Works with any implementation - Tock, Linux, Hubris
    let result = syscalls.command_immediate(0x01, 1, 100, 0)?;
    
    // Use static buffers (no allocation)
    let static_data = b"Hello, World!";
    let handle = syscalls.setup_buffer(0x01, 0, static_data)?;
    
    Ok(())
}
```

**2. With Memory Allocation**
```rust
fn allocating_operation(syscalls: &impl SyscallsWithMemory) -> Result<Vec<u8>, ErrorCode> {
    // Only works with implementations that support allocation
    let buffer_handle = syscalls.allocate_buffer(1024)?;
    
    // Do some operation...
    
    let data = syscalls.read_from_buffer(buffer_handle)?;
    syscalls.free_buffer(buffer_handle)?;
    
    Ok(data)
}
```

**3. Async Operation with Callbacks**
```rust
fn async_operation(syscalls: &impl GenericSyscalls + CallbackManager) -> Result<CallbackData, ErrorCode> {
    // Set up async operation
    let callback_handle = syscalls.setup_callback(0x01, 0)?;
    
    // Start the operation
    syscalls.command_immediate(0x01, 1, 0, 0)?;
    
    // Wait for completion
    let result = syscalls.wait_callback(&callback_handle)?;
    syscalls.cleanup_callback(callback_handle)?;
    
    Ok(result)
}
```

#### Benefits of Trait Separation

**1. Compile-Time Guarantees**
- Applications that don't use allocation cannot accidentally call allocation methods
- Type system prevents usage of unavailable functionality
- Clear documentation of platform capabilities

**2. Reduced Binary Size**
- No-alloc implementations don't include allocation code
- Unused trait methods are optimized away
- Minimal runtime overhead

**3. Platform Flexibility**
- Each platform can implement only relevant traits
- Gradual feature adoption is possible
- Easy to add new capabilities as traits

**4. Testing and Mocking**
- Each trait can be mocked independently
- Test different functionality combinations
- Isolate testing concerns

### 4. Execution Context
