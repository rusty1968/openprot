// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Flash device abstractions layered on top of SMC wrappers.

mod flash;

pub use flash::{FlashDevice, SpiNorFlash};
