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

/// Simple byte array key wrapper for software implementations (borrowed data)
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

/// Secure owned key type that owns its data on the stack (no allocation required)
/// Maximum key size is 128 bytes to support all HMAC variants (SHA-512 block size)
/// This solves the lifetime issue where ByteArrayKey<'static> cannot be created from local data
#[derive(Debug, Clone)]
pub struct SecureOwnedKey {
    data: [u8; 128], // Fixed size buffer - no allocation needed
    len: usize,      // Actual key length
}

impl SecureOwnedKey {
    /// Maximum supported key length (128 bytes for SHA-512 block size)
    pub const MAX_KEY_SIZE: usize = 128;

    /// Create a new secure key by copying the provided bytes
    /// Returns error if key is too large for our fixed buffer
    pub fn new(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() > Self::MAX_KEY_SIZE {
            return Err(CryptoError::InvalidKeyLength);
        }

        let mut data = [0u8; 128];
        data[..bytes.len()].copy_from_slice(bytes);

        Ok(Self {
            data,
            len: bytes.len(),
        })
    }

    /// Create a secure key from a fixed-size array (zero-copy for arrays up to 128 bytes)
    pub fn from_array<const N: usize>(array: [u8; N]) -> Result<Self, CryptoError> {
        if N > Self::MAX_KEY_SIZE {
            return Err(CryptoError::InvalidKeyLength);
        }

        let mut data = [0u8; 128];
        data[..N].copy_from_slice(&array);

        Ok(Self { data, len: N })
    }

    /// Get the key bytes as a slice (only the valid portion)
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// Get the key length
    pub fn len(&self) -> usize {
        self.len
    }

    /// Check if the key is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl KeyHandle for SecureOwnedKey {}

impl TryFrom<&[u8]> for SecureOwnedKey {
    type Error = CryptoError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::new(bytes)
    }
}

impl<const N: usize> TryFrom<[u8; N]> for SecureOwnedKey {
    type Error = CryptoError;

    fn try_from(array: [u8; N]) -> Result<Self, Self::Error> {
        Self::from_array(array)
    }
}

impl<const N: usize> TryFrom<&[u8; N]> for SecureOwnedKey {
    type Error = CryptoError;

    fn try_from(array: &[u8; N]) -> Result<Self, Self::Error> {
        Self::new(array)
    }
}

// Implement Drop to securely zero the key data when dropped
impl Drop for SecureOwnedKey {
    fn drop(&mut self) {
        // Securely zero the key data
        self.data.fill(0);
    }
}

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
                if let Some(word) = words.get_mut(i) {
                    *word = u32::from_le_bytes(bytes);
                } else {
                    return Err(CryptoError::OperationFailed);
                }
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
                if let Some(word) = words.get_mut(i) {
                    *word = u32::from_le_bytes(bytes);
                } else {
                    return Err(CryptoError::OperationFailed);
                }
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
                if let Some(word) = words.get_mut(i) {
                    *word = u32::from_le_bytes(bytes);
                } else {
                    return Err(CryptoError::OperationFailed);
                }
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
    type Key = SecureOwnedKey;
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
    type Key = SecureOwnedKey;
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
    type Key = SecureOwnedKey;
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
        let key = SecureOwnedKey::new(b"super secret key").unwrap();

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
        let key = SecureOwnedKey::new(b"key").unwrap();
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

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_secure_owned_key() {
        // Test creation from slice
        let key1 = SecureOwnedKey::new(b"test key").unwrap();
        assert_eq!(key1.as_bytes(), b"test key");
        assert_eq!(key1.len(), 8);
        assert!(!key1.is_empty());

        // Test creation from fixed array
        let key2 = SecureOwnedKey::from_array(*b"another key").unwrap();
        assert_eq!(key2.as_bytes(), b"another key");
        assert_eq!(key2.len(), 11);

        // Test TryFrom trait implementations
        let key3: SecureOwnedKey = b"from slice".as_slice().try_into().unwrap();
        assert_eq!(key3.as_bytes(), b"from slice");

        let key4: SecureOwnedKey = (*b"from fixed array").try_into().unwrap();
        assert_eq!(key4.as_bytes(), b"from fixed array");

        let key5: SecureOwnedKey = b"from array ref".try_into().unwrap();
        assert_eq!(key5.as_bytes(), b"from array ref");

        // Test cloning
        let key6 = key1.clone();
        assert_eq!(key6.as_bytes(), key1.as_bytes());

        // Test empty key
        let empty_key = SecureOwnedKey::new(&[]).unwrap();
        assert!(empty_key.is_empty());
        assert_eq!(empty_key.len(), 0);

        // Test maximum key size (should succeed)
        let max_key_data = [0u8; SecureOwnedKey::MAX_KEY_SIZE];
        let max_key = SecureOwnedKey::new(&max_key_data).unwrap();
        assert_eq!(max_key.len(), SecureOwnedKey::MAX_KEY_SIZE);

        // Test oversized key (should fail)
        let oversized_data = [0u8; SecureOwnedKey::MAX_KEY_SIZE + 1];
        let result = SecureOwnedKey::new(&oversized_data);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), CryptoError::InvalidKeyLength);
    }
}
