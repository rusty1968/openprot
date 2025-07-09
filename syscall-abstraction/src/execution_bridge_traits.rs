//! Execution bridge trait for connecting operations to execution models.

use crate::error_handling::ErrorCode;
use crate::operation::Operation;
use std::future::Future;
use std::pin::Pin;

/// Trait for bridging operations to different execution models.
///
/// This trait provides a unified interface for converting operations
/// into different execution forms (sync, async, polling).
pub trait ExecutionBridge<T> {
    /// Convert the operation into a blocking function.
    ///
    /// Returns a closure that will execute the operation synchronously
    /// when called, blocking until completion.
    fn into_blocking(self) -> Box<dyn FnOnce() -> Result<T, ErrorCode> + Send>;

    /// Convert the operation into an async Future.
    ///
    /// Returns a Future that will resolve when the operation completes.
    #[cfg(feature = "async")]
    fn into_async(self) -> Pin<Box<dyn Future<Output = Result<T, ErrorCode>> + Send>>;

    /// Convert the operation into a polling function.
    ///
    /// Returns a closure that can be called repeatedly to poll the
    /// operation's status without blocking.
    fn into_polling(self) -> Box<dyn FnMut() -> crate::execution_context::OperationResult<T> + Send>;
}
