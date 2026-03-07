# aspeed-rust UART Backend Compatibility

This document describes the changes needed in aspeed-rust to make the UART console backend work when switching from the `pigweed-drv` branch to the `i2c-core` branch.

## Current Configuration

The Bazel build fetches aspeed-rust from GitHub using the configuration in [third_party/aspeed_ddk.bzl](../third_party/aspeed_ddk.bzl):

```python
git_repository(
    name = "aspeed_ddk",
    remote = "https://github.com/stevenlee7189/aspeed-rust.git",
    branch = "pigweed-drv",
    build_file = "@@//third_party:aspeed_ddk.BUILD.bazel",
)
```

## Problem Statement

The console backend in [target/ast1060-evb/console_backend.rs](../target/ast1060-evb/console_backend.rs) expects this API:

```rust
use aspeed_ddk::uart::{Config, Parity, StopBits, UartController};
```

However, the `i2c-core` branch of aspeed-rust exports a different module:

```rust
pub mod uart_core;  // NOT `pub mod uart;`
```

This causes a build error:
```
error[E0432]: unresolved import `aspeed_ddk::uart`
 --> target/ast1060-evb/console_backend.rs:15:17
  |
  | use aspeed_ddk::uart::{Config, Parity, StopBits, UartController};
  |                 ^^^^ could not find `uart` in `aspeed_ddk`
```

## API Differences

### `pigweed-drv` branch (`src/uart.rs`)

Single-file module with:

```rust
pub struct Config {
    pub baud_rate: u32,
    pub word_length: u8,  // 0=5, 1=6, 2=7, 3=8
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub clock: u32,
}

pub enum Parity { None, Odd, Even }
pub enum StopBits { One, OnePointFive, Two }

pub struct UartController<'a> {
    uart: Uart,
    delay: &'a mut dyn DelayNs,
}

impl<'a> UartController<'a> {
    pub fn new(uart: Uart, delay: &'a mut dyn DelayNs) -> Self;
    pub unsafe fn init(&self, config: &Config);
}
```

### `i2c-core` branch (`src/uart_core/`)

Modular directory structure with:

```rust
pub use config::{BaudRate, Parity, StopBits, UartConfig, WordLength};
pub use controller::UartController;

pub struct UartConfig {
    pub baud_rate: BaudRate,
    pub word_length: WordLength,
    pub parity: Parity,
    pub stop_bits: StopBits,
}

pub enum BaudRate { B9600, B19200, B38400, B57600, B115200, Custom(u32) }
pub enum WordLength { Five, Six, Seven, Eight }
pub enum Parity { None, Odd, Even }
pub enum StopBits { One, OnePointFive, Two }

pub struct UartController { /* ... */ }

impl UartController {
    pub fn new(uart: UART, config: UartConfig, clock_hz: u32) -> Self;
}
```

## Required Changes

### Option A: Add Compatibility Alias in aspeed-rust (Recommended)

Add a `uart` module to `i2c-core` branch that re-exports `uart_core` types with the expected API:

**`src/uart.rs`** (new file in aspeed-rust):
```rust
//! Compatibility layer for uart_core module.
//! Provides the legacy `uart` API expected by existing code.

pub use crate::uart_core::{Parity, StopBits, UartController};
pub use crate::uart_core::UartConfig as Config;

// If UartController API differs, add adapter methods or wrapper struct
```

**`src/lib.rs`** (modification):
```rust
pub mod uart_core;
pub mod uart;  // Add this line
```

### Option B: Update console_backend.rs (Alternative)

Modify the console backend to use the `uart_core` API directly:

```rust
// Before (pigweed-drv API)
use aspeed_ddk::uart::{Config, Parity, StopBits, UartController};

let config = Config {
    baud_rate: 115200,
    word_length: 3,
    parity: Parity::None,
    stop_bits: StopBits::One,
    clock: 24_000_000,
};
let controller = UartController::new(peripherals.uart, delay);
controller.init(&config);

// After (i2c-core API)
use aspeed_ddk::uart_core::{BaudRate, Parity, StopBits, UartConfig, UartController, WordLength};

let config = UartConfig {
    baud_rate: BaudRate::B115200,
    word_length: WordLength::Eight,
    parity: Parity::None,
    stop_bits: StopBits::One,
};
let controller = UartController::new(peripherals.uart, config, 24_000_000);
```

## Recommendation

**Option A** is recommended because:
1. Maintains backward compatibility with existing code
2. Single change point in aspeed-rust benefits all consumers
3. Allows gradual migration to new API

If aspeed-rust cannot be modified, **Option B** requires changes to:
- [target/ast1060-evb/console_backend.rs](../target/ast1060-evb/console_backend.rs)
- Any other code using `aspeed_ddk::uart`

## Switching Branches

To switch aspeed-rust from `pigweed-drv` to `i2c-core`, modify [third_party/aspeed_ddk.bzl](../third_party/aspeed_ddk.bzl):

```python
git_repository(
    name = "aspeed_ddk",
    remote = "https://github.com/stevenlee7189/aspeed-rust.git",
    branch = "i2c-core",  # Changed from "pigweed-drv"
    build_file = "@@//third_party:aspeed_ddk.BUILD.bazel",
)
```

Then clean the Bazel cache:
```bash
bazel clean --expunge
```
