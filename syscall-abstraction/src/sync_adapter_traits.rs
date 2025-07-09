//! Synchronous execution adapter traits.

use crate::error_handling::ErrorCode;
use crate::operation::Operation;

/// Trait for executing operations in synchronous (blocking) contexts.
///
/// This trait enables operations to be used in traditional synchronous
/// code where blocking until completion is acceptable.
///
/// # Features
///
/// - **Blocking**: Blocks the calling thread until completion
/// - **Simple Error Handling**: Direct Result<T, ErrorCode> return
/// - **Retry Logic**: Built-in retry mechanisms for transient failures
/// - **Timeout Support**: Operations can be bounded by time limits
pub trait SyncAdapter {
    /// Execute an operation synchronously, blocking until completion.
    ///
    /// This will block the calling thread until the operation completes,
    /// either successfully or with an error.
    fn execute_blocking<T, O: Operation<T>>(operation: O) -> Result<T, ErrorCode>;

    /// Execute an operation with retry logic.
    ///
    /// This will attempt the operation multiple times with configurable
    /// retry parameters if transient failures occur.
    fn execute_with_retry<T, O: Operation<T>>(
        operation: O,
        max_retries: u32,
    ) -> Result<T, ErrorCode>;

    /// Execute an operation with a timeout.
    ///
    /// This will attempt the operation but give up after the specified
    /// timeout period, returning a timeout error.
    fn execute_with_timeout<T, O: Operation<T>>(
        operation: O,
        timeout_ms: u32,
    ) -> Result<T, ErrorCode>;

    /// Execute an operation with both retry and timeout.
    ///
    /// This combines retry logic with timeout bounds for maximum robustness.
    fn execute_with_retry_and_timeout<T, O: Operation<T>>(
        operation: O,
        max_retries: u32,
        timeout_ms: u32,
    ) -> Result<T, ErrorCode>;
}
