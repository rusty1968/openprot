// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

//! AST10x0 HACE DES/TDES KAT suite.
//!
//! Verifies DES/TDES ECB/CBC encrypt+decrypt against known-answer vectors
//! (see `vectors.rs` for provenance), plus InvalidInput enforcement for
//! non-block-size input.

use ast10x0_board::{Ast10x0Board, Ast10x0BoardDescriptor};
use ast10x0_peripherals::hace::{DesCipher, DesKey, HaceDevice, HaceError};
use codegen as _;
use console_backend::console_backend_write_all;
use entry as _;
use target_common::{declare_target, TargetInterface};

pub struct Target {}

const ERR_DES_FAILED: &str = "hace des op failed";
const ERR_VERIFY_FAILED: &str = "hace des mismatch";
const ERR_A4: &str = "hace des delta-A4 not enforced";

// DMA scratch buffers. Static RAM is required for SG-DMA access.
static mut DES_IN: [u8; 4096] = [0u8; 4096];
static mut DES_OUT: [u8; 4096] = [0u8; 4096];

#[inline]
fn pat(i: usize) -> u8 {
    (i % 251) as u8
}

fn check(name: &str, actual: &[u8], expected: &[u8]) -> Result<(), &'static str> {
    if actual != expected {
        pw_log::error!("{}: mismatch", name as &str);
        return Err(ERR_VERIFY_FAILED);
    }
    pw_log::info!("{}: PASS", name as &str);
    Ok(())
}

/// Run one DES/TDES KAT case and compare output bytes.
macro_rules! kat {
    ($name:expr, $op:ident, $key:expr, $iv:expr, $inp:expr, $expected:expr) => {{
        pw_log::info!("case: {}", $name as &str);
        let n = $inp.len();
        // SAFETY: serial single-threaded use of the DMA scratch buffers.
        let inb = unsafe { &mut *core::ptr::addr_of_mut!(DES_IN) };
        let outb = unsafe { &mut *core::ptr::addr_of_mut!(DES_OUT) };
        inb[..n].copy_from_slice($inp);
        outb[..n].fill(0);
        // SAFETY: test runs once at boot with exclusive HACE access.
        let mut device = unsafe { HaceDevice::new_global(|_| core::hint::spin_loop()) };
        // SAFETY: single-threaded test; no concurrent HACE access.
        let mut des = unsafe { DesCipher::from_device(&mut device) };
        kat!(@call des, $op, $key, $iv, inb, outb, n);
        check($name, &outb[..n], $expected)?;
    }};
    (@call $des:ident, ecb_encrypt, $key:expr, $iv:expr, $inb:ident, $outb:ident, $n:expr) => {
        $des.ecb_encrypt($key, &$inb[..$n], &mut $outb[..$n]).map_err(|_| ERR_DES_FAILED)?
    };
    (@call $des:ident, ecb_decrypt, $key:expr, $iv:expr, $inb:ident, $outb:ident, $n:expr) => {
        $des.ecb_decrypt($key, &$inb[..$n], &mut $outb[..$n]).map_err(|_| ERR_DES_FAILED)?
    };
    (@call $des:ident, cbc_encrypt, $key:expr, $iv:expr, $inb:ident, $outb:ident, $n:expr) => {
        $des.cbc_encrypt($key, $iv, &$inb[..$n], &mut $outb[..$n]).map_err(|_| ERR_DES_FAILED)?
    };
    (@call $des:ident, cbc_decrypt, $key:expr, $iv:expr, $inb:ident, $outb:ident, $n:expr) => {
        $des.cbc_decrypt($key, $iv, &$inb[..$n], &mut $outb[..$n]).map_err(|_| ERR_DES_FAILED)?
    };
}

include!("vectors.rs");

fn run_hace_des_kats() -> Result<(), &'static str> {
    pw_log::info!("=== AST10x0 HACE DES/TDES KAT suite ===");

    let board = Ast10x0Board::new(Ast10x0BoardDescriptor {
        pinctrl_groups: &[],
        i2c_buses: &[],
    });
    // SAFETY: test runs once at boot with exclusive access to the board.
    unsafe {
        let _ = board.init();
    };

    let des_key = DesKey::Des(DES_ECB_KEY);
    kat!(
        "des-ecb encrypt",
        ecb_encrypt,
        &des_key,
        &DES_ECB_KEY,
        &DES_ECB_PT,
        &DES_ECB_CT
    );
    kat!(
        "des-ecb decrypt",
        ecb_decrypt,
        &des_key,
        &DES_ECB_KEY,
        &DES_ECB_CT,
        &DES_ECB_PT
    );

    let tdes_ecb_key = DesKey::Tdes(TDES_ECB_KEY);
    kat!(
        "tdes-ecb encrypt",
        ecb_encrypt,
        &tdes_ecb_key,
        &DES_ECB_KEY,
        &TDES_ECB_PT,
        &TDES_ECB_CT
    );
    kat!(
        "tdes-ecb decrypt",
        ecb_decrypt,
        &tdes_ecb_key,
        &DES_ECB_KEY,
        &TDES_ECB_CT,
        &TDES_ECB_PT
    );

    let des_cbc_key = DesKey::Des(DES_CBC_KEY);
    kat!(
        "des-cbc encrypt",
        cbc_encrypt,
        &des_cbc_key,
        &DES_CBC_IV,
        &DES_CBC_PT,
        &DES_CBC_CT
    );
    kat!(
        "des-cbc decrypt",
        cbc_decrypt,
        &des_cbc_key,
        &DES_CBC_IV,
        &DES_CBC_CT,
        &DES_CBC_PT
    );

    let tdes_cbc_key = DesKey::Tdes(TDES_CBC_KEY);
    kat!(
        "tdes-cbc encrypt",
        cbc_encrypt,
        &tdes_cbc_key,
        &TDES_CBC_IV,
        &TDES_CBC_PT,
        &TDES_CBC_CT
    );
    kat!(
        "tdes-cbc decrypt",
        cbc_decrypt,
        &tdes_cbc_key,
        &TDES_CBC_IV,
        &TDES_CBC_CT,
        &TDES_CBC_PT
    );

    // Non-block-size input must return InvalidInput.
    {
        pw_log::info!("case: {}", "delta-A4 reject 9B" as &str);
        let inb = unsafe { &mut *core::ptr::addr_of_mut!(DES_IN) };
        let outb = unsafe { &mut *core::ptr::addr_of_mut!(DES_OUT) };
        for k in 0..9 {
            inb[k] = pat(k);
        }
        let mut device = unsafe { HaceDevice::new_global(|_| core::hint::spin_loop()) };
        let mut des = unsafe { DesCipher::from_device(&mut device) };
        let key = DesKey::Des(DES_ECB_KEY);
        match des.ecb_encrypt(&key, &inb[..9], &mut outb[..9]) {
            Err(HaceError::InvalidInput) => pw_log::info!("delta-A4 reject 9B: PASS"),
            _ => return Err(ERR_A4),
        }
    }

    pw_log::info!("=== AST10x0 HACE DES/TDES KAT suite complete ===");
    Ok(())
}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 HACE DES/TDES KAT";

    fn main() -> ! {
        let sentinel: &[u8] = match run_hace_des_kats() {
            Ok(()) => b"TEST_RESULT:PASS\n",
            Err(error) => {
                pw_log::error!("HACE DES/TDES KAT suite failed: {}", error as &str);
                b"TEST_RESULT:FAIL\n"
            }
        };

        let _ = console_backend_write_all(sentinel);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
