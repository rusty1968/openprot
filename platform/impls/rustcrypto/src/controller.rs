// Licensed under the Apache-2.0 license

//! RustCrypto-based Crypto Controller
//!
//! A Hubris-compatible controller that can handle both digest and MAC requests using RustCrypto implementations.
//! This serves as a drop-in backend for Hubris digest servers.

use core::fmt;
use hmac::{Hmac, Mac as HmacTrait};
use openprot_hal_blocking::digest::owned::{DigestInit, DigestOp};
use openprot_hal_blocking::digest::{Digest, Sha2_256, Sha2_384, Sha2_512};
use openprot_hal_blocking::digest::{
    Error as DigestError, ErrorKind as DigestErrorKind, ErrorType as DigestErrorType,
};
use openprot_hal_blocking::mac::owned::{MacInit, MacOp};
use openprot_hal_blocking::mac::{
    Error as MacError, ErrorKind as MacErrorKind, ErrorType as MacErrorType, KeyHandle,
};
use openprot_hal_blocking::mac::{HmacSha2_256, HmacSha2_384, HmacSha2_512};
use sha2::{Digest as Sha2Digest, Sha256, Sha384, Sha512};

/// A type implementing RustCrypto-based hash/digest owned traits.
/// Compatible with Hubris digest server requirements
pub struct RustCryptoController {
    // No state needed - each operation creates its own context
}

impl RustCryptoController {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for RustCryptoController {
    fn default() -> Self {
        Self::new()
    }
}

/// Error type for RustCrypto operations
#[derive(Debug, Clone, PartialEq)]
pub enum CryptoError {
    InvalidKeyLength,
    InvalidOutputLength,
    OperationFailed,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptoError::InvalidKeyLength => write!(f, "Invalid key length"),
            CryptoError::InvalidOutputLength => write!(f, "Invalid output length"),
            CryptoError::OperationFailed => write!(f, "Operation failed"),
        }
    }
}

impl core::error::Error for CryptoError {}

impl DigestError for CryptoError {
    fn kind(&self) -> DigestErrorKind {
        match self {
            CryptoError::InvalidKeyLength => DigestErrorKind::InvalidInputLength,
            CryptoError::InvalidOutputLength => DigestErrorKind::InvalidOutputSize,
            CryptoError::OperationFailed => DigestErrorKind::HardwareFailure,
        }
    }
}

impl MacError for CryptoError {
    fn kind(&self) -> MacErrorKind {
        match self {
            CryptoError::InvalidKeyLength => MacErrorKind::InvalidInputLength,
            CryptoError::InvalidOutputLength => MacErrorKind::InvalidInputLength,
            CryptoError::OperationFailed => MacErrorKind::HardwareFailure,
        }
    }
}

/// Simple byte array key wrapper for software implementations
#[derive(Debug, Clone)]
pub struct ByteArrayKey<'a>(&'a [u8]);

impl<'a> ByteArrayKey<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0
    }
}

impl<'a> KeyHandle for ByteArrayKey<'a> {}

/// Digest contexts for different SHA algorithms
pub struct DigestContext256(Sha256);
pub struct DigestContext384(Sha384);
pub struct DigestContext512(Sha512);

/// MAC contexts for different HMAC algorithms  
pub struct MacContext256(Hmac<Sha256>);
pub struct MacContext384(Hmac<Sha384>);
pub struct MacContext512(Hmac<Sha512>);

// Error type implementations for the controller
impl DigestErrorType for RustCryptoController {
    type Error = CryptoError;
}

impl MacErrorType for RustCryptoController {
    type Error = CryptoError;
}

// Digest initialization - creates SHA-256 context
impl DigestInit<Sha2_256> for RustCryptoController {
    type Context = DigestContext256;
    type Output = Digest<8>; // SHA-256 output as 8 words of 32 bits

    fn init(self, _algorithm: Sha2_256) -> Result<Self::Context, Self::Error> {
        Ok(DigestContext256(Sha256::new()))
    }
}

// Digest initialization - creates SHA-384 context
impl DigestInit<Sha2_384> for RustCryptoController {
    type Context = DigestContext384;
    type Output = Digest<12>; // SHA-384 output as 12 words of 32 bits

    fn init(self, _algorithm: Sha2_384) -> Result<Self::Context, Self::Error> {
        Ok(DigestContext384(Sha384::new()))
    }
}

// Digest initialization - creates SHA-512 context
impl DigestInit<Sha2_512> for RustCryptoController {
    type Context = DigestContext512;
    type Output = Digest<16>; // SHA-512 output as 16 words of 32 bits

    fn init(self, _algorithm: Sha2_512) -> Result<Self::Context, Self::Error> {
        Ok(DigestContext512(Sha512::new()))
    }
}

// SHA-256 digest operations
impl DigestErrorType for DigestContext256 {
    type Error = CryptoError;
}

impl DigestOp for DigestContext256 {
    type Output = Digest<8>; // SHA-256 as Digest<8>
    type Controller = RustCryptoController;

    fn update(mut self, data: &[u8]) -> Result<Self, Self::Error> {
        self.0.update(data);
        Ok(self)
    }

    fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> {
        let result = self.0.finalize();

        // Convert SHA-256 output (32 bytes) to Digest<8> (8 x 32-bit words)
        let mut words = [0u32; 8];

        // Safe iteration with proper bounds checking
        for (i, chunk) in result.chunks_exact(4).enumerate().take(8) {
            if let Ok(bytes) = chunk.try_into() {
                words[i] = u32::from_le_bytes(bytes);
            } else {
                return Err(CryptoError::OperationFailed);
            }
        }

        let digest = Digest::new(words);
        Ok((digest, RustCryptoController::new()))
    }

    fn cancel(self) -> Self::Controller {
        RustCryptoController::new()
    }
}

// SHA-384 digest operations
impl DigestErrorType for DigestContext384 {
    type Error = CryptoError;
}

impl DigestOp for DigestContext384 {
    type Output = Digest<12>; // SHA-384 as Digest<12>
    type Controller = RustCryptoController;

    fn update(mut self, data: &[u8]) -> Result<Self, Self::Error> {
        self.0.update(data);
        Ok(self)
    }

    fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> {
        let result = self.0.finalize();

        // Convert SHA-384 output (48 bytes) to Digest<12> (12 x 32-bit words)
        let mut words = [0u32; 12];

        // Safe iteration with proper bounds checking
        for (i, chunk) in result.chunks_exact(4).enumerate().take(12) {
            if let Ok(bytes) = chunk.try_into() {
                words[i] = u32::from_le_bytes(bytes);
            } else {
                return Err(CryptoError::OperationFailed);
            }
        }

        let digest = Digest::new(words);
        Ok((digest, RustCryptoController::new()))
    }

    fn cancel(self) -> Self::Controller {
        RustCryptoController::new()
    }
}

// SHA-512 digest operations
impl DigestErrorType for DigestContext512 {
    type Error = CryptoError;
}

impl DigestOp for DigestContext512 {
    type Output = Digest<16>; // SHA-512 as Digest<16>
    type Controller = RustCryptoController;

    fn update(mut self, data: &[u8]) -> Result<Self, Self::Error> {
        self.0.update(data);
        Ok(self)
    }

    fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> {
        let result = self.0.finalize();

        // Convert SHA-512 output (64 bytes) to Digest<16> (16 x 32-bit words)
        let mut words = [0u32; 16];

        // Safe iteration with proper bounds checking
        for (i, chunk) in result.chunks_exact(4).enumerate().take(16) {
            if let Ok(bytes) = chunk.try_into() {
                words[i] = u32::from_le_bytes(bytes);
            } else {
                return Err(CryptoError::OperationFailed);
            }
        }

        let digest = Digest::new(words);
        Ok((digest, RustCryptoController::new()))
    }

    fn cancel(self) -> Self::Controller {
        RustCryptoController::new()
    }
}

// MAC initialization - creates HMAC-SHA256 context
impl MacInit<HmacSha2_256> for RustCryptoController {
    type Key = ByteArrayKey<'static>;
    type Context = MacContext256;
    type Output = [u8; 32]; // HMAC-SHA256 output size

    fn init(self, _algorithm: HmacSha2_256, key: Self::Key) -> Result<Self::Context, Self::Error> {
        let hmac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
            .map_err(|_| CryptoError::InvalidKeyLength)?;
        Ok(MacContext256(hmac))
    }
}

// MAC initialization - creates HMAC-SHA384 context
impl MacInit<HmacSha2_384> for RustCryptoController {
    type Key = ByteArrayKey<'static>;
    type Context = MacContext384;
    type Output = [u8; 48]; // HMAC-SHA384 output size

    fn init(self, _algorithm: HmacSha2_384, key: Self::Key) -> Result<Self::Context, Self::Error> {
        let hmac = Hmac::<Sha384>::new_from_slice(key.as_bytes())
            .map_err(|_| CryptoError::InvalidKeyLength)?;
        Ok(MacContext384(hmac))
    }
}

// MAC initialization - creates HMAC-SHA512 context
impl MacInit<HmacSha2_512> for RustCryptoController {
    type Key = ByteArrayKey<'static>;
    type Context = MacContext512;
    type Output = [u8; 64]; // HMAC-SHA512 output size

    fn init(self, _algorithm: HmacSha2_512, key: Self::Key) -> Result<Self::Context, Self::Error> {
        let hmac = Hmac::<Sha512>::new_from_slice(key.as_bytes())
            .map_err(|_| CryptoError::InvalidKeyLength)?;
        Ok(MacContext512(hmac))
    }
}

// HMAC-SHA256 operations
impl MacErrorType for MacContext256 {
    type Error = CryptoError;
}

impl MacOp for MacContext256 {
    type Output = [u8; 32];
    type Controller = RustCryptoController;

    fn update(mut self, data: &[u8]) -> Result<Self, Self::Error> {
        self.0.update(data);
        Ok(self)
    }

    fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> {
        let result = self.0.finalize();
        let mut output = [0u8; 32];
        output.copy_from_slice(&result.into_bytes());
        Ok((output, RustCryptoController::new()))
    }

    fn cancel(self) -> Self::Controller {
        RustCryptoController::new()
    }
}

// HMAC-SHA384 operations
impl MacErrorType for MacContext384 {
    type Error = CryptoError;
}

impl MacOp for MacContext384 {
    type Output = [u8; 48];
    type Controller = RustCryptoController;

    fn update(mut self, data: &[u8]) -> Result<Self, Self::Error> {
        self.0.update(data);
        Ok(self)
    }

    fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> {
        let result = self.0.finalize();
        let mut output = [0u8; 48];
        output.copy_from_slice(&result.into_bytes());
        Ok((output, RustCryptoController::new()))
    }

    fn cancel(self) -> Self::Controller {
        RustCryptoController::new()
    }
}

// HMAC-SHA512 operations
impl MacErrorType for MacContext512 {
    type Error = CryptoError;
}

impl MacOp for MacContext512 {
    type Output = [u8; 64];
    type Controller = RustCryptoController;

    fn update(mut self, data: &[u8]) -> Result<Self, Self::Error> {
        self.0.update(data);
        Ok(self)
    }

    fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> {
        let result = self.0.finalize();
        let mut output = [0u8; 64];
        output.copy_from_slice(&result.into_bytes());
        Ok((output, RustCryptoController::new()))
    }

    fn cancel(self) -> Self::Controller {
        RustCryptoController::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_digest_operations() {
        // Test SHA-256
        let controller = RustCryptoController::new();
        let context = DigestInit::<Sha2_256>::init(controller, Sha2_256).unwrap();
        let context = context.update(b"hello world").unwrap();
        let (hash256, controller) = context.finalize().unwrap();

        // Test Digest<8> properties (compatible with Hubris)
        let hash_array = hash256.into_array(); // Safe conversion
        assert_eq!(hash_array.len(), 8); // 8 x 32-bit words = 256 bits
        let hash_bytes = hash256.as_bytes(); // Zero-copy byte access
        assert_eq!(hash_bytes.len(), 32); // 32 bytes total

        // Test SHA-384
        let context = DigestInit::<Sha2_384>::init(controller, Sha2_384).unwrap();
        let context = context.update(b"hello world").unwrap();
        let (hash384, controller) = context.finalize().unwrap();

        // Test Digest<12> properties
        let hash_array = hash384.into_array(); // Safe conversion
        assert_eq!(hash_array.len(), 12); // 12 x 32-bit words = 384 bits
        let hash_bytes = hash384.as_bytes(); // Zero-copy byte access
        assert_eq!(hash_bytes.len(), 48); // 48 bytes total

        // Test SHA-512
        let context = DigestInit::<Sha2_512>::init(controller, Sha2_512).unwrap();
        let context = context.update(b"hello world").unwrap();
        let (hash512, _controller) = context.finalize().unwrap();

        // Test Digest<16> properties
        let hash_array = hash512.into_array(); // Safe conversion
        assert_eq!(hash_array.len(), 16); // 16 x 32-bit words = 512 bits
        let hash_bytes = hash512.as_bytes(); // Zero-copy byte access
        assert_eq!(hash_bytes.len(), 64); // 64 bytes total
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_mac_operations() {
        let key = ByteArrayKey::new(b"super secret key");

        // Test HMAC-SHA256
        let controller = RustCryptoController::new();
        let context = MacInit::<HmacSha2_256>::init(controller, HmacSha2_256, key.clone()).unwrap();
        let context = context.update(b"hello world").unwrap();
        let (mac256, controller) = context.finalize().unwrap();
        assert_eq!(mac256.len(), 32); // HMAC-SHA256 produces 32 bytes
        assert_ne!(mac256, [0u8; 32]); // Should contain actual MAC data

        // Test HMAC-SHA384
        let context = MacInit::<HmacSha2_384>::init(controller, HmacSha2_384, key.clone()).unwrap();
        let context = context.update(b"hello world").unwrap();
        let (mac384, controller) = context.finalize().unwrap();
        assert_eq!(mac384.len(), 48); // HMAC-SHA384 produces 48 bytes
        assert_ne!(mac384, [0u8; 48]); // Should contain actual MAC data

        // Test HMAC-SHA512
        let context = MacInit::<HmacSha2_512>::init(controller, HmacSha2_512, key).unwrap();
        let context = context.update(b"hello world").unwrap();
        let (mac512, _controller) = context.finalize().unwrap();
        assert_eq!(mac512.len(), 64); // HMAC-SHA512 produces 64 bytes
        assert_ne!(mac512, [0u8; 64]); // Should contain actual MAC data
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_mixed_operations_controller_recovery() {
        let controller = RustCryptoController::new();

        // Use controller for SHA-256 digest
        let digest_ctx = DigestInit::<Sha2_256>::init(controller, Sha2_256).unwrap();
        let digest_ctx = digest_ctx.update(b"test data").unwrap();
        let (hash256, controller) = digest_ctx.finalize().unwrap();

        // Use recovered controller for HMAC-SHA384
        let key = ByteArrayKey::new(b"key");
        let mac_ctx = MacInit::<HmacSha2_384>::init(controller, HmacSha2_384, key).unwrap();
        let mac_ctx = mac_ctx.update(b"test data").unwrap();
        let (mac384, controller) = mac_ctx.finalize().unwrap();

        // Use recovered controller for SHA-512 digest
        let digest_ctx = DigestInit::<Sha2_512>::init(controller, Sha2_512).unwrap();
        let digest_ctx = digest_ctx.update(b"more data").unwrap();
        let (hash512, _controller) = digest_ctx.finalize().unwrap();

        // Verify operations completed successfully
        // hash256 is Digest<8>, mac384 and hash512 are byte arrays
        let hash256_bytes = hash256.as_bytes();
        assert_eq!(hash256_bytes.len(), 32); // SHA-256 output
        assert_eq!(mac384.len(), 48); // HMAC-SHA384 output
        let hash512_bytes = hash512.as_bytes();
        assert_eq!(hash512_bytes.len(), 64); // SHA-512 output
    }
}
