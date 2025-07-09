// Licensed under the Apache-2.0 license

//! Generic syscall trait and related types.

use crate::error_handling::ErrorCode;

/// Unique identifier for a callback registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallbackHandle {
    /// Driver ID that registered the callback
    pub driver_id: u32,
    /// Callback ID within the driver
    pub callback_id: u32,
    /// Internal unique ID for tracking
    pub(crate) internal_id: u64,
}

impl CallbackHandle {
    /// Create a new callback handle.
    pub fn new(driver_id: u32, callback_id: u32, internal_id: u64) -> Self {
        Self {
            driver_id,
            callback_id,
            internal_id,
        }
    }
    
    /// Get the internal ID for this callback handle.
    pub fn internal_id(&self) -> u64 {
        self.internal_id
    }
}

/// Unique identifier for a buffer allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferHandle {
    /// Driver ID that allocated the buffer
    pub driver_id: u32,
    /// Buffer ID within the driver
    pub buffer_id: u32,
    /// Internal unique ID for tracking
    pub(crate) internal_id: u64,
}

impl BufferHandle {
    /// Create a new buffer handle.
    pub fn new(driver_id: u32, buffer_id: u32, internal_id: u64) -> Self {
        Self {
            driver_id,
            buffer_id,
            internal_id,
        }
    }
    
    /// Get the internal ID for this buffer handle.
    pub fn internal_id(&self) -> u64 {
        self.internal_id
    }
}

/// Status of a registered callback.
#[derive(Debug, Clone, PartialEq)]
pub enum CallbackStatus {
    /// Callback not yet fired
    Pending,
    /// Callback completed with data
    Completed(CallbackData),
    /// Callback completed with error
    Error(ErrorCode),
}

/// Data returned from a completed callback.
#[derive(Debug, Clone, PartialEq)]
pub enum CallbackData {
    /// No data (unit operation)
    None,
    /// Single number value
    Number(u64),
    /// Timestamp value
    Timestamp(u64),
    /// Structured data with multiple fields
    Structured {
        /// First callback argument
        arg0: u32,
        /// Second callback argument  
        arg1: u32,
        /// Third callback argument
        arg2: u32,
        /// Application-provided data
        user_data: u32,
    },
}

impl CallbackData {
    /// Create new structured callback data.
    pub fn new(arg0: u32, arg1: u32, arg2: u32, user_data: u32) -> Self {
        Self::Structured {
            arg0,
            arg1,
            arg2,
            user_data,
        }
    }
}

/// Information about a buffer allocation.
#[derive(Debug, Clone, PartialEq)]
pub struct BufferInfo {
    /// Size of the buffer in bytes
    pub size: usize,
    /// Address of the buffer
    pub address: *const u8,
    /// Whether the buffer is mutable
    pub is_mutable: bool,
    /// Required alignment for the buffer
    pub alignment: usize,
}

/// OS-agnostic syscall interface.
///
/// This trait provides a unified interface for system calls that works across
/// different operating systems. It abstracts away OS-specific details while
/// providing the necessary functionality for implementing higher-level services.
///
/// The interface is designed to be:
/// - **OS-agnostic**: Works with Tock, Linux, Hubris, etc.
/// - **Non-blocking**: All operations can be used in non-blocking contexts
/// - **Resource-safe**: Handle-based management prevents resource leaks
/// - **Extensible**: Easy to add new syscall types and operations
///
/// # Handle-based Resource Management
///
/// The interface uses handles to manage resources like buffers and callbacks.
/// This provides several benefits:
/// - **Type safety**: Handles are typed and prevent misuse
/// - **Resource tracking**: The system can track and clean up resources
/// - **OS independence**: Handles abstract OS-specific resource identifiers
///
/// # Examples
///
/// ```rust
/// use syscall_abstraction::prelude::*;
/// 
/// # fn example(syscalls: impl GenericSyscalls) -> Result<(), ErrorCode> {
/// // Immediate command
/// let result = syscalls.command_immediate(0, 1, 100, 0)?;
/// 
/// // Buffer operations
/// let data = b"Hello, world!";
/// let buffer_handle = syscalls.setup_buffer(0, 0, data)?;
/// 
/// // Use buffer...
/// 
/// // Clean up
/// syscalls.cleanup_buffer(buffer_handle)?;
/// # Ok(())
/// # }
/// 
/// // For callback operations, use CallbackManager trait:
/// # fn callback_example(callbacks: impl CallbackManager) -> Result<(), ErrorCode> {
/// let callback_handle = callbacks.setup_callback(0, 0)?;
/// 
/// // Poll the callback status
/// match callbacks.poll_callback(&callback_handle) {
///     CallbackStatus::Completed(data) => {
///         println!("Callback completed with data: {:?}", data);
///     }
///     CallbackStatus::Pending => {
///         println!("Callback still pending");
///     }
///     CallbackStatus::Error(e) => {
///         println!("Callback failed: {:?}", e);
///     }
/// }
/// 
/// // Clean up the callback
/// callbacks.cleanup_callback(callback_handle)?;
/// # Ok(())
/// # }
/// ```
pub trait GenericSyscalls: Clone {
    /// Execute an immediate command that returns synchronously.
    ///
    /// This is used for simple operations that can complete immediately
    /// without requiring callbacks or complex state management.
    ///
    /// # Parameters
    ///
    /// - `driver_id`: ID of the driver to send the command to
    /// - `command_id`: ID of the command within the driver
    /// - `arg0`: First command argument
    /// - `arg1`: Second command argument
    ///
    /// # Returns
    ///
    /// The result of the command execution.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(syscalls: impl GenericSyscalls) -> Result<(), ErrorCode> {
    /// // Get the current alarm frequency
    /// let frequency = syscalls.command_immediate(0, 1, 0, 0)?;
    /// println!("Alarm frequency: {} Hz", frequency);
    /// # Ok(())
    /// # }
    /// ```
    fn command_immediate(&self, driver_id: u32, command_id: u32, arg0: u32, arg1: u32) -> Result<u32, ErrorCode>;

    /// Setup a read-only buffer for use with a driver.
    ///
    /// This allows the driver to read data from the application's memory
    /// without copying. The buffer remains valid until cleaned up.
    ///
    /// # Parameters
    ///
    /// - `driver_id`: ID of the driver to share the buffer with
    /// - `buffer_id`: ID of the buffer slot within the driver
    /// - `buffer`: The buffer data to share
    ///
    /// # Returns
    ///
    /// A handle to the buffer allocation that can be used for cleanup.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(syscalls: impl GenericSyscalls) -> Result<(), ErrorCode> {
    /// let data = b"Hello, world!";
    /// let buffer_handle = syscalls.setup_buffer(0, 0, data)?;
    /// 
    /// // Use the buffer with driver operations...
    /// 
    /// // Clean up when done
    /// syscalls.cleanup_buffer(buffer_handle)?;
    /// # Ok(())
    /// # }
    /// ```
    fn setup_buffer(&self, driver_id: u32, buffer_id: u32, buffer: &[u8]) -> Result<BufferHandle, ErrorCode>;

    /// Setup a mutable buffer for use with a driver.
    ///
    /// This allows the driver to write data to the application's memory
    /// without copying. The buffer remains valid until cleaned up.
    ///
    /// # Parameters
    ///
    /// - `driver_id`: ID of the driver to share the buffer with
    /// - `buffer_id`: ID of the buffer slot within the driver
    /// - `buffer`: The mutable buffer to share
    ///
    /// # Returns
    ///
    /// A handle to the buffer allocation that can be used for cleanup.
    fn setup_mutable_buffer(&self, driver_id: u32, buffer_id: u32, buffer: &mut [u8]) -> Result<BufferHandle, ErrorCode>;

    /// Get information about a buffer allocation.
    ///
    /// This returns metadata about a buffer that was previously allocated
    /// with `setup_buffer` or `setup_mutable_buffer`.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the buffer allocation
    ///
    /// # Returns
    ///
    /// Information about the buffer.
    fn get_buffer_info(&self, handle: &BufferHandle) -> Result<BufferInfo, ErrorCode>;

    /// Clean up a buffer allocation.
    ///
    /// This releases the buffer and makes it safe to deallocate the
    /// underlying memory. The handle becomes invalid after this call.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the buffer allocation
    ///
    /// # Returns
    ///
    /// Result of the cleanup operation.
    fn cleanup_buffer(&self, handle: BufferHandle) -> Result<(), ErrorCode>;

    // === Console-specific convenience methods ===

    /// Write data to a buffer.
    fn write_buffer(&self, data: &[u8]) -> Result<CallbackHandle, ErrorCode>;

    /// Read data from a buffer.
    fn read_buffer(&self, handle: BufferHandle, size: usize) -> Result<CallbackHandle, ErrorCode>;

    /// Check if input is available.
    fn input_available(&self) -> Result<bool, ErrorCode>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_callback_handle_creation() {
        let handle = CallbackHandle::new(1, 2, 12345);
        assert_eq!(handle.driver_id, 1);
        assert_eq!(handle.callback_id, 2);
        assert_eq!(handle.internal_id, 12345);
    }

    #[test]
    fn test_buffer_handle_creation() {
        let handle = BufferHandle::new(3, 4, 67890);
        assert_eq!(handle.driver_id, 3);
        assert_eq!(handle.buffer_id, 4);
        assert_eq!(handle.internal_id, 67890);
    }

    #[test]
    fn test_callback_data_creation() {
        let data = CallbackData::new(1, 2, 3, 4);
        match data {
            CallbackData::Structured { arg0, arg1, arg2, user_data } => {
                assert_eq!(arg0, 1);
                assert_eq!(arg1, 2);
                assert_eq!(arg2, 3);
                assert_eq!(user_data, 4);
            }
            _ => panic!("Expected structured data"),
        }
    }

    #[test]
    fn test_callback_status_variants() {
        let pending = CallbackStatus::Pending;
        assert!(matches!(pending, CallbackStatus::Pending));

        let completed = CallbackStatus::Completed(CallbackData::new(1, 2, 3, 4));
        assert!(matches!(completed, CallbackStatus::Completed(_)));

        let error = CallbackStatus::Error(ErrorCode::Timeout);
        assert!(matches!(error, CallbackStatus::Error(ErrorCode::Timeout)));
    }
}
