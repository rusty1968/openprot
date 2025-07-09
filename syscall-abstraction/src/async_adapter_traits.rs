//! Asynchronous execution adapter traits.

use crate::error_handling::ErrorCode;
use crate::operation::Operation;
use std::future::Future;
use std::pin::Pin;

/// Trait for executing operations in asynchronous (non-blocking) contexts.
///
/// This trait enables operations to be used with async/await syntax
/// and any async runtime (tokio, async-std, etc.).
///
/// # Features
///
/// - **Non-blocking**: Uses async/await for cooperative multitasking
/// - **Runtime Agnostic**: Works with any async runtime
/// - **Waker Support**: Proper async notifications
/// - **Cancellation**: Support for async cancellation
pub trait AsyncAdapter {
    /// Execute an operation asynchronously.
    ///
    /// Returns a Future that resolves when the operation completes.
    fn execute_async<T, O: Operation<T>>(
        operation: O,
    ) -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>>;

    /// Execute an operation with async timeout.
    ///
    /// Returns a Future that resolves with the result or times out.
    fn execute_with_timeout<T, O: Operation<T>>(
        operation: O,
        timeout_ms: u32,
    ) -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>>;

    /// Execute multiple operations concurrently.
    ///
    /// Returns a Future that resolves when all operations complete.
    fn execute_all<T, O: Operation<T>>(
        operations: Vec<O>,
    ) -> Pin<Box<dyn Future<Output = Vec<Result<T, ErrorCode>>> + Send>>;

    /// Execute operations and return the first successful result.
    ///
    /// Returns a Future that resolves with the first successful result.
    fn execute_race<T, O: Operation<T>>(
        operations: Vec<O>,
    ) -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>>;
}
