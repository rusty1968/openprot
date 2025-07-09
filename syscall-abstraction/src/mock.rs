//! Mock implementation for testing and development.
//!
//! This module provides a mock syscall implementation that can be used
//! for testing and development without requiring a real OS or hardware.

use crate::error_handling::ErrorCode;
use crate::generic_syscalls::{
    GenericSyscalls, CallbackHandle, BufferHandle, CallbackStatus, CallbackData, BufferInfo
};

use core::cell::{Cell, RefCell};
use std::collections::HashMap;

/// Mock syscall implementation for testing.
#[derive(Debug)]
pub struct MockSyscalls {
    next_callback_id: Cell<u64>,
    next_buffer_id: Cell<u64>,
    callbacks: RefCell<HashMap<u64, MockCallback>>,
    buffers: RefCell<HashMap<u64, MockBuffer>>,
    current_time: Cell<u64>,
}

#[derive(Debug, Clone)]
struct MockCallback {
    driver_id: u32,
    callback_id: u32,
    status: CallbackStatus,
    delay_ms: u32,
    created_time: u64,
}

#[derive(Debug, Clone)]
struct MockBuffer {
    driver_id: u32,
    buffer_id: u32,
    data: Vec<u8>,
    size: usize,
}

impl MockSyscalls {
    /// Create a new mock syscall implementation.
    pub fn new() -> Self {
        Self {
            next_callback_id: Cell::new(1),
            next_buffer_id: Cell::new(1),
            callbacks: RefCell::new(HashMap::new()),
            buffers: RefCell::new(HashMap::new()),
            current_time: Cell::new(0),
        }
    }

    /// Advance the mock time by the specified amount.
    pub fn advance_time(&self, delta_ms: u64) {
        let new_time = self.current_time.get() + delta_ms;
        self.current_time.set(new_time);
        
        // Update callback statuses based on time
        let mut callbacks = self.callbacks.borrow_mut();
        for (_, callback) in callbacks.iter_mut() {
            if matches!(callback.status, CallbackStatus::Pending) {
                let elapsed = new_time - callback.created_time;
                if elapsed >= callback.delay_ms as u64 {
                    callback.status = CallbackStatus::Completed(CallbackData::None);
                }
            }
        }
    }
}

impl Default for MockSyscalls {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for MockSyscalls {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl GenericSyscalls for MockSyscalls {
    fn command_immediate(&self, _driver_id: u32, _command_id: u32, _arg0: u32, _arg1: u32) -> Result<u32, ErrorCode> {
        // Mock implementation always succeeds
        Ok(42)
    }



    fn setup_buffer(&self, driver_id: u32, buffer_id: u32, buffer: &[u8]) -> Result<BufferHandle, ErrorCode> {
        let id = self.next_buffer_id.get();
        self.next_buffer_id.set(id + 1);
        
        let data = buffer.to_vec();
        
        let mock_buffer = MockBuffer {
            driver_id,
            buffer_id,
            data,
            size: buffer.len(),
        };
        
        self.buffers.borrow_mut().insert(id, mock_buffer);
        
        Ok(BufferHandle::new(driver_id, buffer_id, id))
    }

    fn setup_mutable_buffer(&self, driver_id: u32, buffer_id: u32, buffer: &mut [u8]) -> Result<BufferHandle, ErrorCode> {
        let id = self.next_buffer_id.get();
        self.next_buffer_id.set(id + 1);
        
        let data = buffer.to_vec();
        
        let mock_buffer = MockBuffer {
            driver_id,
            buffer_id,
            data,
            size: buffer.len(),
        };
        
        self.buffers.borrow_mut().insert(id, mock_buffer);
        
        Ok(BufferHandle::new(driver_id, buffer_id, id))
    }





    fn get_buffer_info(&self, handle: &BufferHandle) -> Result<BufferInfo, ErrorCode> {
        let buffers = self.buffers.borrow();
        if let Some(buffer) = buffers.get(&handle.internal_id) {
            Ok(BufferInfo {
                size: buffer.size,
                address: buffer.data.as_ptr(),
                is_mutable: true,
                alignment: 4,
            })
        } else {
            Err(ErrorCode::InvalidHandle)
        }
    }

    fn cleanup_buffer(&self, handle: BufferHandle) -> Result<(), ErrorCode> {
        let mut buffers = self.buffers.borrow_mut();
        if buffers.remove(&handle.internal_id).is_some() {
            Ok(())
        } else {
            Err(ErrorCode::InvalidHandle)
        }
    }



    // === Console-specific convenience methods ===





    fn write_buffer(&self, data: &[u8]) -> Result<CallbackHandle, ErrorCode> {
        // Create a callback using the CallbackManager trait
        let handle = crate::callback_manager::CallbackManager::setup_callback(self, 0x01, 0)?;
        
        // Update the callback status to completed
        let mut callbacks = self.callbacks.borrow_mut();
        if let Some(callback) = callbacks.get_mut(&handle.internal_id) {
            callback.status = CallbackStatus::Completed(CallbackData::Number(data.len() as u64));
        }
        
        Ok(handle)
    }

    fn read_buffer(&self, _handle: BufferHandle, size: usize) -> Result<CallbackHandle, ErrorCode> {
        // Create a callback using the CallbackManager trait
        let handle = crate::callback_manager::CallbackManager::setup_callback(self, 0x01, 1)?;
        
        // Update the callback status to completed
        let mut callbacks = self.callbacks.borrow_mut();
        if let Some(callback) = callbacks.get_mut(&handle.internal_id) {
            callback.status = CallbackStatus::Completed(CallbackData::Number(size as u64));
        }
        
        Ok(handle)
    }

    fn input_available(&self) -> Result<bool, ErrorCode> {
        // Mock implementation - randomly return true/false
        Ok(true)
    }
}

impl crate::memory_allocator::MemoryAllocator for MockSyscalls {
    fn allocate_buffer(&self, size: usize) -> Result<BufferHandle, ErrorCode> {
        let id = self.next_buffer_id.get();
        self.next_buffer_id.set(id + 1);
        
        let mock_buffer = MockBuffer {
            driver_id: 0xFF, // System driver
            buffer_id: 0,
            data: Vec::new(),
            size,
        };
        
        self.buffers.borrow_mut().insert(id, mock_buffer);
        
        Ok(BufferHandle::new(0xFF, 0, id))
    }

    fn free_buffer(&self, handle: BufferHandle) -> Result<(), ErrorCode> {
        self.cleanup_buffer(handle)
    }

    fn read_from_buffer(&self, handle: BufferHandle) -> Result<Vec<u8>, ErrorCode> {
        let buffers = self.buffers.borrow();
        if let Some(buffer) = buffers.get(&handle.internal_id) {
            Ok(buffer.data.clone())
        } else {
            Err(ErrorCode::InvalidHandle)
        }
    }
}

impl crate::callback_manager::CallbackManager for MockSyscalls {
    fn setup_callback(&self, driver_id: u32, callback_id: u32) -> Result<CallbackHandle, ErrorCode> {
        let id = self.next_callback_id.get();
        self.next_callback_id.set(id + 1);
        
        let callback = MockCallback {
            driver_id,
            callback_id,
            status: CallbackStatus::Pending,
            delay_ms: 100, // Default delay
            created_time: self.current_time.get(),
        };
        
        self.callbacks.borrow_mut().insert(id, callback);
        
        Ok(CallbackHandle::new(driver_id, callback_id, id))
    }

    fn poll_callback(&self, handle: &CallbackHandle) -> CallbackStatus {
        let callbacks = self.callbacks.borrow();
        if let Some(callback) = callbacks.get(&handle.internal_id) {
            callback.status.clone()
        } else {
            CallbackStatus::Error(ErrorCode::InvalidHandle)
        }
    }

    fn wait_callback(&self, handle: &CallbackHandle) -> Result<CallbackData, ErrorCode> {
        let callbacks = self.callbacks.borrow();
        if let Some(callback) = callbacks.get(&handle.internal_id) {
            match &callback.status {
                CallbackStatus::Completed(data) => Ok(data.clone()),
                CallbackStatus::Pending => Err(ErrorCode::Busy),
                CallbackStatus::Error(e) => Err(*e),
            }
        } else {
            Err(ErrorCode::InvalidHandle)
        }
    }

    fn cleanup_callback(&self, handle: CallbackHandle) -> Result<(), ErrorCode> {
        let mut callbacks = self.callbacks.borrow_mut();
        if callbacks.remove(&handle.internal_id).is_some() {
            Ok(())
        } else {
            Err(ErrorCode::InvalidHandle)
        }
    }

    fn check_callback(&self, handle: CallbackHandle) -> Result<CallbackStatus, ErrorCode> {
        let callbacks = self.callbacks.borrow();
        if let Some(callback) = callbacks.get(&handle.internal_id) {
            Ok(callback.status.clone())
        } else {
            Err(ErrorCode::InvalidHandle)
        }
    }

    fn cancel_callback(&self, handle: CallbackHandle) -> Result<(), ErrorCode> {
        let mut callbacks = self.callbacks.borrow_mut();
        if callbacks.remove(&handle.internal_id).is_some() {
            Ok(())
        } else {
            Err(ErrorCode::InvalidHandle)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_syscalls_creation() {
        let mock = MockSyscalls::new();
        assert_eq!(mock.current_time.get(), 0);
    }

    #[test]
    fn test_mock_time_advancement() {
        let mock = MockSyscalls::new();
        mock.advance_time(100);
        assert_eq!(mock.current_time.get(), 100);
    }

    #[test]
    fn test_mock_console_operations() {
        use crate::memory_allocator::MemoryAllocator;
        use crate::callback_manager::CallbackManager;
        
        let mock = MockSyscalls::new();
        
        // Test write buffer
        let handle = mock.write_buffer(b"Hello, World!").unwrap();
        let status = mock.check_callback(handle).unwrap();
        assert!(matches!(status, CallbackStatus::Completed(_)));
        
        // Test buffer allocation
        let buffer_handle = mock.allocate_buffer(256).unwrap();
        mock.free_buffer(buffer_handle).unwrap();
        
        // Test console operations
        assert!(mock.input_available().unwrap());
    }
}
