// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![deny(
	clippy::unwrap_used,
	clippy::expect_used,
	clippy::panic,
	clippy::unreachable,
	clippy::todo,
	clippy::unimplemented
)]

pub mod i2c;
pub mod scu;
pub mod smc;
pub mod uart;
