// Licensed under the Apache-2.0 license

//! # ECDSA Digital Signature Operations
//!
//! This module provides a comprehensive, type-safe abstraction for Elliptic Curve Digital
//! Signature Algorithm (ECDSA) operations. The design follows security best practices and
//! provides a generic interface that can work with any elliptic curve.
//!
//! ## Features
//!
//! - **Type Safety**: Generic over curve types to prevent cross-curve contamination
//! - **Security First**: Mandatory cryptographic RNG, proper key validation, secure memory clearing
//! - **No-std Compatible**: Works in embedded environments without standard library
//! - **Comprehensive Error Handling**: Detailed error types for proper debugging and security
//! - **Zero-copy Serialization**: Efficient serialization using `zerocopy` traits
//!
//! ## Architecture
//!
//! The module follows a trait-based design with the following key components:
//!
//! ```text
//! Curve (Abstract EC Parameters)
//! ├── DigestType: DigestAlgorithm
//! └── Scalar: IntoBytes + FromBytes
//!
//! Key Management
//! ├── PrivateKey<C>: Zeroize + Serialization + Validation
//! └── PublicKey<C>: Serialization + Coordinate Access + Validation
//!
//! Signatures
//! └── Signature<C>: Serialization + Component Access + Validation
//!
//! Operations
//! ├── EcdsaKeyGen<C>: Key pair generation
//! ├── EcdsaSign<C>: Digital signing with RNG
//! └── EcdsaVerify<C>: Signature verification
//!
//! Error Handling
//! ├── Error: Debug → ErrorKind mapping
//! ├── ErrorType: Associated error types
//! └── ErrorKind: Common error classifications
//! ```
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! # use openprot_hal_blocking::ecdsa::*;
//! # use rand_core::{RngCore, CryptoRng};
//! #
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // This example shows the basic pattern for ECDSA operations
//! // Actual implementations would provide concrete curve types
//!
//! // Key generation pattern:
//! // let mut key_generator = YourKeyGenImpl::new();
//! // let mut rng = YourCryptoRng::new();
//! // let (private_key, public_key) = key_generator.generate_keypair(&mut rng)?;
//!
//! // Key validation pattern:
//! // private_key.validate()?;
//! // public_key.validate()?;
//!
//! // Signing pattern:
//! // let mut signer = YourSignerImpl::new();
//! // let message_digest = your_hash_function(message);
//! // let signature = signer.sign(&private_key, &message_digest, &mut rng)?;
//!
//! // Verification pattern:
//! // let mut verifier = YourVerifierImpl::new();
//! // let is_valid = verifier.verify(&public_key, &message_digest, &signature)?;
//!
//! # Ok(())
//! # }
//! ```
//!
//! ## Security Considerations
//!
//! - **Always validate inputs**: Use the `validate()` methods on keys and signatures
//! - **Use cryptographic RNG**: Only `CryptoRng + RngCore` is accepted for signing
//! - **Clear sensitive data**: Private keys implement `Zeroize` for secure memory clearing
//! - **Constant-time operations**: Implementers should use constant-time algorithms where possible
//! - **Side-channel protection**: Be aware of timing attacks in verification operations

use crate::digest::DigestAlgorithm;
use core::fmt::Debug;
use zerocopy::{FromBytes, IntoBytes};
use zeroize::Zeroize;

/// Trait for converting implementation-specific ECDSA errors into generic error kinds.
///
/// This trait allows HAL implementations to define their own detailed error types
/// while still providing a common interface for generic code to handle errors.
///
/// # Example
///
/// ```rust
/// # use openprot_hal_blocking::ecdsa::{Error, ErrorKind};
/// #[derive(Debug)]
/// enum MyEcdsaError {
///     HardwareFault,
///     InvalidParameters,
///     Timeout,
/// }
///
/// impl Error for MyEcdsaError {
///     fn kind(&self) -> ErrorKind {
///         match self {
///             MyEcdsaError::HardwareFault => ErrorKind::Other,
///             MyEcdsaError::InvalidParameters => ErrorKind::InvalidKeyFormat,
///             MyEcdsaError::Timeout => ErrorKind::Busy,
///         }
///     }
/// }
/// ```
pub trait Error: core::fmt::Debug {
    /// Convert error to a generic error kind
    ///
    /// By using this method, errors freely defined by HAL implementations
    /// can be converted to a set of generic errors upon which generic
    /// code can act.
    fn kind(&self) -> ErrorKind;
}

impl Error for core::convert::Infallible {
    fn kind(&self) -> ErrorKind {
        match *self {}
    }
}

/// Trait for associating a type with an ECDSA error type.
///
/// This trait is used throughout the ECDSA module to associate operations
/// with their specific error types while maintaining type safety.
///
/// # Example
///
/// ```rust
/// # use openprot_hal_blocking::ecdsa::{ErrorType, Error, ErrorKind};
/// # #[derive(Debug)]
/// # enum MyError { Failed }
/// # impl Error for MyError {
/// #     fn kind(&self) -> ErrorKind { ErrorKind::Other }
/// # }
/// struct MyEcdsaDevice;
///
/// impl ErrorType for MyEcdsaDevice {
///     type Error = MyError;
/// }
/// ```
pub trait ErrorType {
    /// Error type.
    type Error: Error;
}

/// Error kind for ECDSA operations.
///
/// This represents a common set of ECDSA operation errors that can occur across
/// different implementations. The enum is `#[non_exhaustive]` to allow for future
/// additions without breaking API compatibility.
///
/// Implementations are free to define more specific or additional error types.
/// However, by providing a mapping to these common errors through the [`Error::kind`]
/// method, generic code can still react to them appropriately.
///
/// # Security Note
///
/// Error types should not leak sensitive information. For example, avoid
/// distinguishing between "key not found" and "wrong key" errors, as this
/// could provide timing attack vectors.
///
/// # Examples
///
/// ```rust
/// # use openprot_hal_blocking::ecdsa::ErrorKind;
/// # let error_kind = ErrorKind::Busy;
/// match error_kind {
///     ErrorKind::InvalidSignature => {
///         // Handle signature verification failure
///         eprintln!("Signature verification failed");
///     }
///     ErrorKind::WeakKey => {
///         // Handle weak key detection
///         eprintln!("Weak key detected - regenerate keypair");
///     }
///     ErrorKind::Busy => {
///         // Handle resource busy - retry later
///         eprintln!("ECDSA hardware is busy, retry later");
///     }
///     _ => {
///         // Handle other errors
///         eprintln!("ECDSA operation failed: {:?}", error_kind);
///     }
/// }
/// ```
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The operation is busy and cannot be completed
    ///
    /// This indicates that the hardware or implementation is currently
    /// busy with another operation. The caller should retry later.
    Busy,

    /// The signature is invalid
    ///
    /// Returned when signature verification fails. This could indicate:
    /// - The signature was corrupted during transmission
    /// - The signature was created with a different key
    /// - The message was modified after signing
    /// - The signature components (r, s) are invalid
    InvalidSignature,

    /// Key generation failed
    ///
    /// Indicates that the key generation process could not complete successfully.
    /// This might be due to insufficient entropy, hardware failures, or other
    /// random number generation issues.
    KeyGenError,

    /// Signing operation failed
    ///
    /// The signing process encountered an error. This is distinct from key
    /// generation errors and typically indicates issues during the actual
    /// signing computation.
    SigningError,

    /// Invalid key format or encoding
    ///
    /// The provided key data could not be parsed or is in an unsupported format.
    /// This includes issues with:
    /// - Incorrect key length
    /// - Invalid encoding (DER, PEM, etc.)
    /// - Malformed key structure
    InvalidKeyFormat,

    /// Point is not on the curve
    ///
    /// The provided coordinates do not represent a valid point on the specified
    /// elliptic curve. This is a critical security check that prevents attacks
    /// using invalid curve points.
    InvalidPoint,

    /// Unsupported curve or algorithm
    ///
    /// The requested elliptic curve or algorithm parameters are not supported
    /// by this implementation. Common reasons include:
    /// - Curve not implemented in hardware
    /// - Disabled curve due to security concerns
    /// - Incompatible curve parameters
    UnsupportedCurve,

    /// Weak key detected (e.g., zero key, key equal to curve order)
    ///
    /// The key fails cryptographic strength requirements. This includes:
    /// - Zero private keys
    /// - Private keys equal to the curve order
    /// - Public keys at the identity point
    /// - Other mathematically weak keys
    WeakKey,

    /// Other unspecified error
    ///
    /// A catch-all for errors that don't fit into the specific categories above.
    /// Implementations should prefer specific error types when possible.
    Other,
}

/// Trait for ECC private keys associated with a specific curve.
///
/// Private keys must implement secure memory clearing through the [`Zeroize`] trait
/// to ensure cryptographic material is properly destroyed when no longer needed.
///
/// # Security Requirements
///
/// Implementations must:
/// - Validate keys are within the valid scalar range (1 < key < curve_order)
/// - Implement constant-time operations where possible
/// - Clear sensitive data from memory using [`Zeroize`]
///
/// # Example
///
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::{PrivateKey, Curve, ErrorKind};
/// use zeroize::Zeroize;
///
/// struct MyPrivateKey([u8; 32]);
///
/// impl<C: Curve> PrivateKey<C> for MyPrivateKey {
///     fn validate(&self) -> Result<(), ErrorKind> {
///         // Check if key is zero or equal to curve order
///         if self.0.iter().all(|&b| b == 0) {
///             return Err(ErrorKind::WeakKey);
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait PrivateKey<C: Curve>: IntoBytes + FromBytes + Zeroize {
    /// Validate that this private key is valid for the curve.
    ///
    /// This method should verify that the private key is within the valid
    /// range for the curve (typically 1 < key < curve_order).
    ///
    /// # Returns
    /// - `Ok(())`: The private key is valid
    /// - `Err(ErrorKind::WeakKey)`: The key is zero, equal to curve order, or otherwise weak
    /// - `Err(ErrorKind::InvalidKeyFormat)`: The key format is invalid
    fn validate(&self) -> Result<(), ErrorKind>;
}

/// A trait representing an abstract elliptic curve with associated types for cryptographic operations.
///
/// This trait defines the fundamental components required for elliptic curve cryptography,
/// including the digest algorithm for hashing operations and the scalar field element type
/// for curve arithmetic.
///
/// # Associated Types
///
/// - [`DigestType`]: The hash function used for message digests in signing operations
/// - [`Scalar`]: The field element type representing curve coordinates and private keys
///
/// # Example
///
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::{Curve, DigestAlgorithm};
/// use openprot_hal_blocking::digest::Sha256;
///
/// struct P256;
///
/// impl Curve for P256 {
///     type DigestType = Sha256;
///     type Scalar = [u8; 32];
/// }
/// ```
///
/// [`DigestType`]: Curve::DigestType
/// [`Scalar`]: Curve::Scalar
pub trait Curve {
    /// The digest algorithm used by this elliptic curve for cryptographic operations.
    type DigestType: DigestAlgorithm;
    /// The scalar field element type used in elliptic curve operations.
    type Scalar: IntoBytes + FromBytes;
}

/// Trait for ECDSA signatures associated with a specific curve.
///
/// This trait provides access to signature components and validation methods
/// for ECDSA signatures over elliptic curves.
///
/// # Security Considerations
///
/// - Signature components (r, s) must be in the range [1, curve_order)
/// - Zero values for r or s make the signature invalid
/// - Implementations should validate signatures before use
///
/// # Example
///
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::{Signature, Curve, ErrorKind};
///
/// struct MySignature {
///     r: [u8; 32],
///     s: [u8; 32],
/// }
///
/// impl<C: Curve> Signature<C> for MySignature {
///     fn r(&self) -> &C::Scalar {
///         // Return reference to r component
///         unimplemented!()
///     }
///
///     fn s(&self) -> &C::Scalar {
///         // Return reference to s component  
///         unimplemented!()
///     }
///
///     fn from_components(r: C::Scalar, s: C::Scalar) -> Result<Self, ErrorKind> {
///         // Validate components and create signature
///         unimplemented!()
///     }
///
///     fn new_unchecked(r: C::Scalar, s: C::Scalar) -> Self {
///         // Create signature without validation (use with caution)
///         unimplemented!()
///     }
/// }
/// ```
pub trait Signature<C: Curve>: IntoBytes + FromBytes {
    /// Get the r component of the signature.
    fn r(&self) -> &C::Scalar;
    /// Get the s component of the signature.
    fn s(&self) -> &C::Scalar;

    /// Create a new signature from r and s components with validation.
    ///
    /// This method validates that both r and s are within the valid range
    /// for the curve (typically 1 ≤ r,s < curve_order).
    ///
    /// # Parameters
    /// - `r`: The r component of the signature
    /// - `s`: The s component of the signature
    ///
    /// # Returns
    /// - `Ok(Self)`: Valid signature
    /// - `Err(ErrorKind::InvalidSignature)`: If r or s are invalid (zero or ≥ curve order)
    fn from_components(r: C::Scalar, s: C::Scalar) -> Result<Self, ErrorKind>
    where
        Self: Sized;

    /// Validate that this signature has valid r and s components.
    ///
    /// # Returns
    /// - `Ok(())`: The signature components are valid
    /// - `Err(ErrorKind::InvalidSignature)`: Invalid r or s component
    fn validate(&self) -> Result<(), ErrorKind>;

    /// Create a new signature from r and s components without validation.
    ///
    /// # Safety
    /// This method should only be used when the caller can guarantee that
    /// r and s are valid for the curve. Use `from_components` for safe
    /// construction with validation.
    fn new_unchecked(r: C::Scalar, s: C::Scalar) -> Self;
}

/// A trait representing a public key associated with a specific elliptic curve.
/// Trait for ECC public keys associated with a specific curve.
///
/// Public keys represent points on elliptic curves and are used for signature verification.
/// This trait provides coordinate access and validation methods to ensure keys represent
/// valid curve points.
///
/// # Security Considerations
///
/// - Points must lie on the specified elliptic curve
/// - The identity element (point at infinity) is typically invalid for cryptographic use
/// - Always validate public keys before using them for verification
///
/// # Example
///
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::{PublicKey, Curve, ErrorKind};
///
/// struct MyPublicKey {
///     x: [u8; 32],
///     y: [u8; 32],
/// }
///
/// impl<C: Curve> PublicKey<C> for MyPublicKey {
///     fn x(&self) -> &C::Scalar {
///         // Return reference to x coordinate
///         unimplemented!()
///     }
///
///     fn y(&self) -> &C::Scalar {
///         // Return reference to y coordinate
///         unimplemented!()
///     }
///
///     fn from_coordinates(x: C::Scalar, y: C::Scalar) -> Result<Self, ErrorKind> {
///         // Validate point is on curve and create public key
///         unimplemented!()
///     }
///
///     fn validate(&self) -> Result<(), ErrorKind> {
///         // Check if point lies on the curve
///         unimplemented!()
///     }
///
///     fn new_unchecked(x: C::Scalar, y: C::Scalar) -> Self {
///         // Create key without validation (use with caution)
///         unimplemented!()
///     }
/// }
/// ```
pub trait PublicKey<C: Curve>: IntoBytes + FromBytes {
    /// Get the x coordinate of the public key.
    fn x(&self) -> &C::Scalar;
    /// Get the y coordinate of the public key.
    fn y(&self) -> &C::Scalar;

    /// Create a new public key from x and y coordinates with validation.
    ///
    /// This method validates that the point (x, y) lies on the specified curve.
    ///
    /// # Parameters
    /// - `x`: The x coordinate of the point
    /// - `y`: The y coordinate of the point  
    ///
    /// # Returns
    /// - `Ok(Self)`: Valid public key if the point is on the curve
    /// - `Err(ErrorKind::InvalidPoint)`: If the point is not on the curve
    /// - `Err(ErrorKind::WeakKey)`: If the point is the identity element
    fn from_coordinates(x: C::Scalar, y: C::Scalar) -> Result<Self, ErrorKind>
    where
        Self: Sized;

    /// Validate that this public key represents a valid point on the curve.
    ///
    /// This method should be called before using a public key for verification
    /// to ensure it represents a valid curve point and is not a weak key.
    ///
    /// # Returns
    /// - `Ok(())`: The public key is valid
    /// - `Err(ErrorKind::InvalidPoint)`: The point is not on the curve
    /// - `Err(ErrorKind::WeakKey)`: The point is the identity element or otherwise weak
    fn validate(&self) -> Result<(), ErrorKind>;

    /// Create a new public key from x and y coordinates without validation.
    ///
    /// # Safety
    /// This method should only be used when the caller can guarantee that
    /// the coordinates represent a valid point on the curve. Use `from_coordinates`
    /// for safe construction with validation.
    fn new_unchecked(x: C::Scalar, y: C::Scalar) -> Self;
}

/// Trait for ECDSA key generation over a specific elliptic curve.
///
/// This trait enables generation of cryptographically secure ECDSA key pairs
/// using a cryptographic random number generator.
///
/// # Security Requirements
///
/// - Must use a cryptographically secure random number generator
/// - Generated keys must be uniformly distributed over the valid scalar range
/// - Private keys must be properly zeroized after use
///
/// # Example
///
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::{EcdsaKeyGen, Curve, ErrorKind};
/// use rand_core::{RngCore, CryptoRng};
///
/// struct MyKeyGenerator;
///
/// impl<C: Curve> EcdsaKeyGen<C> for MyKeyGenerator {
///     type PrivateKey = MyPrivateKey;
///     type PublicKey = MyPublicKey;
///
///     fn generate_keypair<R>(&mut self, rng: &mut R)
///         -> Result<(Self::PrivateKey, Self::PublicKey), Self::Error>
///     where
///         R: RngCore + CryptoRng,
///     {
///         // Generate cryptographically secure key pair
///         unimplemented!()
///     }
/// }
/// ```
pub trait EcdsaKeyGen<C: Curve>: ErrorType {
    /// The type representing the private key for the curve.
    type PrivateKey: PrivateKey<C>;
    /// The type representing the public key for the curve.
    type PublicKey: PublicKey<C>;

    /// Generates an ECDSA key pair.
    ///
    /// # Parameters
    /// - `rng`: A cryptographically secure random number generator.
    ///
    /// # Returns
    /// A tuple containing the generated private key and public key.
    fn generate_keypair<R>(
        &mut self,
        rng: &mut R,
    ) -> Result<(Self::PrivateKey, Self::PublicKey), Self::Error>
    where
        R: rand_core::RngCore + rand_core::CryptoRng;
}

/// Trait for ECDSA signing using a digest algorithm.
///
/// This trait provides ECDSA signature generation from message digests.
/// The digest should be produced by a cryptographically secure hash function
/// that matches the curve's security level.
///
/// # Security Considerations
///
/// - Use cryptographically secure random number generators for nonce generation
/// - Ensure digest length matches the curve's security level
/// - Private keys must be validated before use
///
/// # Example
///
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::{EcdsaSign, Curve, PrivateKey};
/// use rand_core::{RngCore, CryptoRng};
///
/// struct MySigner;
///
/// impl<C: Curve> EcdsaSign<C> for MySigner {
///     type PrivateKey = MyPrivateKey;
///     type Signature = MySignature;
///
///     fn sign<R>(&mut self, key: &Self::PrivateKey, digest: &[u8], rng: &mut R)
///         -> Result<Self::Signature, Self::Error>
///     where
///         R: RngCore + CryptoRng,
///     {
///         // Generate ECDSA signature
///         unimplemented!()
///     }
/// }
/// ```
pub trait EcdsaSign<C: Curve>: ErrorType {
    /// The type representing the private key for the curve.
    type PrivateKey: PrivateKey<C>;
    /// The type representing the signature for the curve.
    type Signature: Signature<C>;

    /// Signs a digest produced by a compatible hash function.
    ///
    /// # Parameters
    /// - `private_key`: The private key used for signing.
    /// - `digest`: The digest output from a hash function.
    /// - `rng`: A cryptographically secure random number generator.
    fn sign<R>(
        &mut self,
        private_key: &Self::PrivateKey,
        digest: <<C as Curve>::DigestType as DigestAlgorithm>::Digest,
        rng: &mut R,
    ) -> Result<Self::Signature, Self::Error>
    where
        R: rand_core::RngCore + rand_core::CryptoRng;
}

/// Trait for ECDSA signature verification using a digest algorithm.
///
/// This trait provides ECDSA signature verification against message digests
/// using public keys. Verification should be performed in constant time
/// where possible to prevent timing attacks.
///
/// # Security Considerations
///
/// - Always validate public keys before verification
/// - Validate signature components are within valid ranges
/// - Use constant-time comparison operations when possible
///
/// # Example
///
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::{EcdsaVerify, Curve, PublicKey};
///
/// struct MyVerifier;
///
/// impl<C: Curve> EcdsaVerify<C> for MyVerifier {
///     type PublicKey = MyPublicKey;
///     type Signature = MySignature;
///
///     fn verify(&mut self, key: &Self::PublicKey, digest: &[u8], signature: &Self::Signature)
///         -> Result<(), Self::Error>
///     {
///         // Verify ECDSA signature
///         unimplemented!()
///     }
/// }
/// ```
pub trait EcdsaVerify<C: Curve>: ErrorType {
    /// The type representing the public key for the curve.
    type PublicKey: PublicKey<C>;
    /// The type representing the signature for the curve.
    type Signature: Signature<C>;

    /// Verifies a signature against a digest.
    ///
    /// # Parameters
    /// - `public_key`: The public key used for verification.
    /// - `digest`: The digest output from a hash function.
    /// - `signature`: The signature to verify.
    fn verify(
        &mut self,
        public_key: &Self::PublicKey,
        digest: <<C as Curve>::DigestType as DigestAlgorithm>::Digest,
        signature: &Self::Signature,
    ) -> Result<(), Self::Error>;
}

/// secp256k1 elliptic curve marker type.
///
/// This zero-sized type represents the secp256k1 elliptic curve, widely used in
/// Bitcoin and other cryptocurrencies. It provides ~128-bit security level.
///
/// ## Parameters
/// - **Field Size**: 256 bits
/// - **Security Level**: ~128 bits
/// - **Standard**: SEC 2
/// - **Common Uses**: Bitcoin, Ethereum, cryptocurrency applications
///
/// ## Example
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::Secp256k1;
///
/// // Type-safe key generation for secp256k1
/// let key_gen = MyKeyGenerator;
/// let (private_key, public_key) = key_gen.generate_keypair(Secp256k1)?;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Secp256k1;

impl Curve for Secp256k1 {
    type DigestType = crate::digest::Sha2_256;
    type Scalar = [u8; 32];
}

/// NIST P-256 elliptic curve marker type.
///
/// This zero-sized type represents the NIST P-256 elliptic curve (secp256r1).
/// This is the most widely used elliptic curve for ECDSA, providing ~128-bit security.
///
/// ## Parameters
/// - **Field Size**: 256 bits
/// - **Security Level**: ~128 bits
/// - **Standard**: FIPS 186-4, RFC 5480
/// - **Common Uses**: TLS certificates, JWT signing, general-purpose cryptography
///
/// ## Example
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::P256;
///
/// // Type-safe key generation for P-256
/// let key_gen = MyKeyGenerator;
/// let (private_key, public_key) = key_gen.generate_keypair(P256)?;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct P256;

impl Curve for P256 {
    type DigestType = crate::digest::Sha2_256;
    type Scalar = [u8; 32];
}

/// NIST P-384 elliptic curve marker type.
///
/// This zero-sized type represents the NIST P-384 elliptic curve (secp384r1).
/// Provides higher security level than P-256 with ~192-bit security.
///
/// ## Parameters
/// - **Field Size**: 384 bits
/// - **Security Level**: ~192 bits
/// - **Standard**: FIPS 186-4, RFC 5480
/// - **Common Uses**: High-security applications, government systems
///
/// ## Example
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::P384;
///
/// // Type-safe key generation for P-384
/// let key_gen = MyKeyGenerator;
/// let (private_key, public_key) = key_gen.generate_keypair(P384)?;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct P384;

impl Curve for P384 {
    type DigestType = crate::digest::Sha2_384;
    type Scalar = [u8; 48];
}

/// NIST P-521 elliptic curve marker type.
///
/// This zero-sized type represents the NIST P-521 elliptic curve (secp521r1).
/// Provides maximum security among NIST curves with ~256-bit security.
///
/// ## Parameters
/// - **Field Size**: 521 bits
/// - **Security Level**: ~256 bits
/// - **Standard**: FIPS 186-4, RFC 5480
/// - **Common Uses**: Maximum security applications, long-term archival
///
/// ## Example
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::P521;
///
/// // Type-safe key generation for P-521
/// let key_gen = MyKeyGenerator;
/// let (private_key, public_key) = key_gen.generate_keypair(P521)?;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct P521;

impl Curve for P521 {
    type DigestType = crate::digest::Sha2_512;
    type Scalar = [u8; 66];
}

/// Brainpool P256r1 elliptic curve marker type.
///
/// This zero-sized type represents the Brainpool P256r1 elliptic curve.
/// Brainpool curves are alternative curves to NIST curves with potentially better properties.
///
/// ## Parameters
/// - **Field Size**: 256 bits
/// - **Security Level**: ~128 bits
/// - **Standard**: RFC 5639
/// - **Common Uses**: Alternative to NIST curves, European standards
///
/// ## Example
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::BrainpoolP256r1;
///
/// // Type-safe key generation for Brainpool P256r1
/// let key_gen = MyKeyGenerator;
/// let (private_key, public_key) = key_gen.generate_keypair(BrainpoolP256r1)?;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct BrainpoolP256r1;

impl Curve for BrainpoolP256r1 {
    type DigestType = crate::digest::Sha2_256;
    type Scalar = [u8; 32];
}

/// Brainpool P384r1 elliptic curve marker type.
///
/// This zero-sized type represents the Brainpool P384r1 elliptic curve.
/// Provides higher security level than P256r1 with 192-bit security.
///
/// ## Parameters
/// - **Field Size**: 384 bits
/// - **Security Level**: ~192 bits
/// - **Standard**: RFC 5639
/// - **Common Uses**: High-security applications, European standards
///
/// ## Example
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::BrainpoolP384r1;
///
/// // Type-safe key generation for Brainpool P384r1
/// let key_gen = MyKeyGenerator;
/// let (private_key, public_key) = key_gen.generate_keypair(BrainpoolP384r1)?;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct BrainpoolP384r1;

impl Curve for BrainpoolP384r1 {
    type DigestType = crate::digest::Sha2_384;
    type Scalar = [u8; 48];
}

/// Brainpool P512r1 elliptic curve marker type.
///
/// This zero-sized type represents the Brainpool P512r1 elliptic curve.
/// Provides maximum security among Brainpool curves with ~256-bit security.
///
/// ## Parameters
/// - **Field Size**: 512 bits
/// - **Security Level**: ~256 bits
/// - **Standard**: RFC 5639
/// - **Common Uses**: Maximum security applications, European standards
///
/// ## Example
/// ```rust,ignore
/// use openprot_hal_blocking::ecdsa::BrainpoolP512r1;
///
/// // Type-safe key generation for Brainpool P512r1
/// let key_gen = MyKeyGenerator;
/// let (private_key, public_key) = key_gen.generate_keypair(BrainpoolP512r1)?;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct BrainpoolP512r1;

impl Curve for BrainpoolP512r1 {
    type DigestType = crate::digest::Sha2_512;
    type Scalar = [u8; 64];
}
