// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AES over the HACE crypto sub-engine (AES-128/256, ECB + CBC, raw key).
//!
//! Behavioral authority: the pinned Zephyr `aspeed_hace` unified driver —
//! `zephyr-reference/{hace_aspeed.c,hace_aspeed.h,crypto_aspeed_priv.h}` @
//! `cfe94dc`; see `plans/goal.md` §1.9 (reference behavior), §2.3 (deltas),
//! §2.4 (NIST correctness authority). Correctness is gated by NIST
//! AESAVS/CAVP KATs, not by byte-parity with the driver's `IV ‖ CT` framing
//! (delta A2).
//!
//! API shape: ADR-A1 (goal.md §3 item 6) — this is the **trait-free,
//! slice-based driver core** on the borrow-arbitrated op, exactly as
//! `SbcOp::{verify_raw,modexp}`. `AesCipher` is obtained *only* by an
//! exclusive `&mut HaceDevice` borrow-split, so AES (the engine's third
//! operation) cannot overlap a digest/HMAC op — a borrow-check error, not a
//! runtime `in_use` flag and not a HW busy-bit read (delta A1 /
//! `design-patterns :: borrow-arbitrated-engine-exclusivity`).
//!
//! Intentional deltas vs. the driver (goal.md §2.3): **A2** the ciphertext
//! buffer is plain `CT` (no in-band IV prefix/strip — the IV is a separate
//! argument, matching the openprot cipher trait shape); **A3** the key/IV
//! region of the `.ram_nc` context is zeroized after every op and on drop;
//! **A4** a non-block-multiple input is rejected with a typed `InvalidInput`
//! before the engine is programmed (a bound the C omits). AES-192 and the
//! OTP/secret-vault key path are out of scope by decision (delta A5).

use super::constants::{
    AES_CMD_BASE, HACE_CMD_AES128, HACE_CMD_AES256, HACE_CMD_CBC, HACE_CMD_ECB, HACE_CMD_ENCRYPT,
    HACE_SG_LAST, POLL_YIELD_NS,
};
use super::context::CryptoContext;
use super::error::HaceError;
use super::helpers::ptr_to_u32;
use super::registers::HaceRegisters;

/// The AES block size, in bytes. ECB/CBC inputs must be a multiple of this
/// (delta A4).
pub const AES_BLOCK: usize = 16;

/// Borrow-arbitrated AES operation over the HACE crypto sub-engine.
///
/// Built only via [`AesCipher::from_device`]; the retained `&'a mut` borrow of
/// the device's `yield_fn` pins `&'a mut HaceDevice` for the op's life, so no
/// second HACE operation (AES, digest, or HMAC) can run concurrently.
pub struct AesCipher<'a> {
    regs: HaceRegisters,
    ctx: &'a mut CryptoContext,
    poll_budget: u32,
    /// Cooperative yield hook, borrowed from the originating `HaceDevice` and
    /// invoked once between every completion poll. Type-erased so the adapter
    /// need not be generic over the strategy (same shape as `HaceDigest`).
    yield_fn: &'a mut dyn FnMut(u32),
}

impl<'a> AesCipher<'a> {
    pub(crate) fn new(
        regs: HaceRegisters,
        ctx: &'a mut CryptoContext,
        poll_budget: u32,
        yield_fn: &'a mut dyn FnMut(u32),
    ) -> Self {
        Self {
            regs,
            ctx,
            poll_budget,
            yield_fn,
        }
    }

    /// Construct an AES adapter from a [`HaceDevice`](super::device::HaceDevice).
    ///
    /// # Safety
    /// Caller must ensure no concurrent or reentrant HACE access for the
    /// lifetime of the returned [`AesCipher`] (the same gate-delegated
    /// single-instance contract as `HaceDigest::from_device`).
    pub unsafe fn from_device<Y: FnMut(u32)>(
        device: &'a mut super::device::HaceDevice<Y>,
    ) -> Self {
        // Borrow split. `regs`/`poll_budget`/`crypto_ctx` are `Copy`d out; the
        // retained `&'a mut device.yield_fn` reborrow pins `&'a mut HaceDevice`
        // for the whole life of the returned op — the arbiter (delta A1).
        let regs = device.regs;
        let poll_budget = device.poll_budget;
        // SAFETY: the device holds the sole pointer to this `.ram_nc` crypto
        // context (acquired once at its `unsafe fn new*` single-instance gate);
        // the caller upholds non-reentrancy and the live `&'a mut device`
        // (pinned by `yield_fn` below) gates it, so no other `&mut` is live.
        let ctx: &'a mut CryptoContext = unsafe { &mut *device.crypto_ctx };
        let yield_fn: &'a mut dyn FnMut(u32) = &mut device.yield_fn;
        Self::new(regs, ctx, poll_budget, yield_fn)
    }

    /// AES key-length → command bits. AES-192 (24 B) is out of scope by
    /// decision (goal.md §2.3 delta A5) and is rejected here.
    fn keylen_bits(key: &[u8]) -> Result<u32, HaceError> {
        match key.len() {
            16 => Ok(HACE_CMD_AES128),
            32 => Ok(HACE_CMD_AES256),
            _ => Err(HaceError::InvalidInput),
        }
    }

    /// One-shot AES transform. `mode_bits` ∈ {`HACE_CMD_ECB`,`HACE_CMD_CBC`};
    /// `iv` is `Some` only for CBC. `input`/`output` are plain data buffers —
    /// **no in-band IV** (delta A2). On return (success *or* failure) the
    /// key/IV region of the context is zeroized (delta A3).
    fn crypt(
        &mut self,
        mode_bits: u32,
        encrypt: bool,
        key: &[u8],
        iv: Option<&[u8; AES_BLOCK]>,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(), HaceError> {
        // Delta A4: the block-multiple / sizing bound the authority's
        // `aspeed_aes_crypt` omits — reject before programming the engine.
        if input.is_empty()
            || input.len() % AES_BLOCK != 0
            || output.len() < input.len()
        {
            return Err(HaceError::InvalidInput);
        }
        let kbits = Self::keylen_bits(key)?;
        let len = u32::try_from(input.len()).map_err(|_| HaceError::InvalidInput)?;

        // Engine context: IV at ctx[0..16) (CBC only), raw key at
        // ctx[16..16+keylen) — `hace_aspeed.c:114,186,200` (goal.md §1.9.3).
        self.ctx.ctx = [0u8; 64];
        if let Some(iv) = iv {
            self.ctx.ctx[..AES_BLOCK].copy_from_slice(iv);
        }
        self.ctx.ctx[AES_BLOCK..AES_BLOCK + key.len()].copy_from_slice(key);

        // SG descriptors: addr = data phys, len = bytes | HACE_SG_LAST
        // (`hace_aspeed.c:132-133`; single/last entry).
        let in_ptr = ptr_to_u32(input.as_ptr())?;
        let out_ptr = ptr_to_u32(output.as_ptr())?;
        self.ctx.src.addr = in_ptr;
        self.ctx.src.len = len | HACE_SG_LAST;
        self.ctx.dst.addr = out_ptr;
        self.ctx.dst.len = len | HACE_SG_LAST;

        let cmd = AES_CMD_BASE
            | kbits
            | mode_bits
            | if encrypt { HACE_CMD_ENCRYPT } else { 0 };
        self.ctx.cmd = cmd;

        // The engine takes the *addresses of the SG descriptors* and the ctx
        // buffer, and `crypto_data_len = src.len` (with the terminator bit) —
        // `hace_aspeed.c:90-94` (goal.md §1.9.1).
        let src_desc = ptr_to_u32(core::ptr::addr_of!(self.ctx.src))?;
        let dst_desc = ptr_to_u32(core::ptr::addr_of!(self.ctx.dst))?;
        let ctx_base = ptr_to_u32(self.ctx.ctx.as_ptr())?;
        let data_len = self.ctx.src.len;

        self.regs.clear_crypto_intflag();
        self.regs
            .program_crypto_operation(src_desc, dst_desc, ctx_base, data_len, cmd);

        let mut done = false;
        for _ in 0..self.poll_budget {
            if self.regs.crypto_intflag_is_set() {
                done = true;
                break;
            }
            (self.yield_fn)(POLL_YIELD_NS);
        }

        // Delta A3: never leave key/IV resident in the `.ram_nc` DMA buffer,
        // on success or on timeout.
        self.ctx.ctx = [0u8; 64];

        if done {
            Ok(())
        } else {
            Err(HaceError::Timeout)
        }
    }

    /// AES-ECB encrypt. `key` is 16 or 32 bytes; `pt.len()` must be a non-zero
    /// multiple of 16; `ct.len() >= pt.len()`. `ct` receives exactly the
    /// ciphertext (delta A2).
    pub fn ecb_encrypt(&mut self, key: &[u8], pt: &[u8], ct: &mut [u8]) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_ECB, true, key, None, pt, ct)
    }

    /// AES-ECB decrypt (inverse of [`ecb_encrypt`](Self::ecb_encrypt)).
    pub fn ecb_decrypt(&mut self, key: &[u8], ct: &[u8], pt: &mut [u8]) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_ECB, false, key, None, ct, pt)
    }

    /// AES-CBC encrypt with the given 16-byte `iv`. `ct` receives exactly the
    /// ciphertext — the IV is **not** prepended (delta A2).
    pub fn cbc_encrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CBC, true, key, Some(iv), pt, ct)
    }

    /// AES-CBC decrypt with the given 16-byte `iv`. `ct` is the bare
    /// ciphertext (no leading IV — delta A2).
    pub fn cbc_decrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CBC, false, key, Some(iv), ct, pt)
    }
}

impl Drop for AesCipher<'_> {
    fn drop(&mut self) {
        // Delta A3 (defensive): no key/IV residue past the op's scope.
        self.ctx.ctx = [0u8; 64];
    }
}
