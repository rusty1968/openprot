//! Polling execution adapter traits.

use crate::error_handling::ErrorCode;
use crate::execution_context::OperationResult;
use crate::operation::Operation;

/// Trait for executing operations in polling (non-blocking) contexts.
///
/// This trait enables operations to be used in custom event loops,
/// real-time systems, or any context where blocking is not acceptable.
/// All operations return immediately with their current status.
///
/// # Features
///
/// - **Non-blocking**: Never blocks the calling thread
/// - **Immediate Return**: Always returns status immediately
/// - **Custom Control**: Caller controls retry timing and strategy
/// - **Deterministic**: Predictable execution timing
pub trait PollAdapter {
    /// Try to execute an operation without blocking.
    ///
    /// This will attempt to execute the operation once and return immediately
    /// with the current status. It never blocks the calling thread.
    fn try_execute<T, O: Operation<T>>(operation: &mut O) -> OperationResult<T>;

    /// Try to execute with a limited number of immediate retries.
    ///
    /// This will attempt the operation multiple times in succession without
    /// any delays. It's useful for operations that might succeed quickly
    /// on retry.
    fn try_execute_with_retries<T, O: Operation<T>>(
        operation: &mut O,
        max_retries: u32,
    ) -> OperationResult<T>;

    /// Execute operation in a polling loop with custom timing control.
    ///
    /// This provides a polling loop where the caller controls whether to
    /// continue polling. The callback function is called between polling
    /// attempts to determine if polling should continue.
    fn poll_until_complete<T, O: Operation<T>, F>(
        operation: &mut O,
        should_continue: F,
    ) -> OperationResult<T>
    where
        F: FnMut() -> bool;

    /// Poll an operation until it completes or a maximum number of attempts.
    ///
    /// This combines polling with attempt limiting, useful for operations
    /// that should eventually complete but might take multiple polls.
    fn poll_with_limit<T, O: Operation<T>>(
        operation: &mut O,
        max_attempts: u32,
    ) -> OperationResult<T>;

    /// Check if an operation can complete immediately.
    ///
    /// This is a quick check that doesn't modify the operation state,
    /// useful for optimizing polling strategies.
    fn can_complete_immediately<T, O: Operation<T>>(operation: &O) -> bool;
}

/// Trait for managing collections of operations in event loops.
///
/// This manages a collection of operations and provides methods for
/// polling them efficiently.
pub trait EventLoop {
    /// Add an operation to the event loop.
    fn add_operation(&mut self, operation: Box<dyn Operation<()>>) -> Result<(), ErrorCode>;

    /// Poll all operations once.
    ///
    /// This will attempt to execute each operation once and move completed
    /// operations to the completed list.
    ///
    /// Returns true if any operations are still pending.
    fn poll_all(&mut self) -> bool;

    /// Run the event loop until all operations are idle.
    ///
    /// This will continue polling until no operations are pending.
    fn run_until_idle(&mut self);

    /// Run the event loop for a specific number of iterations.
    ///
    /// Returns true if operations are still pending after max iterations.
    fn run_for_iterations(&mut self, max_iterations: u32) -> bool;

    /// Get the number of pending operations.
    fn pending_count(&self) -> usize;

    /// Get the number of completed operations.
    fn completed_count(&self) -> usize;

    /// Clear all completed operations.
    fn clear_completed(&mut self);

    /// Cancel all pending operations.
    fn cancel_all(&mut self);
}
