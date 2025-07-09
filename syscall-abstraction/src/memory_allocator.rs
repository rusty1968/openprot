// Licensed under the Apache-2.0 license

//! Memory allocation trait for syscall abstraction.
//!
//! This trait provides memory allocation services that are separate from
//! the core syscall interface. This allows implementations to support
//! `no_alloc` environments by implementing only the core `GenericSyscalls`
//! trait without the allocation methods.

use crate::error_handling::ErrorCode;
use crate::generic_syscalls::BufferHandle;

/// Memory allocation interface for syscall implementations.
///
/// This trait provides dynamic memory allocation services that complement
/// the core syscall interface. It's separated from `GenericSyscalls` to
/// allow implementations in `no_alloc` environments like Tock.
///
/// # Design Rationale
///
/// The memory allocation methods were separated from the core syscall interface
/// because:
///
/// - **No-alloc compatibility**: Embedded systems like Tock don't support dynamic allocation
/// - **Separation of concerns**: Memory management is conceptually separate from syscalls
/// - **Optional functionality**: Not all syscall implementations need dynamic allocation
///
/// # Examples
///
/// ```rust
/// use syscall_abstraction::prelude::*;
/// 
/// # fn example(syscalls: impl GenericSyscalls + MemoryAllocator) -> Result<(), ErrorCode> {
/// // Allocate a buffer dynamically
/// let buffer_handle = syscalls.allocate_buffer(1024)?;
/// 
/// // Use the buffer...
/// 
/// // Free the buffer
/// syscalls.free_buffer(buffer_handle)?;
/// # Ok(())
/// # }
/// ```
pub trait MemoryAllocator {
    /// Allocate a buffer of the specified size.
    ///
    /// This creates a new buffer that can be used with drivers. The buffer
    /// is managed by the allocator and must be freed when no longer needed.
    ///
    /// # Parameters
    ///
    /// - `size`: Size of the buffer to allocate in bytes
    ///
    /// # Returns
    ///
    /// A handle to the allocated buffer.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(allocator: impl MemoryAllocator) -> Result<(), ErrorCode> {
    /// let buffer_handle = allocator.allocate_buffer(1024)?;
    /// 
    /// // Use the buffer with drivers...
    /// 
    /// allocator.free_buffer(buffer_handle)?;
    /// # Ok(())
    /// # }
    /// ```
    fn allocate_buffer(&self, size: usize) -> Result<BufferHandle, ErrorCode>;

    /// Free a previously allocated buffer.
    ///
    /// This releases the buffer and makes the handle invalid. The buffer
    /// should not be used after this call.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the buffer to free
    ///
    /// # Returns
    ///
    /// Result of the free operation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(allocator: impl MemoryAllocator) -> Result<(), ErrorCode> {
    /// let buffer_handle = allocator.allocate_buffer(1024)?;
    /// 
    /// // Use the buffer...
    /// 
    /// allocator.free_buffer(buffer_handle)?;
    /// # Ok(())
    /// # }
    /// ```
    fn free_buffer(&self, handle: BufferHandle) -> Result<(), ErrorCode>;

    /// Read data from a buffer into a Vec.
    ///
    /// This copies data from a buffer handle into a newly allocated Vec.
    /// This method requires dynamic allocation and will not be available
    /// in `no_alloc` environments.
    ///
    /// # Parameters
    ///
    /// - `handle`: Handle to the buffer to read from
    ///
    /// # Returns
    ///
    /// A Vec containing the buffer data.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use syscall_abstraction::prelude::*;
    /// # fn example(allocator: impl MemoryAllocator) -> Result<(), ErrorCode> {
    /// let buffer_handle = allocator.allocate_buffer(1024)?;
    /// 
    /// // Buffer gets filled by some operation...
    /// 
    /// let data = allocator.read_from_buffer(buffer_handle)?;
    /// println!("Read {} bytes", data.len());
    /// 
    /// allocator.free_buffer(buffer_handle)?;
    /// # Ok(())
    /// # }
    /// ```
    fn read_from_buffer(&self, handle: BufferHandle) -> Result<Vec<u8>, ErrorCode>;
}

/// Extension trait for implementations that support both syscalls and memory allocation.
///
/// This trait is automatically implemented for types that implement both
/// `GenericSyscalls` and `MemoryAllocator`.
pub trait SyscallsWithMemory: crate::generic_syscalls::GenericSyscalls + MemoryAllocator {}

impl<T> SyscallsWithMemory for T where T: crate::generic_syscalls::GenericSyscalls + MemoryAllocator {}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAllocator;

    impl MemoryAllocator for MockAllocator {
        fn allocate_buffer(&self, size: usize) -> Result<BufferHandle, ErrorCode> {
            if size > 0 {
                Ok(BufferHandle::new(0, 0, size as u64))
            } else {
                Err(ErrorCode::InvalidArgument)
            }
        }

        fn free_buffer(&self, _handle: BufferHandle) -> Result<(), ErrorCode> {
            Ok(())
        }

        fn read_from_buffer(&self, _handle: BufferHandle) -> Result<Vec<u8>, ErrorCode> {
            Ok(vec![1, 2, 3, 4])
        }
    }

    #[test]
    fn test_allocate_buffer() {
        let allocator = MockAllocator;
        let handle = allocator.allocate_buffer(1024).unwrap();
        assert_eq!(handle.internal_id, 1024);
    }

    #[test]
    fn test_free_buffer() {
        let allocator = MockAllocator;
        let handle = BufferHandle::new(0, 0, 1024);
        allocator.free_buffer(handle).unwrap();
    }

    #[test]
    fn test_read_from_buffer() {
        let allocator = MockAllocator;
        let handle = BufferHandle::new(0, 0, 1024);
        let data = allocator.read_from_buffer(handle).unwrap();
        assert_eq!(data, vec![1, 2, 3, 4]);
    }
}
