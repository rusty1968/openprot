// Licensed under the Apache-2.0 license

//! Crypto Client Test Application
//!
//! Tests the crypto server by making IPC requests for various crypto operations.
//! Uses the ergonomic `CryptoClient` API.

#![no_main]
#![no_std]

use app_crypto_client::handle;
use crypto_client::CryptoClient;
use pw_status::{Result, Error};
use userspace::entry;
use userspace::syscall;

fn test_sha256(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing SHA-256...");

    let hash = crypto.sha256(b"hello world")
        .map_err(|_| Error::Internal)?;

    const EXPECTED: [u8; 32] = [
        0xb9, 0x4d, 0x27, 0xb9, 0x93, 0x4d, 0x3e, 0x08,
        0xa5, 0x2e, 0x52, 0xd7, 0xda, 0x7d, 0xab, 0xfa,
        0xc4, 0x84, 0xef, 0xe3, 0x7a, 0x53, 0x80, 0xee,
        0x90, 0x88, 0xf7, 0xac, 0xe2, 0xef, 0xcd, 0xe9,
    ];

    if hash != EXPECTED {
        pw_log::error!("SHA-256 mismatch!");
        return Err(Error::Unknown);
    }

    pw_log::info!("SHA-256: PASS");
    Ok(())
}

fn test_sha384(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing SHA-384...");

    let hash = crypto.sha384(b"hello world")
        .map_err(|_| Error::Internal)?;

    const EXPECTED: [u8; 48] = [
        0xfd, 0xbd, 0x8e, 0x75, 0xa6, 0x7f, 0x29, 0xf7,
        0x01, 0xa4, 0xe0, 0x40, 0x38, 0x5e, 0x2e, 0x23,
        0x98, 0x63, 0x03, 0xea, 0x10, 0x23, 0x92, 0x11,
        0xaf, 0x90, 0x7f, 0xcb, 0xb8, 0x35, 0x78, 0xb3,
        0xe4, 0x17, 0xcb, 0x71, 0xce, 0x64, 0x6e, 0xfd,
        0x08, 0x19, 0xdd, 0x8c, 0x08, 0x8d, 0xe1, 0xbd,
    ];

    if hash != EXPECTED {
        pw_log::error!("SHA-384 mismatch!");
        return Err(Error::Unknown);
    }

    pw_log::info!("SHA-384: PASS");
    Ok(())
}

fn test_sha512(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing SHA-512...");

    let hash = crypto.sha512(b"hello world")
        .map_err(|_| Error::Internal)?;

    const EXPECTED: [u8; 64] = [
        0x30, 0x9e, 0xcc, 0x48, 0x9c, 0x12, 0xd6, 0xeb,
        0x4c, 0xc4, 0x0f, 0x50, 0xc9, 0x02, 0xf2, 0xb4,
        0xd0, 0xed, 0x77, 0xee, 0x51, 0x1a, 0x7c, 0x7a,
        0x9b, 0xcd, 0x3c, 0xa8, 0x6d, 0x4c, 0xd8, 0x6f,
        0x98, 0x9d, 0xd3, 0x5b, 0xc5, 0xff, 0x49, 0x96,
        0x70, 0xda, 0x34, 0x25, 0x5b, 0x45, 0xb0, 0xcf,
        0xd8, 0x30, 0xe8, 0x1f, 0x60, 0x5d, 0xcf, 0x7d,
        0xc5, 0x54, 0x2e, 0x93, 0xae, 0x9c, 0xd7, 0x6f,
    ];

    if hash != EXPECTED {
        pw_log::error!("SHA-512 mismatch!");
        return Err(Error::Unknown);
    }

    pw_log::info!("SHA-512: PASS");
    Ok(())
}

fn test_sha256_streaming(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing SHA-256 streaming...");

    // Hash "hello world" in two chunks: "hello " + "world"
    let mut session = crypto.sha256_begin()
        .map_err(|_| Error::Internal)?;
    session.update(b"hello ").map_err(|_| Error::Internal)?;
    session.update(b"world").map_err(|_| Error::Internal)?;
    let hash = session.finalize().map_err(|_| Error::Internal)?;

    // Same expected hash as one-shot "hello world"
    const EXPECTED: [u8; 32] = [
        0xb9, 0x4d, 0x27, 0xb9, 0x93, 0x4d, 0x3e, 0x08,
        0xa5, 0x2e, 0x52, 0xd7, 0xda, 0x7d, 0xab, 0xfa,
        0xc4, 0x84, 0xef, 0xe3, 0x7a, 0x53, 0x80, 0xee,
        0x90, 0x88, 0xf7, 0xac, 0xe2, 0xef, 0xcd, 0xe9,
    ];

    if hash != EXPECTED {
        pw_log::error!("SHA-256 streaming mismatch!");
        return Err(Error::Unknown);
    }

    pw_log::info!("SHA-256 streaming: PASS");
    Ok(())
}

fn test_hmac_sha256(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing HMAC-SHA256...");

    let key = b"secret-key-1234567890123456";
    let data = b"message to authenticate";

    let mac = crypto.hmac_sha256(key, data)
        .map_err(|_| Error::Internal)?;

    if mac.iter().all(|&b| b == 0) {
        pw_log::error!("HMAC-SHA256 returned all zeros!");
        return Err(Error::Unknown);
    }

    pw_log::info!("HMAC-SHA256: PASS");
    Ok(())
}

fn test_hmac_sha384(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing HMAC-SHA384...");

    let key = b"secret-key-1234567890123456";
    let data = b"message to authenticate";

    let mac = crypto.hmac_sha384(key, data)
        .map_err(|_| Error::Internal)?;

    if mac.iter().all(|&b| b == 0) {
        pw_log::error!("HMAC-SHA384 returned all zeros!");
        return Err(Error::Unknown);
    }

    // Verify determinism
    let mac2 = crypto.hmac_sha384(key, data)
        .map_err(|_| Error::Internal)?;

    if mac != mac2 {
        pw_log::error!("HMAC-SHA384 not deterministic!");
        return Err(Error::Unknown);
    }

    pw_log::info!("HMAC-SHA384: PASS");
    Ok(())
}

fn test_hmac_sha512(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing HMAC-SHA512...");

    let key = b"secret-key-1234567890123456";
    let data = b"message to authenticate";

    let mac = crypto.hmac_sha512(key, data)
        .map_err(|_| Error::Internal)?;

    if mac.iter().all(|&b| b == 0) {
        pw_log::error!("HMAC-SHA512 returned all zeros!");
        return Err(Error::Unknown);
    }

    // Verify determinism
    let mac2 = crypto.hmac_sha512(key, data)
        .map_err(|_| Error::Internal)?;

    if mac != mac2 {
        pw_log::error!("HMAC-SHA512 not deterministic!");
        return Err(Error::Unknown);
    }

    pw_log::info!("HMAC-SHA512: PASS");
    Ok(())
}

fn test_aes_gcm(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing AES-256-GCM...");

    let key: [u8; 32] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
        0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
        0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    ];
    let nonce: [u8; 12] = [0x00; 12];
    let plaintext = b"secret message!";

    // Seal
    let mut ciphertext = [0u8; 64];
    let ct_len = crypto.aes256_gcm_seal(&key, &nonce, plaintext, &mut ciphertext)
        .map_err(|_| Error::Internal)?;

    pw_log::info!("Sealed {} bytes -> {} bytes", plaintext.len() as u32, ct_len as u32);

    // Open
    let mut decrypted = [0u8; 64];
    let pt_len = crypto.aes256_gcm_open(&key, &nonce, &ciphertext[..ct_len], &mut decrypted)
        .map_err(|_| Error::Internal)?;

    if pt_len != plaintext.len() {
        pw_log::error!("AES-GCM decrypted length mismatch: {} vs {}", pt_len as u32, plaintext.len() as u32);
        return Err(Error::Unknown);
    }

    if &decrypted[..pt_len] != plaintext {
        pw_log::error!("AES-GCM decrypted content mismatch!");
        return Err(Error::Unknown);
    }

    pw_log::info!("AES-256-GCM: PASS");
    Ok(())
}

#[cfg(feature = "ecdsa")]
fn test_ecdsa_p256(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing ECDSA P-256...");

    let private_key: [u8; 32] = [
        0xc9, 0xaf, 0xa9, 0xd8, 0x45, 0xba, 0x75, 0x16,
        0x6b, 0x5c, 0x21, 0x57, 0x67, 0xb1, 0xd6, 0x93,
        0x4e, 0x50, 0xc3, 0xdb, 0x36, 0xe8, 0x9b, 0x12,
        0x7b, 0x8a, 0x62, 0x2b, 0x12, 0x0f, 0x67, 0x21,
    ];

    let public_key: [u8; 65] = [
        0x04,
        0x60, 0xfe, 0xd4, 0xba, 0x25, 0x5a, 0x9d, 0x31,
        0xc9, 0x61, 0xeb, 0x74, 0xc6, 0x35, 0x6d, 0x68,
        0xc0, 0x49, 0xb8, 0x92, 0x3b, 0x61, 0xfa, 0x6c,
        0xe6, 0x69, 0x62, 0x2e, 0x60, 0xf2, 0x9f, 0xb6,
        0x79, 0x03, 0xfe, 0x10, 0x08, 0xb8, 0xbc, 0x99,
        0xa4, 0x1a, 0xe9, 0xe9, 0x56, 0x28, 0xbc, 0x64,
        0xf2, 0xf1, 0xb2, 0x0c, 0x2d, 0x7e, 0x9f, 0x51,
        0x77, 0xa3, 0xc2, 0x94, 0xd4, 0x46, 0x22, 0x99,
    ];

    let message = b"test message for ECDSA";

    // Sign — returns signature directly
    let signature = crypto.ecdsa_p256_sign(&private_key, message)
        .map_err(|_| Error::Internal)?;

    pw_log::info!("ECDSA P-256 signature generated");

    // Verify — Ok(()) means valid
    crypto.ecdsa_p256_verify(&public_key, message, &signature)
        .map_err(|_| Error::Internal)?;

    // Verify with wrong message should fail
    let wrong_message = b"wrong message";
    let result = crypto.ecdsa_p256_verify(&public_key, wrong_message, &signature);

    if result.is_ok() {
        pw_log::error!("ECDSA P-256 accepted invalid signature!");
        return Err(Error::Unknown);
    }

    pw_log::info!("ECDSA P-256: PASS");
    Ok(())
}

#[cfg(feature = "ecdsa")]
fn test_ecdsa_p384(crypto: &CryptoClient) -> Result<()> {
    pw_log::info!("Testing ECDSA P-384...");

    let private_key: [u8; 48] = [
        0x6b, 0x9d, 0x3d, 0xad, 0x2e, 0x1b, 0x8c, 0x1c,
        0x05, 0xb1, 0x98, 0x75, 0xb6, 0x65, 0x9f, 0x4d,
        0xe2, 0x3c, 0x3b, 0x66, 0x7b, 0xf2, 0x97, 0xba,
        0x9a, 0xa4, 0x77, 0x40, 0x78, 0x71, 0x37, 0xd8,
        0x96, 0xd5, 0x72, 0x4e, 0x4c, 0x70, 0xa8, 0x25,
        0xf8, 0x72, 0xc9, 0xea, 0x60, 0xd2, 0xed, 0xf5,
    ];

    let message = b"test message for ECDSA P-384";

    // Sign — returns signature directly
    let signature = crypto.ecdsa_p384_sign(&private_key, message)
        .map_err(|_| Error::Internal)?;

    pw_log::info!("ECDSA P-384 signature generated");

    // Verify determinism
    let signature2 = crypto.ecdsa_p384_sign(&private_key, message)
        .map_err(|_| Error::Internal)?;

    if signature != signature2 {
        pw_log::error!("ECDSA P-384 signatures not deterministic!");
        return Err(Error::Unknown);
    }

    if signature.iter().all(|&b| b == 0) {
        pw_log::error!("ECDSA P-384 signature is all zeros!");
        return Err(Error::Unknown);
    }

    pw_log::info!("ECDSA P-384: PASS");
    Ok(())
}

fn run_crypto_tests() -> Result<()> {
    pw_log::info!("Starting crypto client tests");

    let crypto = CryptoClient::new(handle::CRYPTO);

    test_sha256(&crypto)?;
    test_sha384(&crypto)?;
    test_sha512(&crypto)?;
    test_sha256_streaming(&crypto)?;
    test_hmac_sha256(&crypto)?;
    test_hmac_sha384(&crypto)?;
    test_hmac_sha512(&crypto)?;
    test_aes_gcm(&crypto)?;
    #[cfg(feature = "ecdsa")]
    {
        test_ecdsa_p256(&crypto)?;
        test_ecdsa_p384(&crypto)?;
    }

    pw_log::info!("All crypto tests PASSED!");
    Ok(())
}

#[entry]
fn entry() -> ! {
    pw_log::info!("🔄 RUNNING");

    let ret = run_crypto_tests();

    if ret.is_err() {
        pw_log::error!("❌ FAILED");
        let _ = syscall::debug_shutdown(ret);
    } else {
        pw_log::info!("✅ PASSED");
        let _ = syscall::debug_shutdown(Ok(()));
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
