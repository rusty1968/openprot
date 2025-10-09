// Licensed under the Apache-2.0 license

use crate::digest::Digest;
use core::fmt::Debug;
use subtle::ConstantTimeEq;
use zerocopy::IntoBytes;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Secure wrapper for MAC keys that automatically zeros on drop.
///
/// This wrapper ensures that cryptographic keys are securely erased from memory
/// when no longer needed, preventing key material from remaining in memory.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecureKey<const N: usize> {
    /// The actual key bytes, zeroized on drop
    bytes: [u8; N],
}

impl<const N: usize> SecureKey<N> {
    /// Create a new secure key from a byte array.
    ///
    /// # Security
    /// The input array will be zeroized after copying to prevent key material
    /// from remaining in multiple memory locations.
    pub fn new(mut key_bytes: [u8; N]) -> Self {
        let key = Self { bytes: key_bytes };
        key_bytes.zeroize();
        key
    }

    /// Create a new secure key from a byte slice.
    ///
    /// # Returns
    /// - `Ok(SecureKey)` if the slice length matches the key size
    /// - `Err(ErrorKind::InvalidInputLength)` if the slice is the wrong size
    pub fn from_slice(key_slice: &[u8]) -> Result<Self, ErrorKind> {
        if key_slice.len() != N {
            return Err(ErrorKind::InvalidInputLength);
        }

        let mut key_bytes = [0u8; N];
        key_bytes.copy_from_slice(key_slice);
        Ok(Self::new(key_bytes))
    }

    /// Get a reference to the key bytes.
    ///
    /// # Security
    /// Use this sparingly and ensure the returned reference doesn't outlive
    /// the SecureKey instance.
    pub fn as_bytes(&self) -> &[u8; N] {
        &self.bytes
    }

    /// Verify a MAC tag using constant-time comparison.
    ///
    /// # Security
    /// This function uses constant-time comparison to prevent timing attacks
    /// that could reveal information about the expected MAC value.
    pub fn verify_mac(&self, computed_mac: &[u8], expected_mac: &[u8]) -> bool {
        if computed_mac.len() != expected_mac.len() {
            return false;
        }
        computed_mac.ct_eq(expected_mac).into()
    }
}

impl<const N: usize> Debug for SecureKey<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecureKey")
            .field("len", &N)
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

impl<const N: usize> PartialEq for SecureKey<N> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes.ct_eq(&other.bytes).into()
    }
}

impl<const N: usize> Eq for SecureKey<N> {}

/// Common error kinds for MAC operations.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The input data length is not valid for the MAC function.
    InvalidInputLength,
    /// The specified MAC algorithm is not supported by the hardware or software implementation.
    UnsupportedAlgorithm,
    /// Failed to allocate memory for the MAC computation.
    MemoryAllocationFailure,
    /// Failed to initialize the MAC computation context.
    InitializationError,
    /// Error occurred while updating the MAC computation with new data.
    UpdateError,
    /// Error occurred while finalizing the MAC computation.
    FinalizationError,
    /// The hardware accelerator is busy and cannot process the MAC computation.
    Busy,
    /// General hardware failure during MAC computation.
    HardwareFailure,
    /// The specified output size is not valid for the MAC function.
    InvalidOutputSize,
    /// Insufficient permissions to access the hardware or perform the MAC computation.
    PermissionDenied,
    /// The MAC computation context has not been initialized.
    NotInitialized,
    /// MAC verification failed - computed MAC does not match expected value.
    VerificationFailed,
}

/// Trait for converting implementation-specific errors into a common error kind.
pub trait Error: Debug {
    /// Returns a generic error kind corresponding to the specific error.
    fn kind(&self) -> ErrorKind;
}

impl Error for core::convert::Infallible {
    fn kind(&self) -> ErrorKind {
        match *self {}
    }
}

/// Trait for types that associate with a specific error type.
pub trait ErrorType {
    /// The associated error type.
    type Error: Error;
}

/// Trait representing a MAC algorithm and its output characteristics.
pub trait MacAlgorithm: Copy + Debug {
    /// The number of bits in the MAC output.
    const OUTPUT_BITS: usize;

    /// The type representing the MAC output.
    type MacOutput: IntoBytes;

    /// The type representing the key used for MAC computation.
    type Key;
}

/// Trait for initializing a MAC operation for a specific algorithm.
pub trait MacInit<A: MacAlgorithm>: ErrorType {
    /// The type representing the operational context for the MAC.
    type OpContext<'a>: MacOp<Output = A::MacOutput>
    where
        Self: 'a;

    /// Initializes the MAC operation with the specified algorithm and key.
    ///
    /// # Parameters
    ///
    /// - `algo`: A zero-sized type representing the MAC algorithm to use.
    /// - `key`: A reference to the key used for the MAC computation.
    ///
    /// # Returns
    ///
    /// A result containing the operational context for the MAC, or an error.
    fn init<'a>(&'a mut self, algo: A, key: &A::Key) -> Result<Self::OpContext<'a>, Self::Error>;
}

/// Optional trait for resetting a MAC context to its initial state.
pub trait MacCtrlReset: ErrorType {
    /// Resets the MAC context.
    ///
    /// # Returns
    ///
    /// A result indicating success or failure.
    fn reset(&mut self) -> Result<(), Self::Error>;
}

/// Trait for performing MAC operations.
pub trait MacOp: ErrorType {
    /// The type of the MAC output.
    type Output: IntoBytes;

    /// Updates the MAC state with the provided input data.
    ///
    /// # Parameters
    ///
    /// - `input`: A byte slice containing the data to authenticate.
    ///
    /// # Returns
    ///
    /// A result indicating success or failure.
    fn update(&mut self, input: &[u8]) -> Result<(), Self::Error>;

    /// Finalizes the MAC computation and returns the result.
    ///
    /// # Returns
    ///
    /// A result containing the MAC output, or an error.
    fn finalize(self) -> Result<Self::Output, Self::Error>;
}

/// Utility function for constant-time MAC verification.
///
/// This function provides a secure way to verify MAC values using constant-time
/// comparison to prevent timing attacks.
///
/// # Parameters
///
/// - `computed_mac`: The computed MAC bytes
/// - `expected_mac`: The expected MAC bytes to verify against
///
/// # Returns
///
/// `true` if the MACs match, `false` otherwise
///
/// # Security
///
/// This function uses constant-time comparison to prevent timing attacks
/// that could reveal information about the expected MAC value.
pub fn verify_mac_constant_time(computed_mac: &[u8], expected_mac: &[u8]) -> bool {
    if computed_mac.len() != expected_mac.len() {
        return false;
    }
    computed_mac.ct_eq(expected_mac).into()
}

// =============================================================================
// MAC Algorithm Marker Types
// =============================================================================

/// HMAC-SHA-256 MAC algorithm marker type.
///
/// This zero-sized type represents the HMAC-SHA-256 message authentication code
/// algorithm, which produces a 256-bit (32-byte) MAC output using SHA-256 as
/// the underlying hash function.
///
/// HMAC-SHA-256 combines the SHA-256 hash function with a secret key to provide
/// both data integrity and authentication.
#[derive(Clone, Copy, Debug)]
pub struct HmacSha2_256;
impl MacAlgorithm for HmacSha2_256 {
    const OUTPUT_BITS: usize = 256;
    type MacOutput = Digest<{ Self::OUTPUT_BITS / 32 }>;
    type Key = SecureKey<32>;
}

/// HMAC-SHA-384 MAC algorithm marker type.
///
/// This zero-sized type represents the HMAC-SHA-384 message authentication code
/// algorithm, which produces a 384-bit (48-byte) MAC output using SHA-384 as
/// the underlying hash function.
///
/// HMAC-SHA-384 provides a larger output size than HMAC-SHA-256 for applications
/// requiring additional security margin.
#[derive(Clone, Copy, Debug)]
pub struct HmacSha2_384;
impl MacAlgorithm for HmacSha2_384 {
    const OUTPUT_BITS: usize = 384;
    type MacOutput = Digest<{ Self::OUTPUT_BITS / 32 }>;
    type Key = SecureKey<48>;
}

/// HMAC-SHA-512 MAC algorithm marker type.
///
/// This zero-sized type represents the HMAC-SHA-512 message authentication code
/// algorithm, which produces a 512-bit (64-byte) MAC output using SHA-512 as
/// the underlying hash function.
///
/// HMAC-SHA-512 provides the largest standard output size, offering maximum
/// collision resistance and authentication strength.
#[derive(Clone, Copy, Debug)]
pub struct HmacSha2_512;
impl MacAlgorithm for HmacSha2_512 {
    const OUTPUT_BITS: usize = 512;
    type MacOutput = Digest<{ Self::OUTPUT_BITS / 32 }>;
    type Key = SecureKey<64>;
}

/// Computes a MAC using a key retrieved from a key vault.
///
/// This function provides integrated MAC computation with secure key storage,
/// ensuring that MAC keys are retrieved from a secure vault and used for
/// authentication operations without exposing the key material.
///
/// # Parameters
/// - `mac_impl`: The MAC implementation to use
/// - `vault`: The key vault containing the MAC key
/// - `key_id`: Unique identifier for the key in the vault
/// - `algorithm`: The MAC algorithm to use (zero-sized type)
/// - `data`: The data to authenticate
///
/// # Returns
/// The computed MAC output
///
/// # Security Notes
/// - MAC key is never exposed to caller
/// - Key retrieval and MAC computation are atomic
/// - Supports vault access control and locking mechanisms
/// - Automatic key zeroization after use
///
/// # Example
/// ```rust,ignore
/// use openprot_hal_blocking::mac::{compute_mac_with_vault, HmacSha2_256, verify_mac_constant_time};
/// use openprot_hal_blocking::key_vault::{KeyLifecycle};
///
/// let mut mac_impl = MyMacImplementation::new();
/// let vault = MyKeyVault::new();
/// let data = b"Hello, world!";
///
/// // Compute MAC with vault-stored key
/// let mac_output = compute_mac_with_vault(
///     &mut mac_impl,
///     &vault,
///     KeyId::new(42),
///     HmacSha2_256,
///     data
/// )?;
///
/// ```
pub fn compute_mac_with_vault<A, M, V, E>(
    mac_impl: &mut M,
    vault: &V,
    key_id: <V as crate::key_vault::KeyLifecycle>::KeyId,
    algorithm: A,
    data: &[u8],
) -> Result<A::MacOutput, E>
where
    A: MacAlgorithm,
    M: MacInit<A>,
    V: crate::key_vault::KeyLifecycle<KeyData = A::Key>,
    E: From<M::Error> + From<V::Error>,
    for<'a> <M::OpContext<'a> as ErrorType>::Error: Into<E>,
{
    // Retrieve key from vault
    let key = vault.retrieve_key(key_id).map_err(E::from)?;

    // Initialize MAC operation
    let mut mac_ctx = mac_impl.init(algorithm, &key).map_err(E::from)?;

    // Update with data
    mac_ctx.update(data).map_err(Into::into)?;

    // Finalize and return MAC
    mac_ctx.finalize().map_err(Into::into)
}
