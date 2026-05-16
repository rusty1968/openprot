// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

use ast10x0_board::{Ast10x0Board, Ast10x0BoardDescriptor};
use ast10x0_peripherals::hace::{HaceDevice, HaceDigest};
use codegen as _;
use console_backend::console_backend_write_all;
use entry as _;
use target_common::{TargetInterface, declare_target};

pub struct Target {}

const ERR_HASH_FAILED: &str = "hace sha256 hash failed";
const ERR_VERIFY_FAILED: &str = "hace sha256 digest mismatch";

fn run_sha256_case(
    name: &str,
    input: &[u8],
    expected: &[u8; 32],
) -> Result<(), &'static str> {
    use openprot_hal_blocking::digest::Sha2_256;
    use openprot_hal_blocking::digest::scoped::{DigestInit, DigestOp};

    pw_log::info!("Running SHA-256 case: {}", name as &str);

    // SAFETY: Test target runs once at boot with exclusive access to the HACE.
    let mut device = unsafe { HaceDevice::new_global(|_| core::hint::spin_loop()) };

    // SAFETY: Single-threaded test; no concurrent HACE access.
    let mut digest_dev = unsafe { HaceDigest::<Sha2_256>::from_device(&mut device) };

    let mut op = digest_dev.init(Sha2_256).map_err(|_| ERR_HASH_FAILED)?;
    op.update(input).map_err(|_| ERR_HASH_FAILED)?;
    let digest = op.finalize().map_err(|_| ERR_HASH_FAILED)?;

    let actual: &[u8] = digest.as_bytes();
    if actual != expected.as_slice() {
        pw_log::error!("{}: digest mismatch", name as &str);
        return Err(ERR_VERIFY_FAILED);
    }

    pw_log::info!("{}: PASS", name as &str);
    Ok(())
}

fn run_hace_sha256_smoke_test() -> Result<(), &'static str> {
    pw_log::info!("=== AST10x0 HACE SHA-256 smoke test ===");

    let board = Ast10x0Board::new(Ast10x0BoardDescriptor {
        pinctrl_groups: &[],
    });

    // SAFETY: Test target runs once at boot with exclusive access to the board.
    unsafe { board.init() };

    // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    run_sha256_case(
        "empty",
        b"",
        &[
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14,
            0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
            0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c,
            0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
        ],
    )?;

    // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2ec73b00361bbef0469849678820950e0c50 (first 32 bytes only shown)
    // actual: ba7816bf8f01cfea414140de5dae2ec73b00361bbef0469849678820950e0c50 -> wait that's 64 hex = 32 bytes
    run_sha256_case(
        "abc",
        b"abc",
        &[
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
            0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x2e, 0xc7,
            0x3b, 0x00, 0x36, 0x1b, 0xbe, 0xf0, 0x46, 0x98,
            0x49, 0x67, 0x88, 0x20, 0x95, 0x0e, 0x0c, 0x50,
        ],
    )?;

    pw_log::info!("=== AST10x0 HACE SHA-256 smoke test complete ===");
    Ok(())
}

impl TargetInterface for Target {
    const NAME: &'static str = "AST10x0 HACE SHA-256";

    fn main() -> ! {
        let sentinel: &[u8] = match run_hace_sha256_smoke_test() {
            Ok(()) => b"TEST_RESULT:PASS\n",
            Err(error) => {
                pw_log::error!("HACE SHA-256 smoke test failed: {}", error as &str);
                b"TEST_RESULT:FAIL\n"
            }
        };

        let _ = console_backend_write_all(sentinel);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
