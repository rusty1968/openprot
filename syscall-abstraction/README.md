# OS-Agnostic Syscall Abstraction for Embedded Systems

**Version:** 1.0  
**Date:** July 7, 2025  
**Author:** rusty1968  

An OS-agnostic syscall abstraction layer designed for embedded systems that enables the same code to work seamlessly across different embedded operating systems (Tock, Hubris) and execution models (synchronous, asynchronous, polling). Linux support is provided for development and prototyping only.

## Overview

This crate provides a unified interface for system calls that decouples embedded application code from specific operating system models and execution patterns. The design supports zero-allocation environments and enables gradual migration from OS-specific code while maintaining zero-cost abstractions.

### Key Features

- **Embedded-First Design**: Optimized for microcontrollers and embedded systems
- **Zero-Allocation Support**: Compatible with no-heap environments (Hubris)
- **OS Independence**: Same API works on Tock, Hubris, and Linux (prototyping only)
- **Execution Model Agnostic**: Support for sync, async, and polling execution
- **Zero-Cost Abstractions**: Compile-time optimizations eliminate runtime overhead
- **Type Safety**: Rust's type system prevents common syscall errors
- **Resource Safety**: Automatic cleanup and handle-based resource management

## Quick Start

```rust
use syscall_abstraction::prelude::*;

// Get platform-specific syscalls
let syscalls = get_syscalls_for_platform();

// Create operations (execution-model agnostic)
let operation = /* some operation */;

// Use in different execution models:

// Synchronous execution
let result = SyncAdapter::execute_blocking(operation)?;

// Asynchronous execution (when async feature enabled)
let result = AsyncAdapter::execute_async(operation).await?;

// Polling execution
let result = PollAdapter::try_execute(&mut operation);
```

## Architecture

The abstraction is built in layers:

1. **Application Layer**: Your business logic
2. **Execution Layer**: Execution model adapters (sync/async/poll)  
3. **Syscall Layer**: OS-specific implementations
4. **Operating System**: Actual OS kernel

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
├─────────────────────────────────────────────────────────────┤
│              Execution Adapters (Traits)                    │
│    SyncAdapter │ AsyncAdapter │ PollAdapter                 │
├─────────────────────────────────────────────────────────────┤
│                  Operation Trait                            │
│           execute(ExecutionContext) → Result                │
├─────────────────────────────────────────────────────────────┤
│                 GenericSyscalls Trait                       │
│  command_immediate() │ setup_callback() │ poll_callback()   │
├─────────────────────────────────────────────────────────────┤
│                    OS Adapters                              │
│       TockAdapter │ LinuxAdapter │ HubrisAdapter            │
├─────────────────────────────────────────────────────────────┤
│                 Operating System                            │
│          Tock │ Linux │ Hubris                              │
└─────────────────────────────────────────────────────────────┘
```

## Core Traits

### GenericSyscalls

The main syscall interface that abstracts OS-specific operations:

```rust
pub trait GenericSyscalls: Clone {
    fn command_immediate(&self, driver_id: u32, command_id: u32, arg0: u32, arg1: u32) -> Result<u32, ErrorCode>;
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode>;
    fn poll_callback(&self, handle: &CallbackHandle) -> CallbackStatus;
    // ... more methods
}
```

### Operation

Defines how operations execute in different contexts:

```rust
pub trait Operation<T> {
    fn execute(&mut self, ctx: ExecutionContext) -> OperationResult<T>;
    fn can_complete_immediately(&self) -> bool;
    fn cancel(&mut self) -> Result<(), ErrorCode>;
    // ... more methods
}
```

### ExecutionContext

The `ExecutionContext` enum represents the execution environment where an operation runs. It provides crucial information about how the operation should behave based on the execution model being used:

```rust
pub enum ExecutionContext {
    /// Synchronous execution - can block indefinitely
    Sync,
    /// Asynchronous execution with waker for notifications
    Async(Waker),
    /// Polling execution - must return immediately
    Poll,
}
```

**Key Design Principles:**

- **Context-Aware Execution**: Operations adapt their behavior based on the execution context
- **Zero-Cost Abstraction**: Context information is compile-time optimized away
- **Consistent Interface**: Same operation can run in any execution model

**Execution Context Types:**

1. **`ExecutionContext::Sync`**
   - Operations can block indefinitely waiting for completion
   - Used by `SyncAdapter` for traditional blocking I/O
   - Simplest model - operation blocks until result is available
   - Suitable for single-threaded or dedicated-thread scenarios

2. **`ExecutionContext::Async(Waker)`**
   - Operations cannot block - must return immediately if not ready
   - Includes a `Waker` for async runtime integration
   - Operations should wake the task when ready to make progress
   - Used by `AsyncAdapter` for futures-based async programming
   - Compatible with tokio, async-std, and other async runtimes

3. **`ExecutionContext::Poll`**
   - Operations must return immediately with current status
   - No blocking or waiting mechanisms available
   - Caller is responsible for retry logic and timing
   - Used by `PollAdapter` for deterministic execution
   - Ideal for real-time systems and event loops

**OperationResult:**

Operations return `OperationResult<T>` to indicate their status:

```rust
pub enum OperationResult<T> {
    /// Operation completed immediately with result
    Ready(Result<T, ErrorCode>),
    /// Operation is pending, will complete later
    Pending,
    /// Operation would block (only in polling context)
    WouldBlock,
}
```

**Example Usage:**

```rust
impl<T> Operation<T> for MyOperation {
    fn execute(&mut self, ctx: ExecutionContext) -> OperationResult<T> {
        match ctx {
            ExecutionContext::Sync => {
                // Can block - wait for completion
                let result = self.blocking_operation();
                OperationResult::Ready(result)
            }
            ExecutionContext::Async(waker) => {
                // Cannot block - check if ready
                if self.is_ready() {
                    OperationResult::Ready(self.get_result())
                } else {
                    // Store waker for later notification
                    self.set_waker(waker);
                    OperationResult::Pending
                }
            }
            ExecutionContext::Poll => {
                // Must return immediately
                if self.is_ready() {
                    OperationResult::Ready(self.get_result())
                } else {
                    OperationResult::WouldBlock
                }
            }
        }
    }
}
```

### Execution Adapters

Traits for different execution models:

```rust
// Synchronous execution
pub trait SyncAdapter {
    fn execute_blocking<T, O: Operation<T>>(operation: O) -> Result<T, ErrorCode>;
    fn execute_with_retry<T, O: Operation<T>>(operation: O, max_retries: u32) -> Result<T, ErrorCode>;
    // ... more methods
}

// Asynchronous execution
pub trait AsyncAdapter {
    fn execute_async<T, O: Operation<T>>(operation: O) -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>>;
    // ... more methods
}

// Polling execution
pub trait PollAdapter {
    fn try_execute<T, O: Operation<T>>(operation: &mut O) -> OperationResult<T>;
    fn try_execute_with_retries<T, O: Operation<T>>(operation: &mut O, max_retries: u32) -> OperationResult<T>;
    // ... more methods
}
```

## Execution Models

### Synchronous
- Blocks until completion
- Simple error handling
- Compatible with existing sync code

```rust
let result = SyncAdapter::execute_blocking(operation)?;
```

### Asynchronous  
- Non-blocking futures
- Works with any async runtime
- Waker-based notifications

```rust
let result = AsyncAdapter::execute_async(operation).await?;
```

### Polling
- Immediate return with status
- Caller controls timing
- Deterministic execution

```rust
match PollAdapter::try_execute(&mut operation) {
    OperationResult::Ready(result) => { /* completed */ },
    OperationResult::Pending => { /* still running */ },
    OperationResult::WouldBlock => { /* try again later */ },
}
```

## OS Support

### Primary Embedded Targets
- **Tock**: Native support via libtock-platform for microcontrollers
- **Hubris**: Task-based messaging and memory leases for embedded systems

### Development Platform
- **Linux**: epoll, io_uring, and standard syscalls (prototyping and testing only)

## Features

- `std`: Enable standard library support (default)
- `async`: Enable async execution adapter
- `mock`: Enable mock implementation for testing
- `tock`: Enable Tock OS adapter
- `linux`: Enable Linux adapter
- `hubris`: Enable Hubris OS adapter

## Current Status

This crate currently provides:
- ✅ Core trait definitions
- ✅ Basic mock implementation for testing
- ✅ Error handling framework
- ✅ Execution context management
- 🚧 OS-specific adapters (to be implemented)
- 🚧 High-level service implementations (to be added)

## Examples

See the `examples/` directory for complete usage examples.

## Contributing

This crate provides a generic syscall abstraction library. Please see the documentation for contribution guidelines.

## License

Licensed under the Apache License, Version 2.0.
