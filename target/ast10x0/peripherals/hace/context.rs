// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Internal context structures for HACE hashing.

use core::cell::UnsafeCell;

pub(crate) const SHA256_DIGEST_SIZE: usize = 32;
pub(crate) const SHA384_DIGEST_SIZE: usize = 48;
pub(crate) const SHA512_DIGEST_SIZE: usize = 64;
/// Block size for SHA-1/224/256.
pub(crate) const HACE_BLOCK_SIZE: usize = 64;
/// Block size for SHA-384/512.
pub(crate) const HACE_BLOCK_SIZE_128: usize = 128;
pub(crate) const HACE_BUFFER_SIZE: usize = 256;

pub(crate) const SHA256_IV: [u32; 8] = [
    0x67e6_096a,
    0x85ae_67bb,
    0x72f3_6e3c,
    0x3af5_4fa5,
    0x7f52_0e51,
    0x8c68_059b,
    0xabd9_831f,
    0x19cd_e05b,
];

/// SHA-384 IV, verbatim from the pinned Zephyr `hash_aspeed_priv.h` (`sha384_iv`).
pub(crate) const SHA384_IV: [u32; 16] = [
    0x5d9d_bbcb,
    0xd89e_05c1,
    0x2a29_9a62,
    0x07d5_7c36,
    0x5a01_5991,
    0x17dd_7030,
    0xd8ec_2f15,
    0x3959_0ef7,
    0x6726_3367,
    0x310b_c0ff,
    0x874a_b48e,
    0x1115_5868,
    0x0d2e_0cdb,
    0xa78f_f964,
    0x1d48_b547,
    0xa44f_fabe,
];

/// SHA-512 IV, verbatim from the pinned Zephyr `hash_aspeed_priv.h` (`sha512_iv`).
pub(crate) const SHA512_IV: [u32; 16] = [
    0x67e6_096a,
    0x08c9_bcf3,
    0x85ae_67bb,
    0x3ba7_ca84,
    0x72f3_6e3c,
    0x2bf8_94fe,
    0x3af5_4fa5,
    0xf136_1d5f,
    0x7f52_0e51,
    0xd182_e6ad,
    0x8c68_059b,
    0x1f6c_3e2b,
    0xabd9_831f,
    0x6bbd_41fb,
    0x19cd_e05b,
    0x7921_7e13,
];

pub(crate) const DIGEST_BUFFER_SIZE: usize = 64;
pub(crate) const KEY_BUFFER_SIZE: usize = 128;

#[derive(Copy, Clone)]
#[repr(C)]
pub(crate) struct Sg {
    pub(crate) len: u32,
    pub(crate) addr: u32,
}

impl Sg {
    pub const fn new() -> Self {
        Self { len: 0, addr: 0 }
    }
}

#[repr(C, align(64))]
pub(crate) struct HashContext {
    pub(crate) sg: [Sg; 2],
    pub(crate) digest: [u8; DIGEST_BUFFER_SIZE],
    pub(crate) method: u32,
    pub(crate) block_size: u32,
    pub(crate) key: [u8; KEY_BUFFER_SIZE],
    pub(crate) key_len: u32,
    pub(crate) ipad: [u8; KEY_BUFFER_SIZE],
    pub(crate) opad: [u8; KEY_BUFFER_SIZE],
    pub(crate) digcnt: [u64; 2],
    pub(crate) bufcnt: u32,
    pub(crate) buffer: [u8; HACE_BUFFER_SIZE],
    pub(crate) iv_size: u8,
}

impl HashContext {
    pub const fn new() -> Self {
        Self {
            sg: [Sg::new(), Sg::new()],
            digest: [0; DIGEST_BUFFER_SIZE],
            method: 0,
            block_size: 0,
            key: [0; KEY_BUFFER_SIZE],
            key_len: 0,
            ipad: [0; KEY_BUFFER_SIZE],
            opad: [0; KEY_BUFFER_SIZE],
            digcnt: [0; 2],
            bufcnt: 0,
            buffer: [0; HACE_BUFFER_SIZE],
            iv_size: 0,
        }
    }
}

#[allow(dead_code)]
pub(crate) struct SectionPlacedContext(UnsafeCell<HashContext>);

// SAFETY: HACE is owned by a single-threaded driver; access is serialized by the caller.
unsafe impl Sync for SectionPlacedContext {}

impl SectionPlacedContext {
    pub const fn new() -> Self {
        Self(UnsafeCell::new(HashContext::new()))
    }

    pub fn get(&self) -> *mut HashContext {
        self.0.get()
    }
}

#[unsafe(link_section = ".ram_nc")]
static SHARED_HASH_CTX: SectionPlacedContext = SectionPlacedContext::new();

/// Acquire the raw pointer to the section-placed hash context, to be held as
/// private state by the one [`HaceDevice`](super::device::HaceDevice).
///
/// This is the *only* path to the context. There is deliberately no free
/// accessor that hands the pointer to arbitrary call sites: the operation
/// state is reached exclusively *through* the borrowed device (every
/// `HaceDigest`/`HaceHmac` reborrows it under the device's `&mut`), which is
/// what makes engine exclusivity borrow-arbitrated rather than caller
/// discipline (`design-patterns` :: `borrow-arbitrated-engine-exclusivity`,
/// Checklist box 2).
///
/// The context must remain a `.ram_nc`, `#[repr(C, align(64))]` static — it
/// holds the SG list / `buffer` / `digest` DMA targets and cannot live on a
/// stack-placed device value (`goal.md` §1.3/§5.1). That residual static is
/// the pattern's stated hardware liability ("language fiction, not a hardware
/// lock"); single-instance is gate-delegated to the `unsafe fn new*` contract
/// below (Checklist box 3), exactly as the sibling SBC port does.
///
/// # Safety
/// The HACE engine is a hardware singleton. The caller (the `HaceDevice`
/// construction gate) must uphold the same single-instance/non-reentrancy
/// contract as `HaceRegisters::new*`: at most one live `HaceDevice`, hence at
/// most one live `&mut` minted from this pointer, at a time.
pub(crate) unsafe fn acquire_shared_ctx() -> *mut HashContext {
    SHARED_HASH_CTX.get()
}
