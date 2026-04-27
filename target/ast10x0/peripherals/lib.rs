// Licensed under the Apache-2.0 license

#![no_std]

#[path = "uart/mod.rs"]
pub mod uart_core;

pub use uart_core as uart;