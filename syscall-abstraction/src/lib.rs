// Licensed under the Apache-2.0 license

//! # OS-Agnostic Syscall Abstraction
//!
//! This crate provides a unified interface for system calls that works across different
//! operating systems (Tock, Linux, Hubris) and execution models
//! (synchronous, asynchronous, polling).
//!
//! ## Key Features
//!
//! - **OS Independence**: Same API works on Tock, Linux, and Hubris
//! - **Execution Model Agnostic**: Support for sync, async, and polling execution
//! - **Zero-Cost Abstractions**: Compile-time optimizations eliminate runtime overhead
//! - **Resource Safety**: Automatic cleanup and handle-based resource management
//! - **Type Safety**: Rust's type system prevents common syscall errors
//!
//! ## Quick Start
//!
//! ```rust
//! use syscall_abstraction::prelude::*;
//! use syscall_abstraction::get_syscalls_for_platform;
//!
//! # fn example() -> Result<(), ErrorCode> {
//! // Create a syscall provider (OS-specific)
//! let syscalls = get_syscalls_for_platform();
//!
//! // Use the syscalls directly in different execution models:
//!
//! // Synchronous execution
//! let result = syscalls.command_immediate(0, 1, 1000, 0)?;
//!
//! // Asynchronous execution (if async feature enabled)
//! // let result = async_adapter.execute_async(operation).await?;
//!
//! // Polling execution
//! let callback_handle = syscalls.setup_callback(0, 0)?;
//! loop {
//!     match syscalls.poll_callback(&callback_handle) {
//!         CallbackStatus::Completed(data) => {
//!             println!("Operation completed: {:?}", data);
//!             break;
//!         }
//!         CallbackStatus::Pending => continue,
//!         CallbackStatus::Error(e) => {
//!             println!("Operation failed: {:?}", e);
//!             break;
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Architecture
//!
//! The abstraction is built in layers:
//!
//! 1. **Application Layer**: Your business logic
//! 2. **Execution Layer**: Execution model adapters (sync/async/poll)
//! 3. **Syscall Layer**: OS-specific implementations
//! 4. **Operating System**: Actual OS kernel
//!
//! ## Execution Models
//!
//! ### Synchronous
//! - Blocks until completion
//! - Simple error handling
//! - Compatible with existing sync code
//!
//! ### Asynchronous
//! - Non-blocking futures
//! - Works with any async runtime
//! - Waker-based notifications
//!
//! ### Polling
//! - Immediate return with status
//! - Caller controls timing
//! - Deterministic execution
//!
//! ## OS Support
//!
//! - **Tock**: Native support via libtock-platform
//! - **Linux**: epoll, io_uring, and standard syscalls
//! - **Hubris**: Task-based messaging and memory leases
//!
//! ## Examples
//!
//! See the `examples/` directory for complete usage examples.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]
#![deny(unsafe_code)]

// Re-export core types for convenience
pub use execution_context::{ExecutionContext, OperationResult};
pub use generic_syscalls::{GenericSyscalls, CallbackHandle, BufferHandle, CallbackStatus, CallbackData};
pub use callback_manager::{CallbackManager, SyscallsWithCallbacks};
pub use memory_allocator::{MemoryAllocator, SyscallsWithMemory};
pub use operation::Operation;
pub use error_handling::ErrorCode;

// Core modules
pub mod execution_context;
pub mod generic_syscalls;
pub mod callback_manager;
pub mod memory_allocator;
pub mod operation;
pub mod error_handling;

// Execution model adapters
// pub mod execution_bridge_traits;
// pub mod execution_bridge;
pub mod sync_adapter_traits;
pub mod sync_adapter;
#[cfg(feature = "async")]
pub mod async_adapter_traits;
#[cfg(feature = "async")]
pub mod async_adapter;
pub mod poll_adapter_traits;
pub mod poll_adapter;

// OS-specific adapters
#[cfg(feature = "tock")]
pub mod tock_adapter;
#[cfg(feature = "linux")]
pub mod linux_adapter;
#[cfg(feature = "hubris")]
pub mod hubris_adapter;

// Testing utilities
#[cfg(any(test, feature = "mock"))]
pub mod mock;

// Convenience re-exports
pub mod prelude {
    //! Common types and traits for typical usage
    
    pub use crate::execution_context::{ExecutionContext, OperationResult};
    pub use crate::generic_syscalls::{GenericSyscalls, CallbackHandle, BufferHandle, CallbackStatus, CallbackData};
    pub use crate::callback_manager::{CallbackManager, SyscallsWithCallbacks};
    pub use crate::memory_allocator::{MemoryAllocator, SyscallsWithMemory};
    pub use crate::operation::Operation;
    pub use crate::error_handling::ErrorCode;
    
    // Async support
    #[cfg(feature = "async")]
    pub use crate::async_adapter::{AsyncAdapter, AsyncOperationExt};
    
    // Platform-specific re-exports
    #[cfg(feature = "tock")]
    pub use crate::tock_adapter::TockSyscallsAdapter;
    #[cfg(feature = "linux")]
    pub use crate::linux_adapter::LinuxSyscallsAdapter;
    #[cfg(feature = "hubris")]
    pub use crate::hubris_adapter::HubrisSyscallsAdapter;
    
    // Mock for testing
    #[cfg(any(test, feature = "mock"))]
    pub use crate::mock::MockSyscalls;
}

/// Get the appropriate syscall provider for the current platform.
///
/// This function uses compile-time feature detection to return the correct
/// syscall implementation for the target platform.
///
/// # Examples
///
/// ```rust
/// use syscall_abstraction::get_syscalls_for_platform;
///
/// let syscalls = get_syscalls_for_platform();
/// // `syscalls` now contains the appropriate implementation for your platform
/// ```
pub fn get_syscalls_for_platform() -> impl GenericSyscalls {
    #[cfg(feature = "tock")]
    return tock_adapter::TockSyscallsAdapter::new();
    
    #[cfg(all(feature = "linux", not(feature = "tock"), not(feature = "hubris")))]
    return linux_adapter::LinuxSyscallsAdapter::new();
    
    #[cfg(all(feature = "hubris", not(feature = "tock"), not(feature = "linux")))]
    return hubris_adapter::HubrisSyscallsAdapter::new();
    
    #[cfg(all(not(feature = "tock"), not(feature = "linux"), not(feature = "hubris")))]
    {
        #[cfg(any(test, feature = "mock"))]
        return mock::MockSyscalls::new();
        
        #[cfg(not(any(test, feature = "mock")))]
        compile_error!("No syscall implementation selected. Enable one of: tock, linux, hubris");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_get_syscalls_for_platform() {
        let _syscalls = get_syscalls_for_platform();
        // Test that we can create a syscall provider
    }
}
