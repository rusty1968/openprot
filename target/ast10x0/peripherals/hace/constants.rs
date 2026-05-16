// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! HACE command and algorithm constants shared across hash and digest modules.

pub const HACE_SHA_BE_EN: u32 = 1 << 3;
pub const HACE_CMD_ACC_MODE: u32 = 1 << 8;
pub const HACE_SG_EN: u32 = 1 << 18;
pub const HACE_SG_LAST: u32 = 1 << 31;
pub const HACE_ALGO_SHA256: u32 = (1 << 4) | (1 << 6);
pub const HACE_ALGO_SHA512: u32 = (1 << 5) | (1 << 6);
pub const HACE_ALGO_SHA384: u32 = (1 << 5) | (1 << 6) | (1 << 10);
pub const SHA256_HASH_CMD: u32 = HACE_CMD_ACC_MODE | HACE_SHA_BE_EN | HACE_SG_EN | HACE_ALGO_SHA256;
pub const SHA384_HASH_CMD: u32 = HACE_CMD_ACC_MODE | HACE_SHA_BE_EN | HACE_SG_EN | HACE_ALGO_SHA384;
pub const SHA512_HASH_CMD: u32 = HACE_CMD_ACC_MODE | HACE_SHA_BE_EN | HACE_SG_EN | HACE_ALGO_SHA512;

pub const DEFAULT_POLL_BUDGET: u32 = 1_000_000;
