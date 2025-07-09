// Licensed under the Apache-2.0 license

//! Callback management trait for syscall abstraction.
//!
//! This trait provides callback management services that are separate from
//! the core syscall interface. This allows implementations to support
//! different callback mechanisms or opt out of callback functionality entirely.

use crate::error_handling::ErrorCode;
use crate::generic_syscalls::{CallbackHandle, CallbackStatus, CallbackData};

/// Callback management interface for syscall implementations.
///
/// This trait provides asynchronous callback management services that complement
/// the core syscall interface. It's separated from `GenericSyscalls` to allow
/// implementations to choose their callback strategy or opt out entirely.
///
/// # Design Rationale
///
/// The callback methods were separated from the core syscall interface because:
///
/// - **Varied callback mechanisms**: Different OS's have fundamentally different callback models
/// - **Optional functionality**: Not all syscall implementations need async callbacks
/// - **Complexity isolation**: Callback management is complex and deserves its own abstraction
/// - **Implementation flexibility**: Allows mixing different callback strategies
///
/// # Callback Models by OS
///
/// - **Tock**: Uses yield-based cooperative scheduling with upcalls
/// - **Linux**: Can use signals, epoll, io_uring, or other async mechanisms  
/// - **Linux**: Uses epoll, io_uring, and event-driven programming
///
/// # Examples
///
/// ```rust
/// use syscall_abstraction::prelude::*;
/// 
/// # fn example(syscalls: impl GenericSyscalls + CallbackManager) -> Result<(), ErrorCode> {
/// // Set up an async operation with callback
/// let callback_handle = syscalls.setup_callback(0x01, 0)?;
/// 
/// // Start the operation
/// syscalls.command_immediate(0x01, 1, 100, 0)?;
/// 
/// // Poll for completion
/// loop {
///     match syscalls.poll_callback(&callback_handle) {
///         CallbackStatus::Completed(data) => {
///             println!("Operation completed: {:?}", data);
///             break;
///         }
///         CallbackStatus::Pending => continue,
///         CallbackStatus::Error(e) => return Err(e),
///     }
/// }
/// 
/// // Clean up
/// syscalls.cleanup_callback(callback_handle)?;
/// # Ok(())
/// # }
/// ```
pub trait CallbackManager {
    /// Setup a callback for receiving asynchronous notifications.
    ///
    /// This registers a callback that will be invoked when the driver
    /// completes an asynchronous operation. The callback can be polled
    /// or waited on depending on the execution context.
    ///
    /// # Parameters
    ///
    /// - `driver_id`: ID of the driver to register the callback with
    /// - `callback_id`: ID of the callback slot within the driver
    ///
    /// # Returns
    ///
    /// A handle to the callback registration that can be used for polling and cleanup.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(callbacks: impl CallbackManager) -> Result<(), ErrorCode> {
    /// // Register callback for timer expiration
    /// let callback_handle = callbacks.setup_callback(0x01, 0)?;
    /// 
    /// // The callback can now be polled or waited on
    /// # Ok(())
    /// # }
    /// ```
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode>;

    /// Poll the status of a registered callback.
    ///
    /// This checks if a callback has fired without blocking. It can be
    /// called multiple times and will return the same result until the
    /// callback is cleaned up.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the callback registration
    ///
    /// # Returns
    ///
    /// The current status of the callback.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(callbacks: impl CallbackManager) -> Result<(), ErrorCode> {
    /// let handle = callbacks.setup_callback(0x01, 0)?;
    /// 
    /// match callbacks.poll_callback(&handle) {
    ///     CallbackStatus::Completed(data) => {
    ///         println!("Callback completed with: {:?}", data);
    ///     }
    ///     CallbackStatus::Pending => {
    ///         println!("Still waiting...");
    ///     }
    ///     CallbackStatus::Error(e) => {
    ///         println!("Callback failed: {:?}", e);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    fn poll_callback(&self, handle: &CallbackHandle) -> CallbackStatus;

    /// Wait for a callback to complete (blocking).
    ///
    /// This will block the current thread until the callback fires or
    /// an error occurs. It should only be used in synchronous execution
    /// contexts.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the callback registration
    ///
    /// # Returns
    ///
    /// The callback data when it completes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(callbacks: impl CallbackManager) -> Result<(), ErrorCode> {
    /// let callback_handle = callbacks.setup_callback(0x01, 0)?;
    /// 
    /// // Wait for completion (blocks)
    /// let data = callbacks.wait_callback(&callback_handle)?;
    /// println!("Operation completed with data: {:?}", data);
    /// 
    /// callbacks.cleanup_callback(callback_handle)?;
    /// # Ok(())
    /// # }
    /// ```
    fn wait_callback(&self, handle: &CallbackHandle) -> Result<CallbackData, ErrorCode>;

    /// Clean up a callback registration.
    ///
    /// This unregisters the callback and releases any associated resources.
    /// The handle becomes invalid after this call.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the callback registration
    ///
    /// # Returns
    ///
    /// Result of the cleanup operation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(callbacks: impl CallbackManager) -> Result<(), ErrorCode> {
    /// let handle = callbacks.setup_callback(0x01, 0)?;
    /// 
    /// // Use the callback...
    /// 
    /// // Clean up when done
    /// callbacks.cleanup_callback(handle)?;
    /// # Ok(())
    /// # }
    /// ```
    fn cleanup_callback(&self, handle: CallbackHandle) -> Result<(), ErrorCode>;

    /// Check the status of a callback (alias for poll_callback).
    ///
    /// This is a convenience method that returns the callback status
    /// wrapped in a Result for consistency with other APIs.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the callback registration
    ///
    /// # Returns
    ///
    /// The current status of the callback wrapped in a Result.
    fn check_callback(&self, handle: CallbackHandle) -> Result<CallbackStatus, ErrorCode> {
        Ok(self.poll_callback(&handle))
    }

    /// Cancel a callback (alias for cleanup_callback).
    ///
    /// This is a convenience method that provides a more semantic name
    /// for canceling an ongoing callback operation.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the callback registration to cancel
    ///
    /// # Returns
    ///
    /// Result of the cancellation operation.
    fn cancel_callback(&self, handle: CallbackHandle) -> Result<(), ErrorCode> {
        self.cleanup_callback(handle)
    }
}

/// Extension trait for implementations that support both syscalls and callbacks.
///
/// This trait is automatically implemented for types that implement both
/// `GenericSyscalls` and `CallbackManager`.
pub trait SyscallsWithCallbacks: crate::generic_syscalls::GenericSyscalls + CallbackManager {}

impl<T> SyscallsWithCallbacks for T where T: crate::generic_syscalls::GenericSyscalls + CallbackManager {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic_syscalls::CallbackData;

    struct MockCallbackManager;

    impl CallbackManager for MockCallbackManager {
        fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode> {
            Ok(CallbackHandle::new(driver_id, callback_id, 12345))
        }

        fn poll_callback(&self, _handle: &CallbackHandle) -> CallbackStatus {
            CallbackStatus::Pending
        }

        fn wait_callback(&self, _handle: &CallbackHandle) -> Result<CallbackData, ErrorCode> {
            Ok(CallbackData::Number(42))
        }

        fn cleanup_callback(&self, _handle: CallbackHandle) -> Result<(), ErrorCode> {
            Ok(())
        }
    }

    #[test]
    fn test_callback_manager_setup() {
        let manager = MockCallbackManager;
        let handle = manager.setup_callback(1, 2).unwrap();
        assert_eq!(handle.driver_id, 1);
        assert_eq!(handle.callback_id, 2);
    }

    #[test]
    fn test_callback_manager_poll() {
        let manager = MockCallbackManager;
        let handle = CallbackHandle::new(1, 2, 12345);
        let status = manager.poll_callback(&handle);
        assert!(matches!(status, CallbackStatus::Pending));
    }

    #[test]
    fn test_callback_manager_wait() {
        let manager = MockCallbackManager;
        let handle = CallbackHandle::new(1, 2, 12345);
        let data = manager.wait_callback(&handle).unwrap();
        assert!(matches!(data, CallbackData::Number(42)));
    }

    #[test]
    fn test_callback_manager_cleanup() {
        let manager = MockCallbackManager;
        let handle = CallbackHandle::new(1, 2, 12345);
        manager.cleanup_callback(handle).unwrap();
    }

    #[test]
    fn test_convenience_methods() {
        let manager = MockCallbackManager;
        let handle = CallbackHandle::new(1, 2, 12345);
        
        // Test check_callback
        let status = manager.check_callback(handle).unwrap();
        assert!(matches!(status, CallbackStatus::Pending));
        
        // Test cancel_callback
        manager.cancel_callback(handle).unwrap();
    }
}
