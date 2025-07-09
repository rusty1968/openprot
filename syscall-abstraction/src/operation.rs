// Licensed under the Apache-2.0 license

//! Core operation trait for syscall abstraction.

use crate::execution_context::{ExecutionContext, OperationResult};
use crate::error_handling::ErrorCode;

/// Core abstraction for any operation that can be executed in different contexts.
///
/// Operations represent a single unit of work that can be performed by the system.
/// They are designed to work in synchronous, asynchronous, and polling execution
/// models through the `execute` method.
///
/// # Examples
///
/// ```rust
/// use syscall_abstraction::prelude::*;
/// 
/// struct SimpleOperation {
///     completed: bool,
/// }
/// 
/// impl Operation<u32> for SimpleOperation {
///     fn execute(&mut self, ctx: ExecutionContext) -> OperationResult<u32> {
///         if self.completed {
///             return OperationResult::Ready(Ok(42));
///         }
///         
///         match ctx {
///             ExecutionContext::Sync => {
///                 // Simulate work
///                 self.completed = true;
///                 OperationResult::Ready(Ok(42))
///             }
///             ExecutionContext::Poll => {
///                 // Non-blocking check
///                 if self.can_complete_immediately() {
///                     self.completed = true;
///                     OperationResult::Ready(Ok(42))
///                 } else {
///                     OperationResult::WouldBlock
///                 }
///             }
///             #[cfg(feature = "async")]
///             ExecutionContext::Async(_waker) => {
///                 // Set up async completion
///                 self.completed = true;
///                 OperationResult::Ready(Ok(42))
///             }
///         }
///     }
///     
///     fn can_complete_immediately(&self) -> bool {
///         self.completed
///     }
///     
///     fn cancel(&mut self) -> Result<(), ErrorCode> {
///         self.completed = false;
///         Ok(())
///     }
/// }
/// ```
pub trait Operation<T> {
    /// Execute the operation in the given execution context.
    ///
    /// This method is called by the execution adapters to perform the actual work.
    /// The implementation should handle the different execution contexts appropriately:
    ///
    /// - **Sync**: Can block until completion
    /// - **Async**: Should not block, use waker for notifications
    /// - **Poll**: Must return immediately with status
    ///
    /// # Parameters
    ///
    /// - `ctx`: The execution context providing constraints and capabilities
    ///
    /// # Returns
    ///
    /// - `Ready(result)`: Operation completed with result
    /// - `Pending`: Operation will complete later via callback
    /// - `WouldBlock`: Operation cannot proceed without blocking (polling only)
    fn execute(&mut self, ctx: ExecutionContext) -> OperationResult<T>;

    /// Check if the operation can complete immediately without blocking.
    ///
    /// This is used for optimization - if an operation can complete immediately,
    /// the execution adapters can avoid setting up complex async machinery.
    ///
    /// # Returns
    ///
    /// `true` if the operation can complete immediately, `false` otherwise.
    fn can_complete_immediately(&self) -> bool {
        false
    }

    /// Cancel the operation and clean up any resources.
    ///
    /// This provides best-effort cancellation:
    /// - For immediate operations: Returns `Ok(())` (nothing to cancel)
    /// - For pending operations: Attempts to cancel and clean up resources
    /// - For completed operations: Returns `ErrorCode::Already`
    ///
    /// # Returns
    ///
    /// - `Ok(())`: Operation was successfully cancelled or was already complete
    /// - `Err(ErrorCode)`: Cancellation failed for some reason
    fn cancel(&mut self) -> Result<(), ErrorCode> {
        // Default implementation - nothing to cancel
        Ok(())
    }

    /// Estimate how long this operation might take to complete.
    ///
    /// This is used for optimization and scheduling decisions. The estimate
    /// should be conservative - it's better to overestimate than underestimate.
    ///
    /// # Returns
    ///
    /// - `Some(duration)`: Estimated completion time
    /// - `None`: Cannot estimate or operation is immediate
    fn estimate_completion_time(&self) -> Option<core::time::Duration> {
        None
    }

    /// Get the types of resources this operation requires.
    ///
    /// This is used for resource management and scheduling decisions.
    ///
    /// # Returns
    ///
    /// List of resource types required by this operation.
    fn required_resources(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Marker trait for operations that can be executed multiple times.
///
/// Some operations are idempotent and can be safely retried or executed
/// multiple times without side effects.
pub trait IdempotentOperation<T>: Operation<T> {}

/// Marker trait for operations that modify system state.
///
/// Operations that implement this trait may have side effects and should
/// be handled carefully in retry scenarios.
pub trait MutatingOperation<T>: Operation<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_context::ExecutionContext;

    struct TestOperation {
        completed: bool,
        result: u32,
    }

    impl TestOperation {
        fn new(result: u32) -> Self {
            Self {
                completed: false,
                result,
            }
        }
    }

    impl Operation<u32> for TestOperation {
        fn execute(&mut self, ctx: ExecutionContext) -> OperationResult<u32> {
            if self.completed {
                return OperationResult::Ready(Ok(self.result));
            }

            match ctx {
                ExecutionContext::Sync => {
                    self.completed = true;
                    OperationResult::Ready(Ok(self.result))
                }
                ExecutionContext::Poll => {
                    if self.can_complete_immediately() {
                        self.completed = true;
                        OperationResult::Ready(Ok(self.result))
                    } else {
                        OperationResult::WouldBlock
                    }
                }
                #[cfg(feature = "async")]
                ExecutionContext::Async(_) => {
                    self.completed = true;
                    OperationResult::Ready(Ok(self.result))
                }
            }
        }

        fn can_complete_immediately(&self) -> bool {
            self.completed
        }

        fn cancel(&mut self) -> Result<(), ErrorCode> {
            self.completed = false;
            Ok(())
        }
    }

    #[test]
    fn test_operation_sync_execution() {
        let mut op = TestOperation::new(42);
        let result = op.execute(ExecutionContext::Sync);
        assert!(result.is_ready());
        assert_eq!(result.ready(), Some(Ok(42)));
    }

    #[test]
    fn test_operation_poll_execution() {
        let mut op = TestOperation::new(42);
        let result = op.execute(ExecutionContext::Poll);
        assert!(result.would_block());
        
        // After completing, should be ready
        op.completed = true;
        let result = op.execute(ExecutionContext::Poll);
        assert!(result.is_ready());
        assert_eq!(result.ready(), Some(Ok(42)));
    }

    #[test]
    fn test_operation_cancel() {
        let mut op = TestOperation::new(42);
        op.completed = true;
        
        assert!(op.cancel().is_ok());
        assert!(!op.completed);
    }

    #[test]
    fn test_operation_can_complete_immediately() {
        let mut op = TestOperation::new(42);
        assert!(!op.can_complete_immediately());
        
        op.completed = true;
        assert!(op.can_complete_immediately());
    }
}
