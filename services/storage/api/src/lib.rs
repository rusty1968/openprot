// Licensed under the Apache-2.0 license

//! Storage Service API
//!
//! Defines the IPC protocol and backend traits for the storage service.
//! Inspired by Caliptra MCU-SW's FlashStorage and BootConfig abstractions.

#![no_std]

pub mod backend;
pub mod protocol;

pub use protocol::*;
