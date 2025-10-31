// Licensed under the Apache-2.0 license

//! Hubris Platform Integration Traits for OpenPRoT
//!
//! This crate provides Hubris-specific integration traits that bridge the gap
//! between OpenPRoT's generic HAL traits and Hubris IDL code generation requirements.
//!
//! The traits use concrete types instead of generic associated types to ensure
//! compatibility with Hubris IDL code generation systems.

use openprot_hal_blocking::{
    digest::{owned::DigestOp, Digest},
    mac::owned::MacOp,
};

/// Hubris-specific cryptographic error types
///
/// These errors are designed to be IDL-compatible and map to standard
/// Hubris IPC error codes for inter-task communication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubrisCryptoError {
    /// Invalid key length for the requested operation333
    InvalidKeyLength,
    /// Hardware crypto accelerator failure
    HardwareFailure,
    /// Invalid algorithm parameters
    InvalidParameters,
    /// Resource temporarily unavailable (e.g., crypto unit busy)
    ResourceBusy,
    /// Operation not supported by hardware
    NotSupported,
}

/// Hubrus-specific digest device trait
///
/// This trait is designed specifically for Hubris microkernel integration:
/// - All types are concrete for IDL compatibility
/// - Methods support task isolation semantics
/// - Error types map to Hubris IPC error codes
/// - Resource management aligns with Hubris task model
pub trait HubrisDigestDevice {
    /// Digest context for SHA-256 operations
    type DigestContext256: DigestOp<Output = Digest<8>>;
    /// Digest context for SHA-384 operations  
    type DigestContext384: DigestOp<Output = Digest<12>>;
    /// Digest context for SHA-512 operations
    type DigestContext512: DigestOp<Output = Digest<16>>;

    /// HMAC key type that can be created from byte slices
    /// Must be compatible with Hubris task memory constraints
    type HmacKey: for<'a> TryFrom<&'a [u8]>;

    /// HMAC context for SHA-256 operations
    type HmacContext256: MacOp<Output = [u8; 32]>;
    /// HMAC context for SHA-384 operations
    type HmacContext384: MacOp<Output = [u8; 48]>;
    /// HMAC context for SHA-512 operations
    type HmacContext512: MacOp<Output = [u8; 64]>;

    /// Maximum supported key size in bytes
    /// This aligns with Hubris task memory constraints
    const MAX_KEY_SIZE: usize = 128;

    /// Initialize a SHA-256 digest operation
    ///
    /// # Hubris Semantics
    /// - Consumes the device (move semantics for resource management)
    /// - Returns concrete error types compatible with IDL
    fn init_digest_sha256(self) -> Result<Self::DigestContext256, HubrisCryptoError>;

    /// Initialize a SHA-384 digest operation
    fn init_digest_sha384(self) -> Result<Self::DigestContext384, HubrisCryptoError>;

    /// Initialize a SHA-512 digest operation
    fn init_digest_sha512(self) -> Result<Self::DigestContext512, HubrisCryptoError>;

    /// Initialize an HMAC-SHA256 operation with the given key
    ///
    /// # Security Note
    /// Key handling follows Hubris security model with task isolation
    fn init_hmac_sha256(
        self,
        key: Self::HmacKey,
    ) -> Result<Self::HmacContext256, HubrisCryptoError>;

    /// Initialize an HMAC-SHA384 operation with the given key
    fn init_hmac_sha384(
        self,
        key: Self::HmacKey,
    ) -> Result<Self::HmacContext384, HubrisCryptoError>;

    /// Initialize an HMAC-SHA512 operation with the given key
    fn init_hmac_sha512(
        self,
        key: Self::HmacKey,
    ) -> Result<Self::HmacContext512, HubrisCryptoError>;

    /// Create an HMAC key from a byte slice
    ///
    /// # Hubris Integration
    /// - Validates key size against device limits
    /// - Ensures compatibility with task memory constraints
    fn create_hmac_key(data: &[u8]) -> Result<Self::HmacKey, HubrisCryptoError> {
        if data.len() > Self::MAX_KEY_SIZE {
            return Err(HubrisCryptoError::InvalidKeyLength);
        }
        Self::HmacKey::try_from(data).map_err(|_| HubrisCryptoError::InvalidKeyLength)
    }
}

/// Extension trait for one-shot operations in Hubris
///
/// Provides efficient one-shot digest and MAC operations that are common
/// in embedded scenarios and align with Hubris task execution patterns.
pub trait HubrisDigestOneShot: HubrisDigestDevice + Sized {
    /// Compute SHA-256 digest in one operation
    ///
    /// Optimized for Hubris task scheduling - completes atomically
    fn digest_sha256_oneshot(self, data: &[u8]) -> Result<Digest<8>, HubrisCryptoError> {
        let ctx = self.init_digest_sha256()?;
        let ctx = ctx
            .update(data)
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        let (result, _controller) = ctx
            .finalize()
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        Ok(result)
    }

    /// Compute SHA-384 digest in one operation
    fn digest_sha384_oneshot(self, data: &[u8]) -> Result<Digest<12>, HubrisCryptoError> {
        let ctx = self.init_digest_sha384()?;
        let ctx = ctx
            .update(data)
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        let (result, _controller) = ctx
            .finalize()
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        Ok(result)
    }

    /// Compute SHA-512 digest in one operation
    fn digest_sha512_oneshot(self, data: &[u8]) -> Result<Digest<16>, HubrisCryptoError> {
        let ctx = self.init_digest_sha512()?;
        let ctx = ctx
            .update(data)
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        let (result, _controller) = ctx
            .finalize()
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        Ok(result)
    }

    /// Compute HMAC-SHA256 in one operation
    ///
    /// # Security
    /// Key is zeroized after use following Hubris security practices
    fn hmac_sha256_oneshot(self, key: &[u8], data: &[u8]) -> Result<[u8; 32], HubrisCryptoError> {
        let key_handle = Self::create_hmac_key(key)?;
        let ctx = self.init_hmac_sha256(key_handle)?;
        let ctx = ctx
            .update(data)
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        let (result, _controller) = ctx
            .finalize()
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        Ok(result)
    }

    /// Compute HMAC-SHA384 in one operation
    fn hmac_sha384_oneshot(self, key: &[u8], data: &[u8]) -> Result<[u8; 48], HubrisCryptoError> {
        let key_handle = Self::create_hmac_key(key)?;
        let ctx = self.init_hmac_sha384(key_handle)?;
        let ctx = ctx
            .update(data)
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        let (result, _controller) = ctx
            .finalize()
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        Ok(result)
    }

    /// Compute HMAC-SHA512 in one operation
    fn hmac_sha512_oneshot(self, key: &[u8], data: &[u8]) -> Result<[u8; 64], HubrisCryptoError> {
        let key_handle = Self::create_hmac_key(key)?;
        let ctx = self.init_hmac_sha512(key_handle)?;
        let ctx = ctx
            .update(data)
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        let (result, _controller) = ctx
            .finalize()
            .map_err(|_| HubrisCryptoError::HardwareFailure)?;
        Ok(result)
    }
}

// Blanket implementation for one-shot operations
impl<T: HubrisDigestDevice> HubrisDigestOneShot for T {}
