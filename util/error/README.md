# util_error

Precise error handling for firmware across the project.

This crate provides a unified error type that encodes errors as `u32` values with bitfield structure. This approach offers several advantages in embedded systems:

- **Simplified Propagation and Handling**: Errors can be passed up the call chain without conversions or unique handling at different layers.
- **Global Uniqueness and Traceability**: Error values identify an error category, module, and specific fault, enabling easy tracing to the source.
- **Reduced Memory Footprint**: Errors are emitted as integers rather than descriptive strings, which are then translated by external tooling.
- **Enhanced Automated Diagnostics**: Monitoring systems can examine precise errors as events.
- **Improved API Consistency**: All error results are standardized around a single type.
- **Backward Compatibility**: Existing HALs and drivers can be gradually migrated through the `ToPreciseError` trait.

## Error Structure

A `u32` error value is subdivided into:

- **Bits 31-30**: Error category (status code)
- **Bits 29-16**: Module identifier (14 bits)
- **Bit 31** (in module field): Extended module flag
- **Bits 15-0**: Error code within the module

## Example Usage

```rust
use util_error::{Error, ErrorModule, define_error_module};

// Define error module using macro
define_error_module!(
    pub const ERR_ECDSA: 0x1234,
    pub const ERR_ECDSA_BUSY = 1,
    pub const ERR_ECDSA_INVALID_SIGNATURE = 2,
    pub const ERR_ECDSA_KEYGEN = 3,
);

// Or manual definition
pub const ERR_CRYPTO: ErrorModule = ErrorModule::new(0x0400);
pub const ERR_CRYPTO_INVALID_KEY: Error = ERR_CRYPTO.error(1);

// Use in functions
fn sign_data(data: &[u8]) -> util_error::Result<[u8; 64]> {
    if is_busy() {
        return Err(ERR_ECDSA_BUSY);
    }
    // ... implementation
    Ok([0u8; 64])
}
```

## ToPreciseError Trait Compatibility

For gradual migration of existing code, the crate provides:

1. **`IntoError` trait**: Convert any error type to `Error`
2. **`ToPreciseError` trait**: Bridge domain-specific error types
3. **`From` implementations**: Automatic conversion from standard types

Example: Integrating with an existing HAL

```rust
use util_error::{Error, ErrorModule, ToPreciseError};

#[derive(Debug)]
pub enum UartError {
    Timeout,
    FramingError,
}

impl ToPreciseError for UartError {
    fn to_precise_error(&self) -> Error {
        match self {
            UartError::Timeout => ERR_UART_TIMEOUT,
            UartError::FramingError => ERR_UART_FRAMING,
        }
    }
}

impl From<UartError> for Error {
    fn from(err: UartError) -> Self {
        err.to_precise_error()
    }
}

// Now existing HAL code can be wrapped:
pub fn read_byte() -> util_error::Result<u8> {
    hal_read_byte().map_err(|e| e.into())
}
```

See [MIGRATION.md](MIGRATION.md) for detailed integration patterns and strategies.

## Status Codes

The error module includes the following status codes (inspired by Google's Abseil StatusCode):

- `OK` (0) - Operation succeeded
- `CANCELLED` (1) - Operation was cancelled
- `UNKNOWN` (2) - Unknown error occurred
- `INVALID_ARGUMENT` (3) - Argument was malformed
- `DEADLINE_EXCEEDED` (4) - Deadline passed
- `NOT_FOUND` (5) - Entity not found
- `ALREADY_EXISTS` (6) - Entity already present
- `PERMISSION_DENIED` (7) - Permission denied
- `RESOURCE_EXHAUSTED` (8) - Insufficient resources
- `FAILED_PRECONDITION` (9) - System not in required state
- `ABORTED` (10) - Operation aborted
- `OUT_OF_RANGE` (11) - Operation out of range
- `UNIMPLEMENTED` (12) - Operation not implemented
- `INTERNAL` (13) - Internal error
- `UNAVAILABLE` (14) - Operation unavailable
- `DATA_LOSS` (15) - Data loss occurred
- `UNAUTHENTICATED` (16) - Caller unauthenticated

## Features

- `defmt` - Support for `defmt::Format` derive on error types
- `std` - Enable `std::io::Error` conversions (for host tooling)

## API

### Types

- `Error` - Unified error value (newtype over `u32`)
- `ErrorModule` - Module namespace for error codes
- `ToPreciseError` - Trait for bridging existing error types
- `IntoError` - Trait for converting to `Error`
- `Result<T>` - Shorthand for `core::result::Result<T, Error>`
- `Status` - Enumeration of error status codes

### Macros

- `define_error_module!` - Ergonomic error definition macro

## Example: HAL Integration

See [examples.rs](examples.rs) for a complete example of integrating an I2C HAL with the error system while maintaining backward compatibility.
