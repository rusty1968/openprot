// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! HACE AES (128/192/256, ECB/CBC/CFB/OFB/CTR, raw-key path).
//!
//! Reference behavior follows pinned Zephyr `aspeed_hace` (`cfe94dc`).
//! Correctness is checked with NIST AESAVS/CAVP KATs.
//!
//! Deltas:
//! - A1: borrow-arbitrated exclusivity via `&mut HaceDevice`
//! - A2: ciphertext is plain `CT` (IV is separate)
//! - A3: key/IV context bytes are zeroized after each op and on drop
//! - A4: invalid block sizing returns `InvalidInput` before programming HW
//! - A5: OTP/secret-vault keys are out of scope

use super::constants::{
    AES_CMD_BASE, HACE_CMD_AES128, HACE_CMD_AES192, HACE_CMD_AES256, HACE_CMD_CBC, HACE_CMD_CFB,
    HACE_CMD_CTR, HACE_CMD_ECB, HACE_CMD_ENCRYPT, HACE_CMD_OFB, HACE_SG_LAST, POLL_YIELD_NS,
};
use super::context::CryptoContext;
use super::device::HaceDevice;
use super::error::HaceError;
use super::helpers::{dcache_invd_all, ptr_to_u32};
use super::registers::HaceRegisters;
use core::marker::PhantomData;
use openprot_hal_blocking::cipher::{
    BlockCipherMode, CipherInit, CipherMode, CipherOp, ErrorType, SymmetricCipher,
};

/// AES block size in bytes. ECB/CBC input must be block-aligned.
pub const AES_BLOCK: usize = 16;

/// Borrow-arbitrated AES operation over HACE.
///
/// Created via [`AesCipher::from_device`]. The retained `&mut` borrow prevents
/// overlapping AES/digest/HMAC operations.
pub struct AesCipher<'a> {
    regs: HaceRegisters,
    ctx: &'a mut CryptoContext,
    poll_budget: u32,
    /// Cooperative yield hook called once per completion poll.
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

    /// Build an AES adapter from a [`HaceDevice`](super::device::HaceDevice).
    ///
    /// # Safety
    /// No concurrent or reentrant HACE access for the returned lifetime.
    pub unsafe fn from_device<Y: FnMut(u32)>(device: &'a mut super::device::HaceDevice<Y>) -> Self {
        // Borrow split; retained `yield_fn` keeps the device exclusively borrowed.
        let regs = device.regs;
        let poll_budget = device.poll_budget;
        // SAFETY: single-instance device + exclusive live borrow gate access.
        let ctx: &'a mut CryptoContext = unsafe { &mut *device.crypto_ctx };
        let yield_fn: &'a mut dyn FnMut(u32) = &mut device.yield_fn;
        Self::new(regs, ctx, poll_budget, yield_fn)
    }

    /// Map AES key length to command bits.
    fn keylen_bits(key: &[u8]) -> Result<u32, HaceError> {
        match key.len() {
            16 => Ok(HACE_CMD_AES128),
            24 => Ok(HACE_CMD_AES192),
            32 => Ok(HACE_CMD_AES256),
            _ => Err(HaceError::InvalidInput),
        }
    }

    /// One-shot AES transform.
    ///
    /// `mode_bits` is ECB or CBC. `iv` is set only for CBC. Buffers are plain
    /// data (no in-band IV). Context key/IV bytes are always zeroized.
    fn crypt(
        &mut self,
        mode_bits: u32,
        encrypt: bool,
        key: &[u8],
        iv: Option<&[u8; AES_BLOCK]>,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(), HaceError> {
        // Enforce block-aligned sizing and DMA staging cap before programming.
        if input.is_empty() || input.len() % AES_BLOCK != 0 || output.len() < input.len() {
            return Err(HaceError::InvalidInput);
        }
        let kbits = Self::keylen_bits(key)?;
        let len = u32::try_from(input.len()).map_err(|_| HaceError::InvalidInput)?;

        // Engine context: IV at [0..16) for CBC, key at [16..16+keylen).
        self.ctx.ctx = [0u8; 64];
        if let Some(iv) = iv {
            // Length-proven array assignment: `iv` is `&[u8; AES_BLOCK]`, so cast
            // the destination prefix to `&mut [u8; AES_BLOCK]` and copy as fixed
            // arrays. `copy_from_slice` would keep a length-mismatch panic branch.
            if let Some(dst) = self.ctx.ctx.get_mut(..AES_BLOCK) {
                if let Ok(dst) = <&mut [u8; AES_BLOCK]>::try_from(dst) {
                    *dst = *iv;
                }
            }
        }
        if let Some(dst) = self.ctx.ctx.get_mut(AES_BLOCK..AES_BLOCK + key.len()) {
            dst.copy_from_slice(key);
        }

        // The AST10x0 crypto MBUS reads payloads from ordinary SRAM.  The
        // engine context and SG descriptors stay in `.ram_nc`, matching the
        // working aspeed-rust implementation, but putting source/destination
        // payloads in that window stalls the synchronous HACE10 start write.
        // Callers must therefore provide readable/writable SRAM buffers (the
        // KAT uses static RAM buffers); flash-backed input is unsupported.
        let in_ptr = ptr_to_u32(input.as_ptr())?;
        let out_ptr = ptr_to_u32(output.as_mut_ptr())?;
        self.ctx.src.addr = in_ptr;
        self.ctx.src.len = len | HACE_SG_LAST;
        self.ctx.dst.addr = out_ptr;
        self.ctx.dst.len = len | HACE_SG_LAST;

        let cmd = AES_CMD_BASE | kbits | mode_bits | if encrypt { HACE_CMD_ENCRYPT } else { 0 };
        self.ctx.cmd = cmd;

        // Program descriptor addresses, ctx base, and data length.
        let src_desc = ptr_to_u32(core::ptr::addr_of!(self.ctx.src))?;
        let dst_desc = ptr_to_u32(core::ptr::addr_of!(self.ctx.dst))?;
        let ctx_base = ptr_to_u32(self.ctx.ctx.as_ptr())?;
        // HACE0C takes the plain byte count.  Only the SG descriptor length
        // words carry HACE_SG_LAST; passing that high bit here makes the
        // engine attempt an invalid-sized transfer and never complete.
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

        // The engine's DMA writes bypass the AST10x0 SRAM cache; the caller's
        // output buffer may be in cacheable SRAM with stale (pre-op) lines
        // resident. Invalidate before anyone reads the result (authority:
        // `hace_aspeed.c` calls `cache_data_invd_all()` after every crypto op).
        dcache_invd_all();

        // Always clear key/IV material from the DMA context buffer.
        self.ctx.ctx = [0u8; 64];

        if done {
            Ok(())
        } else {
            pw_log::error!(
                "hace: AES timeout: HACE1C={:#010x}, cmd={:#010x}, len={}",
                self.regs.hace1c_bits() as u32,
                cmd as u32,
                len as u32,
            );
            Err(HaceError::Timeout)
        }
    }

    /// AES-ECB encrypt.
    ///
    /// `key` is 16 or 32 bytes. `pt` must be non-empty and block-aligned.
    /// `ct` must be at least `pt.len()`. Output is ciphertext only.
    pub fn ecb_encrypt(&mut self, key: &[u8], pt: &[u8], ct: &mut [u8]) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_ECB, true, key, None, pt, ct)
    }

    /// AES-ECB decrypt (inverse of [`ecb_encrypt`](Self::ecb_encrypt)).
    pub fn ecb_decrypt(&mut self, key: &[u8], ct: &[u8], pt: &mut [u8]) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_ECB, false, key, None, ct, pt)
    }

    /// AES-CBC encrypt with a 16-byte IV. Output excludes IV prefix.
    pub fn cbc_encrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CBC, true, key, Some(iv), pt, ct)
    }

    /// AES-CBC decrypt with a 16-byte IV; input excludes IV prefix.
    pub fn cbc_decrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CBC, false, key, Some(iv), ct, pt)
    }

    /// AES-CFB encrypt with a 16-byte IV.
    pub fn cfb_encrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CFB, true, key, Some(iv), pt, ct)
    }

    /// AES-CFB decrypt with a 16-byte IV.
    pub fn cfb_decrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CFB, false, key, Some(iv), ct, pt)
    }

    /// AES-OFB encrypt with a 16-byte IV.
    pub fn ofb_encrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_OFB, true, key, Some(iv), pt, ct)
    }

    /// AES-OFB decrypt with a 16-byte IV.
    pub fn ofb_decrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_OFB, false, key, Some(iv), ct, pt)
    }

    /// AES-CTR encrypt with a 16-byte initial counter block.
    pub fn ctr_encrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        pt: &[u8],
        ct: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CTR, true, key, Some(iv), pt, ct)
    }

    /// AES-CTR decrypt with a 16-byte initial counter block.
    pub fn ctr_decrypt(
        &mut self,
        key: &[u8],
        iv: &[u8; AES_BLOCK],
        ct: &[u8],
        pt: &mut [u8],
    ) -> Result<(), HaceError> {
        self.crypt(HACE_CMD_CTR, false, key, Some(iv), ct, pt)
    }
}

impl Drop for AesCipher<'_> {
    fn drop(&mut self) {
        // Defensive scrub on drop.
        self.ctx.ctx = [0u8; 64];
    }
}

// ===== Optional openprot cipher-trait skin (ADR-A1) =====================
//
// Thin fixed-`N` wrapper over `AesCipher`. Kept separate because
// `SymmetricCipher` uses fixed associated buffer types and cannot express
// large streaming DMA paths.

/// AES-ECB mode marker (port-defined; the hal declares no concrete modes).
#[derive(Debug, Clone, Copy)]
pub struct Ecb;
/// AES-CBC mode marker.
#[derive(Debug, Clone, Copy)]
pub struct Cbc;
/// AES-CFB mode marker.
#[derive(Debug, Clone, Copy)]
pub struct Cfb;
/// AES-OFB mode marker.
#[derive(Debug, Clone, Copy)]
pub struct Ofb;
/// AES-CTR mode marker.
#[derive(Debug, Clone, Copy)]
pub struct Ctr;

impl CipherMode for Ecb {}
impl BlockCipherMode for Ecb {}
impl CipherMode for Cbc {}
impl BlockCipherMode for Cbc {}
impl CipherMode for Cfb {}
impl BlockCipherMode for Cfb {}
impl CipherMode for Ofb {}
impl BlockCipherMode for Ofb {}
impl CipherMode for Ctr {}
impl BlockCipherMode for Ctr {}

/// Owned AES key for the trait skin (raw-key path only).
///
/// Size selects variant: 16 => AES-128, 24 => AES-192, 32 => AES-256.
///
/// The key bytes are zeroized when this value is dropped.
#[derive(Clone)]
pub enum AesKey {
    Aes128([u8; 16]),
    Aes192([u8; 24]),
    Aes256([u8; 32]),
}

impl AesKey {
    #[inline]
    fn as_slice(&self) -> &[u8] {
        match self {
            AesKey::Aes128(k) => k,
            AesKey::Aes192(k) => k,
            AesKey::Aes256(k) => k,
        }
    }

    fn zeroize(&mut self) {
        match self {
            AesKey::Aes128(k) => k.fill(0),
            AesKey::Aes192(k) => k.fill(0),
            AesKey::Aes256(k) => k.fill(0),
        }
    }
}

impl Drop for AesKey {
    fn drop(&mut self) {
        self.zeroize();
    }
}

/// Fixed-`N` openprot cipher-trait skin bound to one [`HaceDevice`].
///
/// Each context wraps borrow-arbitrated [`AesCipher`].
pub struct AesSkin<'d, Y: FnMut(u32), const N: usize> {
    dev: &'d mut HaceDevice<Y>,
}

impl<'d, Y: FnMut(u32), const N: usize> AesSkin<'d, Y, N> {
    /// Bind the cipher skin to the device.
    ///
    /// # Safety
    /// Same contract as [`AesCipher::from_device`]: no concurrent/reentrant
    /// HACE access for this skin or derived contexts.
    pub unsafe fn new(dev: &'d mut HaceDevice<Y>) -> Self {
        Self { dev }
    }
}

/// In-flight trait-skin op: core plus session key/IV.
pub struct AesOp<'a, const N: usize, M> {
    core: AesCipher<'a>,
    key: AesKey,
    iv: [u8; AES_BLOCK],
    /// Set after one `encrypt`/`decrypt` call. Only consulted by modes whose
    /// HACE-side feedback/counter state can't be safely re-derived from a
    /// second call's inputs (`Cfb`/`Ofb`/`Ctr`); `Ecb`/`Cbc` ignore it and
    /// remain multi-call.
    used: bool,
    _m: PhantomData<M>,
}

impl<'d, Y: FnMut(u32), const N: usize> ErrorType for AesSkin<'d, Y, N> {
    type Error = HaceError;
}

impl<'d, Y: FnMut(u32), const N: usize> SymmetricCipher for AesSkin<'d, Y, N> {
    type Key = AesKey;
    type Nonce = [u8; AES_BLOCK];
    type PlainText = [u8; N];
    type CipherText = [u8; N];
}

impl<'a, const N: usize, M> ErrorType for AesOp<'a, N, M> {
    type Error = HaceError;
}

impl<'a, const N: usize, M> SymmetricCipher for AesOp<'a, N, M> {
    type Key = AesKey;
    type Nonce = [u8; AES_BLOCK];
    type PlainText = [u8; N];
    type CipherText = [u8; N];
}

macro_rules! cipher_init {
    ($mode:ty) => {
        impl<'d, Y: FnMut(u32), const N: usize> CipherInit<$mode> for AesSkin<'d, Y, N> {
            type CipherContext<'a>
                = AesOp<'a, N, $mode>
            where
                Self: 'a;

            fn init<'a>(
                &'a mut self,
                key: &Self::Key,
                nonce: &Self::Nonce,
            ) -> Result<Self::CipherContext<'a>, Self::Error> {
                // SAFETY: `AesSkin::new` guarantees non-reentrancy; reborrow is exclusive.
                let core = unsafe { AesCipher::from_device(&mut *self.dev) };
                Ok(AesOp {
                    core,
                    key: key.clone(),
                    iv: *nonce,
                    used: false,
                    _m: PhantomData,
                })
            }
        }
    };
}
cipher_init!(Ecb);
cipher_init!(Cbc);
cipher_init!(Cfb);
cipher_init!(Ofb);
cipher_init!(Ctr);

impl<'a, const N: usize> CipherOp<Ecb> for AesOp<'a, N, Ecb> {
    fn encrypt(&mut self, plaintext: [u8; N]) -> Result<[u8; N], HaceError> {
        const { assert!(N % AES_BLOCK == 0, "AesSkin<N>: N must be a multiple of 16") };
        let mut ct = [0u8; N];
        self.core
            .ecb_encrypt(self.key.as_slice(), &plaintext, &mut ct)?;
        Ok(ct)
    }

    fn decrypt(&mut self, ciphertext: [u8; N]) -> Result<[u8; N], HaceError> {
        const { assert!(N % AES_BLOCK == 0, "AesSkin<N>: N must be a multiple of 16") };
        let mut pt = [0u8; N];
        self.core
            .ecb_decrypt(self.key.as_slice(), &ciphertext, &mut pt)?;
        Ok(pt)
    }
}

impl<'a, const N: usize> CipherOp<Cbc> for AesOp<'a, N, Cbc> {
    fn encrypt(&mut self, plaintext: [u8; N]) -> Result<[u8; N], HaceError> {
        const { assert!(N % AES_BLOCK == 0, "AesSkin<N>: N must be a multiple of 16") };
        let mut ct = [0u8; N];
        self.core
            .cbc_encrypt(self.key.as_slice(), &self.iv, &plaintext, &mut ct)?;
        // Advance IV to last ciphertext block so sequential encrypt() calls
        // form a correct CBC chain instead of reusing the original IV.
        if let Some(last) = ct.get(N - AES_BLOCK..) {
            if let Ok(last) = <[u8; AES_BLOCK]>::try_from(last) {
                self.iv = last;
            }
        }
        Ok(ct)
    }

    fn decrypt(&mut self, ciphertext: [u8; N]) -> Result<[u8; N], HaceError> {
        const { assert!(N % AES_BLOCK == 0, "AesSkin<N>: N must be a multiple of 16") };
        let mut pt = [0u8; N];
        self.core
            .cbc_decrypt(self.key.as_slice(), &self.iv, &ciphertext, &mut pt)?;
        // Advance IV to last ciphertext block (CBC decrypt chaining).
        if let Some(last) = ciphertext.get(N - AES_BLOCK..) {
            if let Ok(last) = <[u8; AES_BLOCK]>::try_from(last) {
                self.iv = last;
            }
        }
        Ok(pt)
    }
}

// CFB/OFB/CTR: single-shot only. Chaining a second call correctly needs the
// keystream (OFB) or an externally-incremented counter (CTR), neither of
// which the engine exposes after one op; CFB's feedback happens to equal the
// last ciphertext block (same as CBC) but is kept single-shot here too so all
// three new modes behave identically. A second `encrypt`/`decrypt` call on
// the same context returns `HaceError::Busy`, matching aspeed-rust's own
// one-shot-per-context discipline for these modes.
macro_rules! cipher_op_single_shot {
    ($mode:ty, $enc:ident, $dec:ident) => {
        impl<'a, const N: usize> CipherOp<$mode> for AesOp<'a, N, $mode> {
            fn encrypt(&mut self, plaintext: [u8; N]) -> Result<[u8; N], HaceError> {
                const { assert!(N % AES_BLOCK == 0, "AesSkin<N>: N must be a multiple of 16") };
                if core::mem::replace(&mut self.used, true) {
                    return Err(HaceError::Busy);
                }
                let mut ct = [0u8; N];
                self.core
                    .$enc(self.key.as_slice(), &self.iv, &plaintext, &mut ct)?;
                Ok(ct)
            }

            fn decrypt(&mut self, ciphertext: [u8; N]) -> Result<[u8; N], HaceError> {
                const { assert!(N % AES_BLOCK == 0, "AesSkin<N>: N must be a multiple of 16") };
                if core::mem::replace(&mut self.used, true) {
                    return Err(HaceError::Busy);
                }
                let mut pt = [0u8; N];
                self.core
                    .$dec(self.key.as_slice(), &self.iv, &ciphertext, &mut pt)?;
                Ok(pt)
            }
        }
    };
}
cipher_op_single_shot!(Cfb, cfb_encrypt, cfb_decrypt);
cipher_op_single_shot!(Ofb, ofb_encrypt, ofb_decrypt);
cipher_op_single_shot!(Ctr, ctr_encrypt, ctr_decrypt);
