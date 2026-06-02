// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 Serial GPIO Matrix (SGPIOM) peripheral driver.

mod controller;
mod hal_impl;
mod register_block;
mod types;

pub use controller::SgpiomController;
pub use hal_impl::{SgpiomBankPort, SgpiomMask};
pub use register_block::Sgpiom;
pub use types::{
    Bank, BankDevice, Direction, Error, InitialLevel, InterruptMode, InterruptTrigger,
    SgpiomPinConfig,
};

