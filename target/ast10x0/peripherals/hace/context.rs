// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Internal context structures for HACE hashing.

use core::cell::UnsafeCell;

pub(crate) const SHA256_DIGEST_SIZE: usize = 32;
pub(crate) const HACE_BLOCK_SIZE: usize = 64;
pub(crate) const HACE_BUFFER_SIZE: usize = 256;

/// Largest input size that still fits in `HACE_BUFFER_SIZE` after SHA-256 padding.
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

/// Returns a raw pointer to the section-placed shared hash context.
///
/// # Safety
/// Caller must ensure exclusive access — no concurrent or reentrant use.
pub(crate) fn shared_ctx_ptr() -> *mut HashContext {
    SHARED_HASH_CTX.get()
}
