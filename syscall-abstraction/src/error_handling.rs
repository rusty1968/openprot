// Licensed under the Apache-2.0 license

//! Error handling types for the syscall abstraction.

/// Error codes for syscall operations.
///
/// This enum provides OS-agnostic error codes that can be translated
/// to and from OS-specific error types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// Operation completed successfully (should not be used in error contexts)
    Success,
    /// Generic failure
    Fail,
    /// Operation would block (used in polling contexts)
    WouldBlock,
    /// Operation is busy/in progress
    Busy,
    /// Operation was already completed
    Already,
    /// Operation was cancelled
    Cancel,
    /// Operation timed out
    Timeout,
    /// No memory available
    NoMemory,
    /// Not supported by this implementation
    NotSupported,
    /// Invalid argument provided
    InvalidArgument,
    /// Invalid driver number
    InvalidDriver,
    /// Invalid command number
    InvalidCommand,
    /// Invalid subscription number
    InvalidSubscription,
    /// Service is not available
    ServiceUnavailable,
    /// Buffer is too large
    BufferTooLarge,
    /// Invalid buffer alignment
    InvalidAlignment,
    /// Permission denied
    PermissionDenied,
    /// Resource not found
    NotFound,
    /// Resource already exists
    AlreadyExists,
    /// Invalid state for operation
    InvalidState,
    /// Invalid handle provided
    InvalidHandle,
    /// Hardware error
    HardwareError,
    /// Network error
    NetworkError,
    /// File system error
    FileSystemError,
    /// Cryptographic error
    CryptoError,
    /// Security violation
    SecurityError,
    /// Resource exhausted
    ResourceExhausted,
    /// Operation interrupted
    Interrupted,
    /// Internal error
    InternalError,
    /// System error
    SystemError,
    /// Device not found
    DeviceNotFound,
    /// Operation failed
    OperationFailed,
}

impl ErrorCode {
    /// Check if this error represents a temporary condition that might succeed on retry.
    pub fn is_temporary(&self) -> bool {
        matches!(self, 
            ErrorCode::WouldBlock |
            ErrorCode::Busy |
            ErrorCode::Timeout |
            ErrorCode::NoMemory |
            ErrorCode::ResourceExhausted |
            ErrorCode::Interrupted
        )
    }
    
    /// Check if this error represents a permanent condition that will not succeed on retry.
    pub fn is_permanent(&self) -> bool {
        !self.is_temporary()
    }
    
    /// Get a human-readable description of the error.
    pub fn description(&self) -> &'static str {
        match self {
            ErrorCode::Success => "Operation completed successfully",
            ErrorCode::Fail => "Generic failure",
            ErrorCode::WouldBlock => "Operation would block",
            ErrorCode::Busy => "Operation is busy",
            ErrorCode::Already => "Operation was already completed",
            ErrorCode::Cancel => "Operation was cancelled",
            ErrorCode::Timeout => "Operation timed out",
            ErrorCode::NoMemory => "No memory available",
            ErrorCode::NotSupported => "Not supported by this implementation",
            ErrorCode::InvalidArgument => "Invalid argument provided",
            ErrorCode::InvalidDriver => "Invalid driver number",
            ErrorCode::InvalidCommand => "Invalid command number",
            ErrorCode::InvalidSubscription => "Invalid subscription number",
            ErrorCode::ServiceUnavailable => "Service is not available",
            ErrorCode::BufferTooLarge => "Buffer is too large",
            ErrorCode::InvalidAlignment => "Invalid buffer alignment",
            ErrorCode::PermissionDenied => "Permission denied",
            ErrorCode::NotFound => "Resource not found",
            ErrorCode::AlreadyExists => "Resource already exists",
            ErrorCode::InvalidState => "Invalid state for operation",
            ErrorCode::InvalidHandle => "Invalid handle provided",
            ErrorCode::HardwareError => "Hardware error",
            ErrorCode::NetworkError => "Network error",
            ErrorCode::FileSystemError => "File system error",
            ErrorCode::CryptoError => "Cryptographic error",
            ErrorCode::SecurityError => "Security violation",
            ErrorCode::ResourceExhausted => "Resource exhausted",
            ErrorCode::Interrupted => "Operation interrupted",
            ErrorCode::InternalError => "Internal error",
            ErrorCode::SystemError => "System error",
            ErrorCode::DeviceNotFound => "Device not found",
            ErrorCode::OperationFailed => "Operation failed",
        }
    }
    
    /// Get the category of this error for grouping and analysis.
    pub fn category(&self) -> ErrorCategory {
        match self {
            ErrorCode::Success => ErrorCategory::Success,
            ErrorCode::Fail | ErrorCode::InternalError | ErrorCode::SystemError | ErrorCode::OperationFailed => ErrorCategory::Generic,
            ErrorCode::WouldBlock | ErrorCode::Busy | ErrorCode::Timeout => ErrorCategory::Temporary,
            ErrorCode::Already | ErrorCode::Cancel => ErrorCategory::State,
            ErrorCode::NoMemory | ErrorCode::ResourceExhausted => ErrorCategory::Resource,
            ErrorCode::NotSupported => ErrorCategory::Capability,
            ErrorCode::InvalidArgument | ErrorCode::InvalidDriver | ErrorCode::InvalidCommand | 
            ErrorCode::InvalidSubscription | ErrorCode::InvalidAlignment | ErrorCode::InvalidState | 
            ErrorCode::InvalidHandle => ErrorCategory::Invalid,
            ErrorCode::ServiceUnavailable | ErrorCode::NotFound | ErrorCode::AlreadyExists | ErrorCode::DeviceNotFound => ErrorCategory::Availability,
            ErrorCode::BufferTooLarge => ErrorCategory::Size,
            ErrorCode::PermissionDenied | ErrorCode::SecurityError => ErrorCategory::Security,
            ErrorCode::HardwareError => ErrorCategory::Hardware,
            ErrorCode::NetworkError => ErrorCategory::Network,
            ErrorCode::FileSystemError => ErrorCategory::FileSystem,
            ErrorCode::CryptoError => ErrorCategory::Crypto,
            ErrorCode::Interrupted => ErrorCategory::Interruption,
        }
    }
}

/// Categories of errors for grouping and analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    /// Successful operation (not an error)
    Success,
    /// Generic or unspecified error
    Generic,
    /// Temporary condition that might succeed on retry
    Temporary,
    /// State-related error (operation already done, cancelled, etc.)
    State,
    /// Resource-related error (memory, handles, etc.)
    Resource,
    /// Capability-related error (not supported, etc.)
    Capability,
    /// Invalid input or parameter
    Invalid,
    /// Availability-related error (service unavailable, not found, etc.)
    Availability,
    /// Size-related error (buffer too large, etc.)
    Size,
    /// Security-related error
    Security,
    /// Hardware-related error
    Hardware,
    /// Network-related error
    Network,
    /// File system-related error
    FileSystem,
    /// Cryptographic error
    Crypto,
    /// Interruption-related error
    Interruption,
}

/// Result type for syscall operations.
pub type Result<T> = core::result::Result<T, ErrorCode>;

/// Convert OS-specific error codes to our generic ErrorCode.
pub trait FromOsError {
    /// Convert from an OS-specific error to our generic ErrorCode.
    fn from_os_error(error: Self) -> ErrorCode;
}

/// Convert our generic ErrorCode to OS-specific error codes.
pub trait ToOsError<T> {
    /// Convert our generic ErrorCode to an OS-specific error.
    fn to_os_error(&self) -> T;
}

// Tock-specific error conversions
#[cfg(feature = "tock")]
impl FromOsError for libtock_platform::ErrorCode {
    fn from_os_error(error: Self) -> ErrorCode {
        match error {
            libtock_platform::ErrorCode::Success => ErrorCode::Success,
            libtock_platform::ErrorCode::Fail => ErrorCode::Fail,
            libtock_platform::ErrorCode::Busy => ErrorCode::Busy,
            libtock_platform::ErrorCode::Already => ErrorCode::Already,
            libtock_platform::ErrorCode::Off => ErrorCode::InvalidState,
            libtock_platform::ErrorCode::Reserve => ErrorCode::ResourceExhausted,
            libtock_platform::ErrorCode::InvalidArgument => ErrorCode::InvalidArgument,
            libtock_platform::ErrorCode::Size => ErrorCode::BufferTooLarge,
            libtock_platform::ErrorCode::Cancel => ErrorCode::Cancel,
            libtock_platform::ErrorCode::NoMemory => ErrorCode::NoMemory,
            libtock_platform::ErrorCode::NoSupport => ErrorCode::NotSupported,
            libtock_platform::ErrorCode::NoDevice => ErrorCode::ServiceUnavailable,
            libtock_platform::ErrorCode::Uninstalled => ErrorCode::NotFound,
            libtock_platform::ErrorCode::NoAck => ErrorCode::Timeout,
        }
    }
}

#[cfg(feature = "tock")]
impl ToOsError<libtock_platform::ErrorCode> for ErrorCode {
    fn to_os_error(&self) -> libtock_platform::ErrorCode {
        match self {
            ErrorCode::Success => libtock_platform::ErrorCode::Success,
            ErrorCode::Fail => libtock_platform::ErrorCode::Fail,
            ErrorCode::Busy => libtock_platform::ErrorCode::Busy,
            ErrorCode::Already => libtock_platform::ErrorCode::Already,
            ErrorCode::Cancel => libtock_platform::ErrorCode::Cancel,
            ErrorCode::NoMemory => libtock_platform::ErrorCode::NoMemory,
            ErrorCode::NotSupported => libtock_platform::ErrorCode::NoSupport,
            ErrorCode::InvalidArgument => libtock_platform::ErrorCode::InvalidArgument,
            ErrorCode::BufferTooLarge => libtock_platform::ErrorCode::Size,
            ErrorCode::ServiceUnavailable => libtock_platform::ErrorCode::NoDevice,
            ErrorCode::NotFound => libtock_platform::ErrorCode::Uninstalled,
            ErrorCode::Timeout => libtock_platform::ErrorCode::NoAck,
            ErrorCode::InvalidState => libtock_platform::ErrorCode::Off,
            ErrorCode::ResourceExhausted => libtock_platform::ErrorCode::Reserve,
            _ => libtock_platform::ErrorCode::Fail,
        }
    }
}

// Standard library error conversions
#[cfg(feature = "std")]
impl From<std::io::Error> for ErrorCode {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            std::io::ErrorKind::AlreadyExists => ErrorCode::AlreadyExists,
            std::io::ErrorKind::WouldBlock => ErrorCode::WouldBlock,
            std::io::ErrorKind::InvalidInput => ErrorCode::InvalidArgument,
            std::io::ErrorKind::InvalidData => ErrorCode::InvalidArgument,
            std::io::ErrorKind::TimedOut => ErrorCode::Timeout,
            std::io::ErrorKind::Interrupted => ErrorCode::Interrupted,
            std::io::ErrorKind::UnexpectedEof => ErrorCode::Fail,
            std::io::ErrorKind::OutOfMemory => ErrorCode::NoMemory,
            _ => ErrorCode::Fail,
        }
    }
}

#[cfg(feature = "std")]
impl From<ErrorCode> for std::io::Error {
    fn from(error: ErrorCode) -> Self {
        let kind = match error {
            ErrorCode::NotFound => std::io::ErrorKind::NotFound,
            ErrorCode::PermissionDenied => std::io::ErrorKind::PermissionDenied,
            ErrorCode::AlreadyExists => std::io::ErrorKind::AlreadyExists,
            ErrorCode::WouldBlock => std::io::ErrorKind::WouldBlock,
            ErrorCode::InvalidArgument => std::io::ErrorKind::InvalidInput,
            ErrorCode::Timeout => std::io::ErrorKind::TimedOut,
            ErrorCode::Interrupted => std::io::ErrorKind::Interrupted,
            ErrorCode::NoMemory => std::io::ErrorKind::OutOfMemory,
            _ => std::io::ErrorKind::Other,
        };
        
        std::io::Error::new(kind, error.description())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_description() {
        assert_eq!(ErrorCode::Success.description(), "Operation completed successfully");
        assert_eq!(ErrorCode::Fail.description(), "Generic failure");
        assert_eq!(ErrorCode::NotFound.description(), "Resource not found");
    }

    #[test]
    fn test_error_code_temporary() {
        assert!(ErrorCode::WouldBlock.is_temporary());
        assert!(ErrorCode::Busy.is_temporary());
        assert!(ErrorCode::Timeout.is_temporary());
        assert!(!ErrorCode::InvalidArgument.is_temporary());
        assert!(!ErrorCode::NotSupported.is_temporary());
    }

    #[test]
    fn test_error_code_permanent() {
        assert!(!ErrorCode::WouldBlock.is_permanent());
        assert!(!ErrorCode::Busy.is_permanent());
        assert!(ErrorCode::InvalidArgument.is_permanent());
        assert!(ErrorCode::NotSupported.is_permanent());
    }

    #[test]
    fn test_error_category() {
        assert_eq!(ErrorCode::Success.category(), ErrorCategory::Success);
        assert_eq!(ErrorCode::Fail.category(), ErrorCategory::Generic);
        assert_eq!(ErrorCode::WouldBlock.category(), ErrorCategory::Temporary);
        assert_eq!(ErrorCode::InvalidArgument.category(), ErrorCategory::Invalid);
        assert_eq!(ErrorCode::NoMemory.category(), ErrorCategory::Resource);
        assert_eq!(ErrorCode::PermissionDenied.category(), ErrorCategory::Security);
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_std_error_conversion() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "test");
        let error_code = ErrorCode::from(io_error);
        assert_eq!(error_code, ErrorCode::NotFound);
        
        let io_error_back = std::io::Error::from(ErrorCode::NotFound);
        assert_eq!(io_error_back.kind(), std::io::ErrorKind::NotFound);
    }
}
