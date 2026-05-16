// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 HACE (Hash and Crypto Engine) peripheral support.

mod constants;
mod context;
mod digest;
mod error;
mod device;
mod helpers;
mod registers;

pub use digest::HaceDigest;
pub use error::HaceError;
pub use device::{HaceDevice, HashAlgo};
pub use registers::HaceRegisters;
