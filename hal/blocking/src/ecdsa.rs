// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! ECDSA digital-signature traits (no-std HAL interface).
//!
//! Generic over an elliptic `Curve`. Software keys add the `Serializable*`
//! marker traits; hardware keys (HSM/enclave) implement only the base traits.
//!
//! Error model (one rule, applied uniformly):
//! - data-type traits (`validate`, `from_coordinates`) return [`ErrorKind`]
//!   directly — a structural check has no implementation-specific error;
//! - operation traits (`EcdsaKeyGen`/`EcdsaSign`/`EcdsaVerify`) return an
//!   opaque `Self::Error: Error`, mappable to [`ErrorKind`] via [`Error::kind`].
//!
//! `Curve` intentionally carries no domain parameters (order/modulus); checks
//! that need them (`k < n`, on-curve) are out of this interface's scope and
//! must live in the implementation if required.

use crate::digest::DigestAlgorithm;
use core::fmt::Debug;
use zerocopy::Immutable;
use zerocopy::{FromBytes, IntoBytes};
use zeroize::Zeroize;

/// Map an implementation error to a generic [`ErrorKind`].
///
/// ```rust
/// # use openprot_hal_blocking::ecdsa::{Error, ErrorKind};
/// #[derive(Debug)]
/// enum MyErr { Hw, Bad, Timeout }
/// impl Error for MyErr {
///     fn kind(&self) -> ErrorKind {
///         match self {
///             MyErr::Hw => ErrorKind::Other,
///             MyErr::Bad => ErrorKind::InvalidKeyFormat,
///             MyErr::Timeout => ErrorKind::Busy,
///         }
///     }
/// }
/// ```
pub trait Error: core::fmt::Debug {
    /// Generic classification of this error.
    fn kind(&self) -> ErrorKind;
}

impl Error for core::convert::Infallible {
    fn kind(&self) -> ErrorKind {
        match *self {}
    }
}

/// Associates a type with its ECDSA error.
///
/// ```rust
/// # use openprot_hal_blocking::ecdsa::{ErrorType, Error, ErrorKind};
/// # #[derive(Debug)]
/// # enum MyError { Failed }
/// # impl Error for MyError { fn kind(&self) -> ErrorKind { ErrorKind::Other } }
/// struct Dev;
/// impl ErrorType for Dev { type Error = MyError; }
/// ```
pub trait ErrorType {
    /// Error type.
    type Error: Error;
}

/// Generic ECDSA error kinds. `#[non_exhaustive]` for forward compatibility.
///
/// Security: do not let `kind()` distinguish secrets (e.g. "key not found"
/// vs "wrong key") — that leaks a timing/oracle signal.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Hardware/implementation busy; retry later.
    Busy,
    /// Signature verification failed (bad sig, wrong key, or modified message).
    InvalidSignature,
    /// Key generation failed (entropy/hardware).
    KeyGenError,
    /// Signing computation failed.
    SigningError,
    /// Key data could not be parsed / wrong length / bad encoding.
    InvalidKeyFormat,
    /// Point is not a valid curve point.
    InvalidPoint,
    /// Curve or algorithm not supported by this implementation.
    UnsupportedCurve,
    /// Mathematically weak key (zero, identity, equal to order, …).
    WeakKey,
    /// Unspecified error; prefer a specific kind where possible.
    Other,
}

/// Abstract elliptic curve: the digest it pairs with and its scalar/coordinate
/// byte type. Carries no domain parameters by design (see module docs).
pub trait Curve {
    /// Hash used for message digests on this curve.
    type DigestType: DigestAlgorithm;
    /// Scalar / coordinate byte representation (e.g. `[u8; 48]` for P-384).
    type Scalar: IntoBytes + FromBytes;
}

/// ECC private key for curve `C`. Must zeroize its secret material.
///
/// Hardware keys may hold only a handle and zeroize that.
pub trait PrivateKey<C: Curve>: Zeroize {
    /// Structural validity (e.g. non-zero). Range check `1 < k < n` needs the
    /// curve order, which `Curve` does not expose — out of scope here.
    ///
    /// `Err(WeakKey)` if zero/weak; `Err(InvalidKeyFormat)` if malformed.
    fn validate(&self) -> Result<(), ErrorKind>;
}

/// ECDSA signature `(r, s)` for curve `C`.
pub trait Signature<C: Curve> {
    /// Build from `(r, s)`. Rejects zero `r`/`s` (`InvalidSignature`); full
    /// `r,s < n` needs the curve order (not in `Curve`) — out of scope.
    fn from_coordinates(r: C::Scalar, s: C::Scalar) -> Result<Self, ErrorKind>
    where
        Self: Sized;

    /// Return `(r, s)` by value (scalars are `Copy` byte arrays).
    fn coordinates(&self) -> (C::Scalar, C::Scalar);
}

/// ECC public key (a curve point) for curve `C`.
pub trait PublicKey<C: Curve> {
    /// Return `(x, y)` by value.
    fn coordinates(&self) -> (C::Scalar, C::Scalar);

    /// Build from `(x, y)`. Rejects the all-zero point (`InvalidPoint`); a
    /// true on-curve check needs domain parameters (not in `Curve`) — out of
    /// scope, must be done by the implementation if required.
    fn from_coordinates(x: C::Scalar, y: C::Scalar) -> Result<Self, ErrorKind>
    where
        Self: Sized;
}

/// Opt-in serialization for software private keys (exposes secret bytes —
/// software keys only).
pub trait SerializablePrivateKey<C: Curve>: PrivateKey<C> + IntoBytes + FromBytes {}

/// Opt-in serialization for public keys.
pub trait SerializablePublicKey<C: Curve>: PublicKey<C> + IntoBytes + FromBytes {}

/// Opt-in serialization for signatures.
pub trait SerializableSignature<C: Curve>: Signature<C> + IntoBytes + FromBytes {}

/// ECDSA key-pair generation over curve `C`. Requires a CSPRNG.
pub trait EcdsaKeyGen<C: Curve>: ErrorType {
    /// Private key type.
    type PrivateKey: PrivateKey<C>;
    /// Public key type.
    type PublicKey: PublicKey<C>;

    /// Generate a key pair from a cryptographic RNG.
    fn generate_keypair<R>(
        &mut self,
        rng: &mut R,
    ) -> Result<(Self::PrivateKey, Self::PublicKey), Self::Error>
    where
        R: rand_core::RngCore + rand_core::CryptoRng;
}

/// ECDSA signing over curve `C`. Requires a CSPRNG for the nonce.
pub trait EcdsaSign<C: Curve>: ErrorType {
    /// Private key type.
    type PrivateKey: PrivateKey<C>;
    /// Signature type.
    type Signature: Signature<C>;

    /// Sign a digest produced by `C::DigestType`.
    fn sign<R>(
        &mut self,
        private_key: &Self::PrivateKey,
        digest: <<C as Curve>::DigestType as DigestAlgorithm>::Digest,
        rng: &mut R,
    ) -> Result<Self::Signature, Self::Error>
    where
        R: rand_core::RngCore + rand_core::CryptoRng;
}

/// ECDSA verification over curve `C`. Implementations should be constant-time
/// where feasible.
pub trait EcdsaVerify<C: Curve>: ErrorType {
    /// Public key type.
    type PublicKey: PublicKey<C>;
    /// Signature type.
    type Signature: Signature<C>;

    /// Verify `signature` over `digest` under `public_key`.
    fn verify(
        &mut self,
        public_key: &Self::PublicKey,
        digest: <<C as Curve>::DigestType as DigestAlgorithm>::Digest,
        signature: &Self::Signature,
    ) -> Result<(), Self::Error>;
}

/// NIST P-384 (secp384r1), ~192-bit security, SHA-384, 48-byte scalars.
/// FIPS 186-4 / RFC 5480. The only curve this crate ships concrete key and
/// signature types for; other platforms add their own marker + types.
#[derive(Clone, Copy, Debug)]
pub struct P384;

impl Curve for P384 {
    type DigestType = crate::digest::Sha2_384;
    type Scalar = [u8; 48];
}

/// P-384 public key: uncompressed `(x, y)`, 48 bytes each.
#[derive(Clone, Debug, IntoBytes, FromBytes, Immutable)]
#[repr(C)]
pub struct P384PublicKey {
    x: [u8; 48],
    y: [u8; 48],
}

impl P384PublicKey {
    /// New key from raw coordinates.
    pub fn new(x: [u8; 48], y: [u8; 48]) -> Self {
        Self { x, y }
    }
}

impl PublicKey<P384> for P384PublicKey {
    fn coordinates(&self) -> ([u8; 48], [u8; 48]) {
        (self.x, self.y)
    }

    fn from_coordinates(x: [u8; 48], y: [u8; 48]) -> Result<Self, ErrorKind> {
        // Checkable invariant only: reject the all-zero point.
        if x.iter().all(|&b| b == 0) || y.iter().all(|&b| b == 0) {
            return Err(ErrorKind::InvalidPoint);
        }
        Ok(Self::new(x, y))
    }
}

impl SerializablePublicKey<P384> for P384PublicKey {}

/// P-384 ECDSA signature: `(r, s)`, 48 bytes each.
#[derive(Clone, Debug, IntoBytes, FromBytes, Immutable)]
#[repr(C)]
pub struct P384Signature {
    r: [u8; 48],
    s: [u8; 48],
}

impl P384Signature {
    /// New signature from raw `(r, s)`.
    pub fn new(r: [u8; 48], s: [u8; 48]) -> Self {
        Self { r, s }
    }
}

impl Signature<P384> for P384Signature {
    fn from_coordinates(r: [u8; 48], s: [u8; 48]) -> Result<Self, ErrorKind> {
        // Checkable invariant only: reject zero r/s (full r,s < n needs order).
        if r.iter().all(|&b| b == 0) || s.iter().all(|&b| b == 0) {
            return Err(ErrorKind::InvalidSignature);
        }
        Ok(Self::new(r, s))
    }

    fn coordinates(&self) -> ([u8; 48], [u8; 48]) {
        (self.r, self.s)
    }
}

impl SerializableSignature<P384> for P384Signature {}
