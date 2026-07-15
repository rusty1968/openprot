// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! HACE DES/Triple-DES (ECB/CBC/CFB/OFB/CTR, raw-key path).
//!
//! Shares the HACE crypto sub-engine and `.ram_nc` DMA-staged `CryptoContext`
//! with `aes.rs`; see that module's header for the DMA-safety rationale (D3):
//! caller input/output is always staged through the non-cacheable
//! `data_in`/`data_out` buffers rather than DMA'd directly to/from a
//! caller-supplied slice.
//!
//! Reference behavior follows pinned Zephyr `aspeed_hace` (`cfe94dc`).
//!
//! Deltas (mirrors `aes.rs`):
//! - key/IV context bytes are zeroized after each op and on drop
//! - invalid block sizing returns `InvalidInput` before programming HW
//! - no `openprot_hal_blocking::cipher` trait-skin yet (low-level API only);
//!   add a `DesSkin`/`DesOp` pair analogous to `aes::AesSkin`/`AesOp` if a
//!   caller needs the fixed-`N` buffer convenience layer.
//!
//! Hardware layout note: unlike AES (IV at `ctx` offset 0), DES/TDES place
//! the 8-byte IV at `ctx` offset 8; the raw key always starts at offset 16
//! for both families (`hace_aspeed.c` crypto context layout).

use super::constants::{
    DES_CMD_BASE, HACE_CMD_CBC, HACE_CMD_CFB, HACE_CMD_CTR, HACE_CMD_ECB, HACE_CMD_ENCRYPT,
    HACE_CMD_OFB, HACE_CMD_SINGLE_DES, HACE_CMD_TRIPLE_DES, HACE_SG_LAST, POLL_YIELD_NS,
};
use super::context::{CryptoContext, AES_DATA_CAP};
use super::device::HaceDevice;
use super::error::HaceError;
use super::helpers::{dcache_invd_all, ptr_to_u32};
use super::registers::HaceRegisters;

/// DES/TDES block size in bytes. ECB/CBC input must be block-aligned.
pub const DES_BLOCK: usize = 8;

/// Offset of the raw key inside `CryptoContext::ctx` (shared with AES).
const KEY_OFFSET: usize = 16;
/// Offset of the IV inside `CryptoContext::ctx` for DES/TDES (AES uses 0).
const IV_OFFSET: usize = 8;

/// Owned DES/TDES key (raw-key path only).
///
/// Size selects variant: 8 => single DES, 24 => Triple-DES (3-key EDE).
///
/// The key bytes are zeroized when this value is dropped.
#[derive(Clone)]
pub enum DesKey {
    Des([u8; 8]),
    Tdes([u8; 24]),
}

impl DesKey {
    #[inline]
    fn as_slice(&self) -> &[u8] {
        match self {
            DesKey::Des(k) => k,
            DesKey::Tdes(k) => k,
        }
    }

    fn zeroize(&mut self) {
        match self {
            DesKey::Des(k) => k.fill(0),
            DesKey::Tdes(k) => k.fill(0),
        }
    }

    #[inline]
    fn cmd_bits(&self) -> u32 {
        match self {
            DesKey::Des(_) => DES_CMD_BASE | HACE_CMD_SINGLE_DES,
            DesKey::Tdes(_) => DES_CMD_BASE | HACE_CMD_TRIPLE_DES,
        }
    }
}

impl Drop for DesKey {
    fn drop(&mut self) {
        self.zeroize();
    }
}

/// Borrow-arbitrated DES/TDES operation over HACE.
///
/// Created via [`DesCipher::from_device`]. Shares the same crypto context and
/// exclusivity discipline as [`super::aes::AesCipher`] — retaining a `&mut`
/// borrow prevents overlapping AES/DES/digest/HMAC operations.
pub struct DesCipher<'a> {
    regs: HaceRegisters,
    ctx: &'a mut CryptoContext,
    poll_budget: u32,
    /// Cooperative yield hook called once per completion poll.
    yield_fn: &'a mut dyn FnMut(u32),
}

impl<'a> DesCipher<'a> {
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

    /// Build a DES/TDES adapter from a [`HaceDevice`](super::device::HaceDevice).
    ///
    /// # Safety
    /// No concurrent or reentrant HACE access for the returned lifetime.
    pub unsafe fn from_device<Y: FnMut(u32)>(device: &'a mut HaceDevice<Y>) -> Self {
        // Borrow split; retained `yield_fn` keeps the device exclusively borrowed.
        let regs = device.regs;
        let poll_budget = device.poll_budget;
        // SAFETY: single-instance device + exclusive live borrow gate access.
        let ctx: &'a mut CryptoContext = unsafe { &mut *device.crypto_ctx };
        let yield_fn: &'a mut dyn FnMut(u32) = &mut device.yield_fn;
        Self::new(regs, ctx, poll_budget, yield_fn)
    }

    /// One-shot DES/TDES transform.
    ///
    /// `mode_bits` is ECB/CBC/CFB/OFB/CTR. `iv` is set for every mode except
    /// ECB. Buffers are plain data (no in-band IV). Context key/IV bytes are
    /// always zeroized.
    fn crypt(
        &mut self,
        mode_bits: u32,
        encrypt: bool,
        key: &DesKey,
        iv: Option<&[u8; DES_BLOCK]>,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(), HaceError> {
        // Enforce block-aligned sizing and DMA staging cap before programming.
        if input.is_empty() || input.len() % DES_BLOCK != 0 || output.len() < input.len() {
            return Err(HaceError::InvalidInput);
        }
        if input.len() > AES_DATA_CAP {
            return Err(HaceError::InvalidInput);
        }
        let key_bytes = key.as_slice();
        let len = u32::try_from(input.len()).map_err(|_| HaceError::InvalidInput)?;

        // Engine context: IV at [8..16) for DES/TDES, key at [16..16+keylen).
        self.ctx.ctx = [0u8; 64];
        if let Some(iv) = iv {
            if let Some(dst) = self.ctx.ctx.get_mut(IV_OFFSET..IV_OFFSET + DES_BLOCK) {
                if let Ok(dst) = <&mut [u8; DES_BLOCK]>::try_from(dst) {
                    *dst = *iv;
                }
            }
        }
        if let Some(dst) = self
            .ctx
            .ctx
            .get_mut(KEY_OFFSET..KEY_OFFSET + key_bytes.len())
        {
            dst.copy_from_slice(key_bytes);
        }

        // DMA safety (D3): copy caller input into the shared `.ram_nc`
        // staging buffer, same rationale as `aes.rs::crypt`.
        if let Some(dst) = self.ctx.data_in.get_mut(..input.len()) {
            dst.copy_from_slice(input);
        }

        let in_ptr = ptr_to_u32(self.ctx.data_in.as_ptr())?;
        let out_ptr = ptr_to_u32(self.ctx.data_out.as_ptr())?;
        self.ctx.src.addr = in_ptr;
        self.ctx.src.len = len | HACE_SG_LAST;
        self.ctx.dst.addr = out_ptr;
        self.ctx.dst.len = len | HACE_SG_LAST;

        let cmd = key.cmd_bits() | mode_bits | if encrypt { HACE_CMD_ENCRYPT } else { 0 };
        self.ctx.cmd = cmd;

        let src_desc = ptr_to_u32(core::ptr::addr_of!(self.ctx.src))?;
        let dst_desc = ptr_to_u32(core::ptr::addr_of!(self.ctx.dst))?;
        let ctx_base = ptr_to_u32(self.ctx.ctx.as_ptr())?;
        // HACE0C takes the plain byte count.  The SG descriptors, not this
        // register, carry HACE_SG_LAST.
        let data_len = len;

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

        // HACE DMA writes bypass the cache. Invalidate before copying the
        // result from the non-cacheable staging buffer and before callers
        // inspect their output buffer, matching the AES path.
        dcache_invd_all();

        // Always clear key/IV material from the DMA context buffer.
        self.ctx.ctx = [0u8; 64];

        if done {
            let n = input.len();
            output
                .get_mut(..n)
                .ok_or(HaceError::InvalidInput)?
                .copy_from_slice(self.ctx.data_out.get(..n).ok_or(HaceError::InvalidInput)?);
            // Scrub staging buffers so plaintext/ciphertext doesn't linger.
            if let Some(s) = self.ctx.data_in.get_mut(..n) {
                s.fill(0);
            }
            if let Some(s) = self.ctx.data_out.get_mut(..n) {
                s.fill(0);
            }
            Ok(())
        } else {
            pw_log::error!(
                "hace: DES timeout: HACE1C={:#010x}, cmd={:#010x}, len={}",
                self.regs.hace1c_bits() as u32,
                cmd as u32,
                len as u32,
            );
            if let Some(s) = self.ctx.data_in.get_mut(..input.len()) {
                s.fill(0);
            }
            Err(HaceError::Timeout)
        }
    }

    /// DES/TDES-ECB encrypt.
    pub fn ecb_encrypt(&mut self, key: &DesKey, pt: &[u8], ct: &mut [u8]) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_ECB, true, key, None, pt, ct)
    }

    /// DES/TDES-ECB decrypt (inverse of [`ecb_encrypt`](Self::ecb_encrypt)).
    pub fn ecb_decrypt(&mut self, key: &DesKey, ct: &[u8], pt: &mut [u8]) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_ECB, false, key, None, ct, pt)
    }

    /// DES/TDES-CBC encrypt with an 8-byte IV. Output excludes IV prefix.
    pub fn cbc_encrypt(
        &mut self,
        key: &DesKey,
        iv: &[u8; DES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CBC, true, key, Some(iv), pt, ct)
    }

    /// DES/TDES-CBC decrypt with an 8-byte IV; input excludes IV prefix.
    pub fn cbc_decrypt(
        &mut self,
        key: &DesKey,
        iv: &[u8; DES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CBC, false, key, Some(iv), ct, pt)
    }

    /// DES/TDES-CFB encrypt with an 8-byte IV.
    pub fn cfb_encrypt(
        &mut self,
        key: &DesKey,
        iv: &[u8; DES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CFB, true, key, Some(iv), pt, ct)
    }

    /// DES/TDES-CFB decrypt with an 8-byte IV.
    pub fn cfb_decrypt(
        &mut self,
        key: &DesKey,
        iv: &[u8; DES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CFB, false, key, Some(iv), ct, pt)
    }

    /// DES/TDES-OFB encrypt with an 8-byte IV.
    pub fn ofb_encrypt(
        &mut self,
        key: &DesKey,
        iv: &[u8; DES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_OFB, true, key, Some(iv), pt, ct)
    }

    /// DES/TDES-OFB decrypt with an 8-byte IV.
    pub fn ofb_decrypt(
        &mut self,
        key: &DesKey,
        iv: &[u8; DES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_OFB, false, key, Some(iv), ct, pt)
    }

    /// DES/TDES-CTR encrypt with an 8-byte initial counter block.
    pub fn ctr_encrypt(
        &mut self,
        key: &DesKey,
        iv: &[u8; DES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CTR, true, key, Some(iv), pt, ct)
    }

    /// DES/TDES-CTR decrypt with an 8-byte initial counter block.
    pub fn ctr_decrypt(
        &mut self,
        key: &DesKey,
        iv: &[u8; DES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CTR, false, key, Some(iv), ct, pt)
    }
}

impl Drop for DesCipher<'_> {
    fn drop(&mut self) {
        // Defensive scrub on drop.
        self.ctx.ctx = [0u8; 64];
    }
}
