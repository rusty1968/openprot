// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 ECDSA (Elliptic Curve Digital Signature) engine support.
//!
//! The engine is hosted by the Secure Boot Controller; see `registers` for the
//! Confined-`unsafe` MMIO façade. Driver/session layers land later under the
//! `peripheral-parity-port` workflow.

mod constants;
mod device;
mod error;
mod op;
mod registers;

pub use device::EcdsaDevice;
pub use error::EcdsaError;
pub use op::EcdsaOp;
pub use registers::EcdsaRegisters;
