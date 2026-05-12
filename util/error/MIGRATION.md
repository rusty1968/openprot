# ToPreciseError Trait Compatibility and Migration Guide

This document explains how to bridge existing error handling code with the unified `util_error` system, enabling gradual migration of the codebase.

## Overview

The `util_error` crate provides two integration points for existing code:

1. **`IntoError` trait**: Converts any error type into a precise `Error`
2. **`ToPreciseError` trait**: Bridges domain-specific error types with the error system

This enables:
- Gradual migration of existing HALs and drivers
- Compatibility with standard library error types
- Custom error type implementations

## Integration Patterns

### Pattern 1: Direct Result Type Conversion

For new code, use the `Result<T>` type alias directly:

```rust
use util_error::{Error, ErrorModule, Result};

pub const ERR_UART: ErrorModule = ErrorModule::new(0x0100);
pub const ERR_UART_TIMEOUT: Error = ERR_UART.error(1);
pub const ERR_UART_FRAMING: Error = ERR_UART.error(2);

pub fn send_byte(byte: u8) -> Result<()> {
    if is_timeout() {
        return Err(ERR_UART_TIMEOUT);
    }
    // ...
    Ok(())
}
```

### Pattern 2: HAL ErrorKind Migration

For existing HALs with `ErrorKind` enums, implement the `ToPreciseError` trait:

```rust
use util_error::{Error, ErrorModule, ToPreciseError};

pub const ERR_I2C: ErrorModule = ErrorModule::new(0x0200);
pub const ERR_I2C_NACK: Error = ERR_I2C.error(1);
pub const ERR_I2C_ARBITRATION_LOST: Error = ERR_I2C.error(2);
pub const ERR_I2C_BUS_ERROR: Error = ERR_I2C.error(3);

#[derive(Debug)]
pub enum I2cError {
    Nack,
    ArbitrationLost,
    BusError,
}

impl ToPreciseError for I2cError {
    fn to_precise_error(&self) -> Error {
        match self {
            I2cError::Nack => ERR_I2C_NACK,
            I2cError::ArbitrationLost => ERR_I2C_ARBITRATION_LOST,
            I2cError::BusError => ERR_I2C_BUS_ERROR,
        }
    }
}

impl From<I2cError> for Error {
    fn from(err: I2cError) -> Self {
        err.to_precise_error()
    }
}

// Now I2cError can be converted to Error automatically
pub fn read_register() -> Result<u8> {
    let value = i2c_read().map_err(|e: I2cError| e.into_error())?;
    Ok(value)
}
```

### Pattern 3: Layered Error Handling

For multi-layer APIs, define errors at each level and compose them:

```rust
use util_error::{Error, ErrorModule};

// Low-level HAL errors
pub const ERR_GPIO: ErrorModule = ErrorModule::new(0x0050);
pub const ERR_GPIO_INVALID_PIN: Error = ERR_GPIO.error(1);

// Mid-level driver errors
pub const ERR_SENSOR: ErrorModule = ErrorModule::new(0x0300);
pub const ERR_SENSOR_NOT_READY: Error = ERR_SENSOR.error(1);
pub const ERR_SENSOR_READ_FAILED: Error = ERR_SENSOR.error(2);

// Application layer can map any of these to a single type
pub fn read_sensor() -> util_error::Result<u16> {
    // Errors from any layer propagate through the same Result<T> type
    initialize_gpio()?;
    acquire_sensor_lock()?;
    let value = read_sensor_raw()?;
    Ok(value)
}
```

### Pattern 4: Standard Library Error Bridging

The crate provides `From` implementations for common types:

```rust
use util_error::Error;
use std::io;

// Automatic conversion from std::io::Error
fn write_file(path: &str) -> util_error::Result<()> {
    let _file = std::fs::File::create(path)?;
    Ok(())
}
```

Mappings:
- `io::ErrorKind::NotFound` → `NOT_FOUND` status
- `io::ErrorKind::PermissionDenied` → `PERMISSION_DENIED` status
- `io::ErrorKind::InvalidInput` → `INVALID_ARGUMENT` status
- `io::ErrorKind::TimedOut` → `DEADLINE_EXCEEDED` status
- `io::ErrorKind::Interrupted` → `ABORTED` status
- Others → `INTERNAL` status

### Pattern 5: External HAL Compatibility (embedded-hal 1.0 style)

For HALs written by other teams or vendors, adapt at the boundary. Do not modify
the upstream HAL crate; wrap and map its error type in your integration crate.

```rust
use embedded_hal::spi;
use util_error::{Error, ErrorModule, Result};

pub const ERR_SPI: ErrorModule = ErrorModule::new(0x0220);
pub const ERR_SPI_OVERRUN: Error = ERR_SPI.error(1);
pub const ERR_SPI_MODE_FAULT: Error = ERR_SPI.error(2);
pub const ERR_SPI_OTHER: Error = ERR_SPI.error(3);

// Generic adapter over any embedded-hal 1.0 SPI bus.
pub struct SpiAdapter<H> {
    hal: H,
}

impl<H> SpiAdapter<H> {
    pub fn new(hal: H) -> Self {
        Self { hal }
    }
}

// Map embedded-hal SPI error kinds to unified util_error::Error.
fn map_spi_kind(kind: spi::ErrorKind) -> Error {
    match kind {
        spi::ErrorKind::Overrun => ERR_SPI_OVERRUN,
        spi::ErrorKind::ModeFault => ERR_SPI_MODE_FAULT,
        _ => ERR_SPI_OTHER,
    }
}

impl<H> SpiAdapter<H>
where
    H: spi::SpiBus<u8>,
{
    pub fn transfer_byte(&mut self, byte: u8) -> Result<u8> {
        let mut buf = [byte];
        self.hal
            .transfer_in_place(&mut buf)
            .map_err(|e| map_spi_kind(e.kind()))?;
        Ok(buf[0])
    }
}
```

Note: `embedded-hal` 1.0 does not define UART traits. For UART compatibility,
use your vendor/UART HAL traits directly or `embedded-io` traits and add a
boundary adapter that maps vendor UART errors into `util_error::Error`.

Why this is compatible with external HALs:
- No edits to third-party HAL source.
- Conversion is centralized in one adapter layer.
- Internal code uses only `util_error::Result<T>`.
- Migration is incremental and module-by-module.

## Macro: `define_error_module!`

For ergonomic definition of module-specific errors:

```rust
use util_error::define_error_module;

define_error_module!(
    pub const ERR_CRYPTO: 0x0400,
    pub const ERR_CRYPTO_INVALID_KEY = 1,
    pub const ERR_CRYPTO_INVALID_PLAINTEXT = 2,
    pub const ERR_CRYPTO_OPERATION_FAILED = 3,
);

// Equivalent to:
// pub const ERR_CRYPTO: ErrorModule = ErrorModule::new(0x0400);
// pub const ERR_CRYPTO_INVALID_KEY: Error = ERR_CRYPTO.error(1);
// pub const ERR_CRYPTO_INVALID_PLAINTEXT: Error = ERR_CRYPTO.error(2);
// pub const ERR_CRYPTO_OPERATION_FAILED: Error = ERR_CRYPTO.error(3);
```

## Migration Strategy

### Phase 1: Establish Error Infrastructure
1. Define module IDs for each major subsystem
2. Implement `ToPreciseError` trait for existing error enums
3. Update CI/CD to validate error uniqueness

### Phase 2: Gradual Module Migration
For each module:
1. Implement `From<ModuleError> for Error`
2. Update public APIs to return `util_error::Result<T>`
3. Add tests verifying error propagation
4. Update documentation

### Phase 3: Full Integration
1. Consolidate all error definitions
2. Remove legacy error types where feasible
3. Update monitoring/diagnostics to consume precise errors
4. Deploy uniqueness tooling

## Example: Migrating an Existing Driver

**Before:**
```rust
pub enum UartError {
    Timeout,
    BufferFull,
    InvalidConfig,
}

pub type Result<T> = core::result::Result<T, UartError>;

pub fn configure(baudrate: u32) -> Result<()> {
    if baudrate == 0 {
        return Err(UartError::InvalidConfig);
    }
    Ok(())
}
```

**After:**
```rust
use util_error::{Error, ErrorModule, define_error_module};

define_error_module!(
    pub const ERR_UART: 0x0100,
    pub const ERR_UART_TIMEOUT = 1,
    pub const ERR_UART_BUFFER_FULL = 2,
    pub const ERR_UART_INVALID_CONFIG = 3,
);

pub enum UartError {
    Timeout,
    BufferFull,
    InvalidConfig,
}

impl From<UartError> for Error {
    fn from(err: UartError) -> Self {
        match err {
            UartError::Timeout => ERR_UART_TIMEOUT,
            UartError::BufferFull => ERR_UART_BUFFER_FULL,
            UartError::InvalidConfig => ERR_UART_INVALID_CONFIG,
        }
    }
}

pub fn configure(baudrate: u32) -> util_error::Result<()> {
    if baudrate == 0 {
        return Err(ERR_UART_INVALID_CONFIG);
    }
    Ok(())
}
```

## Backward Compatibility

The `ToPreciseError` trait and conversion traits ensure:

- Existing `Result` types with custom error enums can be wrapped with `.map_err(|e| e.into_error())?`
- New code uses `util_error::Result<T>` directly
- Both patterns can coexist during migration
- No breaking changes to public APIs during transition

## Testing

Tests should verify:

1. **Error Creation**: Module and code combination produces expected values
2. **Error Propagation**: `?` operator and `map_err` work correctly
3. **Status Mapping**: Standard library conversions map correctly
4. **Uniqueness**: No duplicate error codes within a module (enforced by CI tooling)

```rust
#[test]
fn test_error_mapping() {
    let uart_err = UartError::Timeout;
    let precise_err: Error = uart_err.into();
    assert_eq!(precise_err.module(), 0x0100);
    assert_eq!(precise_err.code(), 1);
}
```
