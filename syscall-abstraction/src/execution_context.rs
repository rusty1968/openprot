// Licensed under the Apache-2.0 license

//! Execution context types for OS-agnostic syscall operations.

#[cfg(feature = "async")]
use core::task::Waker;

/// Represents the execution environment where an operation runs.
/// 
/// Different execution contexts have different constraints:
/// - **Sync**: Can block indefinitely, expects immediate completion
/// - **Async**: Cannot block, needs notification mechanism via Waker
/// - **Poll**: Cannot block, caller handles retry logic
#[derive(Debug)]
pub enum ExecutionContext {
    /// Synchronous execution - blocks until completion
    Sync,
    /// Asynchronous execution with a waker for notifications  
    #[cfg(feature = "async")]
    Async(Waker),
    /// Polling execution - returns immediately with status
    Poll,
}

/// Result of executing an operation in a specific context.
#[derive(Debug)]
pub enum OperationResult<T> {
    /// Operation completed immediately with result
    Ready(Result<T, crate::error_handling::ErrorCode>),
    /// Operation is pending, will complete later via callback
    Pending,
    /// Operation would block in polling context
    WouldBlock,
}

impl<T> OperationResult<T> {
    /// Check if the operation is ready
    pub fn is_ready(&self) -> bool {
        matches!(self, OperationResult::Ready(_))
    }
    
    /// Check if the operation is pending
    pub fn is_pending(&self) -> bool {
        matches!(self, OperationResult::Pending)
    }
    
    /// Check if the operation would block
    pub fn would_block(&self) -> bool {
        matches!(self, OperationResult::WouldBlock)
    }
    
    /// Convert to Option, returning Some only if Ready
    pub fn ready(self) -> Option<Result<T, crate::error_handling::ErrorCode>> {
        match self {
            OperationResult::Ready(result) => Some(result),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_result_ready() {
        let result: OperationResult<u32> = OperationResult::Ready(Ok(42));
        assert!(result.is_ready());
        assert!(!result.is_pending());
        assert!(!result.would_block());
        assert_eq!(result.ready(), Some(Ok(42)));
    }

    #[test]
    fn test_operation_result_pending() {
        let result: OperationResult<u32> = OperationResult::Pending;
        assert!(!result.is_ready());
        assert!(result.is_pending());
        assert!(!result.would_block());
        assert_eq!(result.ready(), None);
    }

    #[test]
    fn test_operation_result_would_block() {
        let result: OperationResult<u32> = OperationResult::WouldBlock;
        assert!(!result.is_ready());
        assert!(!result.is_pending());
        assert!(result.would_block());
        assert_eq!(result.ready(), None);
    }
}
