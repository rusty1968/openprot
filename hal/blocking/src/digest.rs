// Licensed under the Apache-2.0 license

//! # Digest HAL Traits
//!
//! This module provides blocking/synchronous Hardware Abstraction Layer (HAL) traits
//! for cryptographic digest operations. It defines a common interface for hash functions
//! and message authentication codes that can be implemented by various hardware and
//! software backends.
//!
//! ## API Evolution
//!
//! This module provides two complementary APIs:
//!
//! ### Scoped API (Current)
//! - **Use case**: One-shot operations, simple baremetal applications
//! - **Pattern**: Borrowed contexts with lifetime constraints
//! - **Benefits**: Minimal overhead, direct hardware mapping
//! - **Limitations**: Cannot store contexts, no persistent sessions
//!
//! ### Owned API (New - Move-based Resource Management)
//! - **Use case**: Server applications, persistent sessions, IPC boundaries
//! - **Pattern**: Owned contexts with resource recovery
//! - **Benefits**: Persistent storage, multiple concurrent contexts, IPC-safe
//! - **Limitations**: Slightly more complex ownership model
//!
//! ## Key Components
//!
//! - `Digest` - A generic container for digest output values
//! - `DigestAlgorithm` - Trait defining digest algorithm properties
//!
//! ### Scoped API
//! - `scoped::DigestInit` - Trait for initializing digest operations (borrowed contexts)
//! - `scoped::DigestContext` - Trait for performing digest computations (borrowed contexts)
//! - `scoped::DigestCtrlReset` - Trait for resetting digest contexts
//!
//! ### Owned API (built on streaming)
//! - `owned::DigestInit` - Factory trait returning `Owned<Context, Controller>`
//! - `owned::Owned` - Re-export of `streaming::Owned<T, C>` for move semantics
//!
//! ### Streaming API
//! - `streaming::DigestContext` - Core operation trait with `&mut self` methods
//! - `streaming::DigestContextFactory` - Factory for creating contexts
//! - `streaming::Owned<T, C>` - Typestate wrapper for resource recovery
//!
//! The `streaming` module provides the core abstraction. The `owned` module
//! builds on it to provide a factory pattern with move-based resource management.
//!
//! ## Supported Algorithms
//!
//! This module includes support for:
//! - SHA-2 family: SHA-256, SHA-384, SHA-512
//! - SHA-3 family: SHA3-224, SHA3-256, SHA3-384, SHA3-512
//!
//! ## Example Usage
//!
//! ### Scoped API (Traditional)
//! ```rust,no_run
//! # use openprot_hal_blocking::digest::*;
//! # use openprot_hal_blocking::digest::scoped::*;
//! # struct MyDigestImpl;
//! # impl ErrorType for MyDigestImpl { type Error = core::convert::Infallible; }
//! # impl DigestInit<Sha2_256> for MyDigestImpl {
//! #     type OpContext<'a> = MyContext<'a> where Self: 'a;
//! #     type Output = Digest<8>;
//! #     fn init<'a>(&'a mut self, _: Sha2_256) -> Result<Self::OpContext<'a>, Self::Error> { todo!() }
//! # }
//! # struct MyContext<'a>(&'a mut MyDigestImpl);
//! # impl ErrorType for MyContext<'_> { type Error = core::convert::Infallible; }
//! # impl DigestContext for MyContext<'_> {
//! #     type Output = Digest<8>;
//! #     fn update(&mut self, _: &[u8]) -> Result<(), Self::Error> { Ok(()) }
//! #     fn finalize(self) -> Result<Self::Output, Self::Error> {
//! #         Ok(Digest { value: [0u32; 8] })
//! #     }
//! # }
//! let mut hasher = MyDigestImpl;
//! let mut ctx = hasher.init(Sha2_256)?;
//! ctx.update(b"hello world")?;
//! let digest = ctx.finalize()?;
//! # Ok::<(), core::convert::Infallible>(())
//! ```
//!
//! ### Owned API (Move-based - for servers/sessions)
//! ```rust,ignore
//! use openprot_hal_blocking::digest::*;
//! use openprot_hal_blocking::digest::streaming::DigestContext;
//! use openprot_hal_blocking::digest::owned::{DigestInit, Owned};
//!
//! // Implement streaming::DigestContext for your backend
//! impl DigestContext for MyStreamingContext {
//!     const OUTPUT_SIZE: usize = 32;
//!     fn update(&mut self, data: &[u8]) -> Result<(), Self::Error> { /* ... */ }
//!     fn finalize(&mut self, output: &mut [u8]) -> Result<usize, Self::Error> { /* ... */ }
//!     fn reset(&mut self) { /* ... */ }
//! }
//!
//! // Implement owned::DigestInit to create Owned wrappers
//! impl DigestInit<Sha2_256> for MyController {
//!     type Context = MyStreamingContext;
//!     fn init(self, algo: Sha2_256) -> Result<Owned<Self::Context, Self>, Self::Error> {
//!         Ok(Owned::new(MyStreamingContext::new(), self))
//!     }
//! }
//!
//! // Usage
//! let controller = MyController::new();
//! let owned_ctx = controller.init(Sha2_256)?;
//! let owned_ctx = owned_ctx.update(b"hello world")?;
//! let mut output = [0u8; 32];
//! let (len, recovered_controller) = owned_ctx.finalize(&mut output)?;
//! ```

use core::fmt::Debug;
use core::result::Result;
use zerocopy::{FromBytes, Immutable, IntoBytes};

/// A generic digest output container.
///
/// This structure represents the output of a cryptographic digest operation.
/// It uses a const generic parameter `N` to specify the number of 32-bit words
/// in the digest output, allowing it to accommodate different digest sizes.
///
/// The structure is marked with `#[repr(C)]` to ensure a predictable memory layout,
/// making it suitable for zero-copy operations and hardware interfaces.
///
/// ## Integration Benefits
///
/// The `Digest<N>` type solves several critical integration challenges:
///
/// ### 1. Concrete vs Opaque Types
/// Unlike opaque associated types (`type Output: IntoBytes`), `Digest<N>` provides
/// a **concrete type** that generic code can work with directly:
///
/// ```rust
/// # use openprot_hal_blocking::digest::Digest;
/// // ✅ CONCRETE: We know exactly what this is
/// fn process_digest(digest: Digest<8>) -> [u32; 8] {
///     digest.into_array()  // Safe, direct conversion
/// }
///
/// // ❌ OPAQUE: We don't know what D::Output actually is  
/// // fn process_generic<D>(output: D::Output) -> /* Unknown type */ {
/// //     // Cannot convert to [u32; 8] safely
/// // }
/// ```
///
/// ### 2. Safe Type Conversions
/// Provides safe methods to access underlying data without unsafe code:
///
/// ```rust
/// # use openprot_hal_blocking::digest::Digest;
/// let digest = Digest::<8> { value: [1, 2, 3, 4, 5, 6, 7, 8] };
///
/// // Safe conversions - no unsafe code needed
/// let array: [u32; 8] = digest.into_array();      // Owned conversion
/// let array_ref: &[u32; 8] = digest.as_array();   // Borrowed conversion  
/// let bytes: &[u8] = digest.as_bytes();           // Byte slice access
/// ```
///
/// ### 3. IPC Integration
/// Designed specifically for Hubris IPC leased memory operations:
///
/// ```rust,no_run
/// # use openprot_hal_blocking::digest::Digest;
/// # struct Leased<T, U>(core::marker::PhantomData<(T, U)>);
/// # impl<T, U> Leased<T, U> { fn write(&self, data: U) -> Result<(), ()> { Ok(()) } }
/// # let digest_out: Leased<(), [u32; 8]> = Leased(core::marker::PhantomData);
/// # let digest = Digest::<8> { value: [0; 8] };
/// // Direct write to IPC lease - no conversion needed
/// digest_out.write(digest.into_array())?;
/// # Ok::<(), ()>(())
/// ```
///
/// ### 4. Server Application Support  
/// Enables servers to store and manipulate digest results safely:
///
/// ```rust
/// # use openprot_hal_blocking::digest::Digest;
/// struct DigestCache {
///     sha256_results: Vec<Digest<8>>,   // Can store concrete types
///     sha384_results: Vec<Digest<12>>,  // Different sizes supported
/// }
///
/// impl DigestCache {
///     fn store_sha256(&mut self, digest: Digest<8>) {
///         self.sha256_results.push(digest);  // Direct storage
///     }
///     
///     fn get_as_array(&self, index: usize) -> [u32; 8] {
///         self.sha256_results[index].into_array()  // Safe access
///     }
/// }
/// ```
///
/// ### 5. Zero-Copy Operations
/// Full zerocopy trait support enables efficient memory operations:
///
/// ```rust
/// # use openprot_hal_blocking::digest::Digest;
/// let digest = Digest::<8> { value: [1, 2, 3, 4, 5, 6, 7, 8] };
///
/// // Zero-copy byte access via zerocopy traits
/// let bytes: &[u8] = zerocopy::IntoBytes::as_bytes(&digest);
///
/// // Safe transmutation between compatible layouts
/// // (enabled by FromBytes + Immutable derives)
/// ```
///
/// ## Comparison with Opaque Output Types
///
/// | Feature | `Digest<N>` (Concrete) | `D::Output` (Opaque) |
/// |---------|-------------------------|----------------------|
/// | **Type Known at Compile Time** | ✅ Always `Digest<N>` | ❌ Unknown until runtime |
/// | **Safe Array Access** | ✅ `into_array()`, `as_array()` | ❌ Requires unsafe casting |
/// | **IPC Integration** | ✅ Direct `[u32; N]` conversion | ❌ Complex type bridging |
/// | **Server Storage** | ✅ Can store in structs | ❌ Difficult generic storage |
/// | **Zero-Copy Support** | ✅ Full zerocopy traits | ❌ Implementation dependent |
/// | **Embedded Friendly** | ✅ Known size, no allocation | ❌ Unknown size, complex |
///
/// # Type Parameters
///
/// * `N` - The number of 32-bit words in the digest output
///
/// # Examples
///
/// ```rust
/// # use openprot_hal_blocking::digest::Digest;
/// // A 256-bit digest (8 words of 32 bits each)
/// let sha256_digest = Digest::<8> {
///     value: [0x12345678, 0x9abcdef0, 0x11111111, 0x22222222,
///             0x33333333, 0x44444444, 0x55555555, 0x66666666],
/// };
///
/// // Safe conversion to array for IPC
/// let array = sha256_digest.into_array();
///
/// // Access as bytes for serialization  
/// let bytes = sha256_digest.as_bytes();
/// assert_eq!(bytes.len(), 32);
/// ```
#[derive(Copy, Clone, PartialEq, Eq, IntoBytes, FromBytes, Immutable)]
#[repr(C)]
pub struct Digest<const N: usize> {
    /// The digest value as an array of 32-bit words
    pub value: [u32; N],
}

impl<const N: usize> Digest<N> {
    /// Create a new digest from an array of words
    pub fn new(value: [u32; N]) -> Self {
        Self { value }
    }

    /// Get the digest as an array of words
    ///
    /// This provides safe access to the underlying array without any conversions.
    pub fn into_array(self) -> [u32; N] {
        self.value
    }

    /// Get a reference to the digest as an array of words
    pub fn as_array(&self) -> &[u32; N] {
        &self.value
    }

    /// Get the digest as a byte slice
    pub fn as_bytes(&self) -> &[u8] {
        zerocopy::IntoBytes::as_bytes(self)
    }
}

impl<const N: usize> AsRef<[u8]> for Digest<N> {
    fn as_ref(&self) -> &[u8] {
        zerocopy::IntoBytes::as_bytes(self)
    }
}

/// Trait defining the properties of a cryptographic digest algorithm.
///
/// This trait provides compile-time information about digest algorithms,
/// including their output size and associated digest type. It serves as
/// a type-level specification that can be used with generic digest operations.
///
/// # Requirements
///
/// Implementing types must be `Copy` and `Debug` to support easy cloning
/// and debugging capabilities.
///
/// # Examples
///
/// ```rust
/// # use openprot_hal_blocking::digest::{DigestAlgorithm, Digest};
/// # use core::fmt::Debug;
/// #[derive(Clone, Copy, Debug)]
/// struct MyCustomAlgorithm;
///
/// impl DigestAlgorithm for MyCustomAlgorithm {
///     const OUTPUT_BITS: usize = 256;
///     type Digest = Digest<8>; // 256 bits / 32 bits per word = 8 words
/// }
/// ```
pub trait DigestAlgorithm: Copy + Debug {
    /// The output size of the digest algorithm in bits.
    ///
    /// This constant defines the total number of bits in the digest output.
    /// For example, SHA-256 would have `OUTPUT_BITS = 256`.
    const OUTPUT_BITS: usize;

    /// The digest output type for this algorithm.
    ///
    /// This associated type specifies the concrete digest type that will be
    /// produced by this algorithm. Typically this will be a [`Digest<N>`]
    /// where `N` is calculated from `OUTPUT_BITS`.
    type Digest;
}

/// SHA-256 digest algorithm marker type.
///
/// This zero-sized type represents the SHA-256 cryptographic hash algorithm,
/// which produces a 256-bit (32-byte) digest output.
///
/// SHA-256 is part of the SHA-2 family and is widely used for cryptographic
/// applications requiring strong collision resistance.
#[derive(Clone, Copy, Debug)]
pub struct Sha2_256;
impl DigestAlgorithm for Sha2_256 {
    const OUTPUT_BITS: usize = 256usize;
    type Digest = Digest<{ Self::OUTPUT_BITS / 32 }>;
}

/// SHA-384 digest algorithm marker type.
///
/// This zero-sized type represents the SHA-384 cryptographic hash algorithm,
/// which produces a 384-bit (48-byte) digest output.
///
/// SHA-384 is part of the SHA-2 family and provides a larger output size
/// than SHA-256 for applications requiring additional security margin.
#[derive(Clone, Copy, Debug)]
pub struct Sha2_384;
impl DigestAlgorithm for Sha2_384 {
    const OUTPUT_BITS: usize = 384usize;
    type Digest = Digest<{ Self::OUTPUT_BITS / 32 }>;
}

/// SHA-512 digest algorithm marker type.
///
/// This zero-sized type represents the SHA-512 cryptographic hash algorithm,
/// which produces a 512-bit (64-byte) digest output.
///
/// SHA-512 is part of the SHA-2 family and provides the largest standard
/// output size, offering maximum collision resistance.
#[derive(Clone, Copy, Debug)]
pub struct Sha2_512;
impl DigestAlgorithm for Sha2_512 {
    const OUTPUT_BITS: usize = 512;
    type Digest = Digest<{ Self::OUTPUT_BITS / 32 }>;
}

/// SHA3-224 digest algorithm marker type.
///
/// This zero-sized type represents the SHA3-224 cryptographic hash algorithm,
/// which produces a 224-bit (28-byte) digest output.
///
/// SHA3-224 is part of the SHA-3 (Keccak) family and offers an alternative
/// to SHA-2 with different underlying mathematical foundations.
#[derive(Clone, Copy, Debug)]
pub struct Sha3_224;
impl DigestAlgorithm for Sha3_224 {
    const OUTPUT_BITS: usize = 224usize;
    type Digest = Digest<{ Self::OUTPUT_BITS / 32 }>;
}

/// SHA3-256 digest algorithm marker type.
///
/// This zero-sized type represents the SHA3-256 cryptographic hash algorithm,
/// which produces a 256-bit (32-byte) digest output.
///
/// SHA3-256 is part of the SHA-3 (Keccak) family and provides equivalent
/// security to SHA-256 with different algorithmic properties.
#[derive(Clone, Copy, Debug)]
pub struct Sha3_256;
impl DigestAlgorithm for Sha3_256 {
    const OUTPUT_BITS: usize = 256usize;
    type Digest = Digest<{ Self::OUTPUT_BITS / 32 }>;
}

/// SHA3-384 digest algorithm marker type.
///
/// This zero-sized type represents the SHA3-384 cryptographic hash algorithm,
/// which produces a 384-bit (48-byte) digest output.
///
/// SHA3-384 is part of the SHA-3 (Keccak) family and provides equivalent
/// security to SHA-384 with different algorithmic properties.
#[derive(Clone, Copy, Debug)]
pub struct Sha3_384;
impl DigestAlgorithm for Sha3_384 {
    const OUTPUT_BITS: usize = 384usize;
    type Digest = Digest<{ Self::OUTPUT_BITS / 32 }>;
}

/// SHA3-512 digest algorithm marker type.
///
/// This zero-sized type represents the SHA3-512 cryptographic hash algorithm,
/// which produces a 512-bit (64-byte) digest output.
///
/// SHA3-512 is part of the SHA-3 (Keccak) family and provides equivalent
/// security to SHA-512 with different algorithmic properties.
#[derive(Clone, Copy, Debug)]
pub struct Sha3_512;
impl DigestAlgorithm for Sha3_512 {
    const OUTPUT_BITS: usize = 512;
    type Digest = Digest<{ Self::OUTPUT_BITS / 32 }>;
}

/// Error kind.
///
/// This represents a common set of digest operation errors. Implementations are
/// free to define more specific or additional error types. However, by providing
/// a mapping to these common errors, generic code can still react to them.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The input data length is not valid for the hash function.
    InvalidInputLength,

    /// The specified hash algorithm is not supported by the hardware or software implementation.
    UnsupportedAlgorithm,

    /// Failed to allocate memory for the hash computation.
    MemoryAllocationFailure,

    /// Failed to initialize the hash computation context.
    InitializationError,

    /// Error occurred while updating the hash computation with new data.
    UpdateError,

    /// Error occurred while finalizing the hash computation.
    FinalizationError,

    /// The hardware accelerator is busy and cannot process the hash computation.
    Busy,

    /// General hardware failure during hash computation.
    HardwareFailure,

    /// The specified output size is not valid for the hash function.
    InvalidOutputSize,

    /// Insufficient permissions to access the hardware or perform the hash computation.
    PermissionDenied,

    /// The hash computation context has not been initialized.
    NotInitialized,
}

impl core::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidInputLength => write!(f, "invalid input data length"),
            Self::UnsupportedAlgorithm => write!(f, "unsupported hash algorithm"),
            Self::MemoryAllocationFailure => write!(f, "memory allocation failed"),
            Self::InitializationError => write!(f, "failed to initialize hash computation"),
            Self::UpdateError => write!(f, "error updating hash computation"),
            Self::FinalizationError => write!(f, "error finalizing hash computation"),
            Self::Busy => write!(f, "hardware accelerator is busy"),
            Self::HardwareFailure => write!(f, "hardware failure during hash computation"),
            Self::InvalidOutputSize => write!(f, "invalid output size for hash function"),
            Self::PermissionDenied => write!(f, "insufficient permissions to access hardware"),
            Self::NotInitialized => write!(f, "hash computation context not initialized"),
        }
    }
}

/// Trait for digest operation errors.
///
/// This trait provides a common interface for all error types that can occur
/// during digest operations. It allows for categorization of errors into
/// common types while still permitting implementation-specific error details.
///
/// All digest error types must implement `Debug` for debugging purposes and
/// provide a method to convert to a generic [`ErrorKind`].
///
/// # Examples
///
/// ```rust
/// # use openprot_hal_blocking::digest::{Error, ErrorKind};
/// # use core::fmt::Debug;
/// #[derive(Debug)]
/// struct MyDigestError {
///     message: &'static str,
/// }
///
/// impl Error for MyDigestError {
///     fn kind(&self) -> ErrorKind {
///         ErrorKind::HardwareFailure
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

/// Trait for types that have an associated error type.
///
/// This trait provides a standard way for digest-related types to specify
/// their error type. It's used throughout the digest HAL to maintain
/// type safety while allowing different implementations to use their own
/// specific error types.
///
/// # Examples
///
/// ```rust
/// # use openprot_hal_blocking::digest::{ErrorType, Error, ErrorKind};
/// # use core::fmt::Debug;
/// # #[derive(Debug)]
/// # struct MyError;
/// # impl Error for MyError {
/// #     fn kind(&self) -> ErrorKind { ErrorKind::HardwareFailure }
/// # }
/// struct MyDigestDevice;
///
/// impl ErrorType for MyDigestDevice {
///     type Error = MyError;
/// }
/// ```
pub trait ErrorType {
    /// Error type.
    type Error: Error;
}

/// Trait for initializing digest operations.
///
/// This trait provides the interface for creating new digest computation contexts.
/// It is parameterized by a [`DigestAlgorithm`] type to ensure type safety and
/// allow different algorithms to have different initialization parameters.
///
/// # Type Parameters
///
/// * `T` - The digest algorithm type that implements [`DigestAlgorithm`]
///
/// # Examples
///
/// ```rust,no_run
/// # use openprot_hal_blocking::digest::*;
/// # struct MyDigestImpl;
/// # impl ErrorType for MyDigestImpl { type Error = core::convert::Infallible; }
/// # impl DigestInit<Sha2_256> for MyDigestImpl {
/// #     type OpContext<'a> = MyContext<'a> where Self: 'a;
/// #     type Output = Digest<8>;
/// #     fn init<'a>(&'a mut self, _: Sha2_256) -> Result<Self::OpContext<'a>, Self::Error> { todo!() }
/// # }
/// # struct MyContext<'a>(&'a mut MyDigestImpl);
/// # impl ErrorType for MyContext<'_> { type Error = core::convert::Infallible; }
/// # impl DigestContext for MyContext<'_> {
/// #     type Output = Digest<8>;
/// #     fn update(&mut self, _: &[u8]) -> Result<(), Self::Error> { Ok(()) }
/// #     fn finalize(self) -> Result<Self::Output, Self::Error> {
/// #         Ok(Digest { value: [0u32; 8] })
/// #     }
/// # }
/// let mut device = MyDigestImpl;
/// let context = device.init(Sha2_256)?;
/// # Ok::<(), core::convert::Infallible>(())
/// ```
pub trait DigestInit<T: DigestAlgorithm>: ErrorType {
    /// The operation context type that will handle the digest computation.
    ///
    /// This associated type represents the stateful context returned by [`init`](Self::init)
    /// that can be used to perform the actual digest operations via [`DigestContext`].
    /// The lifetime parameter ensures the context cannot outlive the device that created it.
    type OpContext<'a>: DigestContext<Output = Self::Output>
    where
        Self: 'a;

    /// The output type produced by this digest implementation.
    ///
    /// This type must implement [`IntoBytes`] to allow conversion to byte arrays
    /// for interoperability with other systems and zero-copy operations.
    type Output: IntoBytes;

    /// Init instance of the crypto function with the given context.
    ///
    /// # Parameters
    ///
    /// - `init_params`: The context or configuration parameters for the crypto function.
    ///
    /// # Returns
    ///
    /// A new instance of the hash function.
    fn init(&mut self, init_params: T) -> Result<Self::OpContext<'_>, Self::Error>;
}

/// Trait for resetting digest computation contexts.
///
/// This trait provides the ability to reset a digest device or context back to
/// its initial state, allowing it to be reused for new digest computations
/// without needing to create a new instance.
///
/// # Examples
///
/// ```rust,no_run
/// # use openprot_hal_blocking::digest::*;
/// # struct MyDigestImpl;
/// # impl ErrorType for MyDigestImpl { type Error = core::convert::Infallible; }
/// # impl DigestCtrlReset for MyDigestImpl {
/// #     fn reset(&mut self) -> Result<(), Self::Error> { Ok(()) }
/// # }
/// let mut device = MyDigestImpl;
/// device.reset()?;
/// # Ok::<(), core::convert::Infallible>(())
/// ```
pub trait DigestCtrlReset: ErrorType {
    /// Reset instance to its initial state.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure. On success, returns `Ok(())`. On failure, returns a `CryptoError`.
    fn reset(&mut self) -> Result<(), Self::Error>;
}

/// Trait for performing digest operations.
///
/// This trait provides the core interface for digest computation operations:
/// updating the digest state with input data and finalizing the computation
/// to produce the digest output.
///
/// This trait is typically implemented by context types returned from
/// [`DigestInit::init`] and represents an active digest computation.
///
/// # State Machine
///
/// Digest operations follow a simple state machine:
/// 1. **Update**: Call [`update`](Self::update) zero or more times with input data
/// 2. **Finalize**: Call [`finalize`](Self::finalize) once to produce the final digest
///
/// After finalization, the context is consumed and cannot be reused.
///
/// # Examples
///
/// ```rust,no_run
/// # use openprot_hal_blocking::digest::*;
/// # struct MyContext;
/// # impl ErrorType for MyContext { type Error = core::convert::Infallible; }
/// # impl DigestContext for MyContext {
/// #     type Output = Digest<8>;
/// #     fn update(&mut self, _: &[u8]) -> Result<(), Self::Error> { Ok(()) }
/// #     fn finalize(self) -> Result<Self::Output, Self::Error> {
/// #         Ok(Digest { value: [0u32; 8] })
/// #     }
/// # }
/// let mut context = MyContext;
/// context.update(b"hello")?;
/// context.update(b" world")?;
/// let digest = context.finalize()?;
/// # Ok::<(), core::convert::Infallible>(())
/// ```
pub trait DigestContext: ErrorType {
    /// The digest output type.
    ///
    /// This type represents the final digest value produced by [`finalize`](Self::finalize).
    /// It must implement [`IntoBytes`] to enable zero-copy conversion to byte arrays.
    type Output: IntoBytes;

    /// Update state using provided input data.
    ///
    /// # Parameters
    ///
    /// - `input`: The input data to be hashed. This can be any type that implements `AsRef<[u8]>`.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure. On success, returns `Ok(())`. On failure, returns a `CryptoError`.
    fn update(&mut self, input: &[u8]) -> Result<(), Self::Error>;

    /// Finalize the computation and produce the output.
    ///
    /// # Parameters
    ///
    /// - `out`: A mutable slice to store the hash output. The length of the slice must be at least `MAX_OUTPUT_SIZE`.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure. On success, returns `Ok(())`. On failure, returns a `CryptoError`.
    fn finalize(self) -> Result<Self::Output, Self::Error>;
}

pub mod scoped {
    //! Scoped digest API with borrowed contexts (current)
    //!
    //! This module contains the original OpenPRoT HAL digest traits that use
    //! borrowed contexts with lifetime constraints. These traits are ideal for:
    //! - One-shot digest operations
    //! - Simple embedded applications  
    //! - Direct hardware mapping
    //! - Minimal memory overhead
    //!
    //! **Limitation**: Contexts cannot be stored or persist across function boundaries
    //! due to lifetime constraints.

    pub use super::{
        DigestAlgorithm, DigestContext, DigestCtrlReset, DigestInit, Error, ErrorKind, ErrorType,
    };
}

pub mod owned {
    //! Owned digest API with move-based resource management
    //!
    //! This module provides a move-based digest API built on top of
    //! [`streaming::Owned`](super::streaming::Owned). It enables:
    //! - Persistent session storage
    //! - Multiple concurrent contexts
    //! - IPC boundary crossing
    //! - Resource recovery patterns
    //! - Compile-time prevention of use-after-finalize
    //!
    //! # Relationship to streaming module
    //!
    //! The `owned` API is a thin layer over `streaming::Owned<T, C>`:
    //! - `streaming::DigestContext` provides the core `&mut self` operations
    //! - `streaming::Owned<T, C>` wraps context + controller with move semantics
    //! - `owned::DigestInit` provides the factory pattern for creating owned contexts
    //!
    //! This layered design means implementations only need to implement
    //! `streaming::DigestContext` and get owned semantics for free.
    //!
    //! ```text
    //! ┌────────────────────────────────────────────────────────────┐
    //! │  owned::DigestInit<A>                                      │
    //! │  - Factory trait returning Owned<Context, Controller>      │
    //! │  - Re-exports streaming::Owned                             │
    //! └────────────────────────────┬───────────────────────────────┘
    //!                              │ builds on
    //! ┌────────────────────────────▼───────────────────────────────┐
    //! │  streaming::Owned<T, C>                                    │
    //! │  - Move-based wrapper: update(self), finalize(self), etc.  │
    //! │  - Controller recovery on finalize/cancel                  │
    //! └────────────────────────────┬───────────────────────────────┘
    //!                              │ wraps
    //! ┌────────────────────────────▼───────────────────────────────┐
    //! │  streaming::DigestContext                                  │
    //! │  - Core trait: &mut self methods                           │
    //! │  - Implement once, get owned semantics free                │
    //! └────────────────────────────────────────────────────────────┘
    //! ```
    //!
    //! # Examples
    //!
    //! ```rust,ignore
    //! use openprot_hal_blocking::digest::owned::DigestInit;
    //! use openprot_hal_blocking::digest::Sha2_256;
    //!
    //! let controller = MyController::new();
    //! let owned_ctx = controller.init(Sha2_256)?;
    //!
    //! // Move-based API
    //! let owned_ctx = owned_ctx.update(b"hello")?;
    //! let owned_ctx = owned_ctx.update(b" world")?;
    //!
    //! // Finalize and recover controller
    //! let mut output = [0u8; 32];
    //! let (len, controller) = owned_ctx.finalize(&mut output)?;
    //! ```

    use super::streaming;
    use super::DigestAlgorithm;
    use core::result::Result;

    // Re-export error traits for convenience
    pub use super::{Error, ErrorKind, ErrorType};

    // Re-export Owned wrapper from streaming module
    pub use super::streaming::Owned;

    /// Trait for initializing digest operations with owned contexts.
    ///
    /// This trait consumes the controller and returns an [`Owned`] wrapper
    /// that provides move-based resource management. The controller is
    /// recovered when the context is finalized or cancelled.
    ///
    /// # Type Parameters
    ///
    /// * `A` - The digest algorithm type that implements [`DigestAlgorithm`]
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use openprot_hal_blocking::digest::owned::DigestInit;
    /// use openprot_hal_blocking::digest::Sha2_256;
    ///
    /// let controller = MyController::new();
    /// let owned_ctx = controller.init(Sha2_256)?;
    ///
    /// let owned_ctx = owned_ctx.update(b"hello")?;
    /// let mut output = [0u8; 32];
    /// let (len, controller) = owned_ctx.finalize(&mut output)?;
    /// ```
    pub trait DigestInit<A: DigestAlgorithm>: ErrorType + Sized {
        /// The underlying streaming context type.
        ///
        /// Must implement [`streaming::DigestContext`] to provide the
        /// core digest operations.
        type Context: streaming::DigestContext<Error = Self::Error>;

        /// Initialize a new owned digest context.
        ///
        /// Consumes the controller and returns an `Owned<Context, Self>`
        /// wrapper that provides move-based operations with automatic
        /// controller recovery.
        ///
        /// # Parameters
        ///
        /// - `algo`: Algorithm-specific initialization parameters
        ///
        /// # Returns
        ///
        /// An `Owned` wrapper combining the context and controller.
        fn init(self, algo: A) -> Result<Owned<Self::Context, Self>, Self::Error>;
    }
}

/// Core digest traits for streaming operations.
///
/// This module provides the **streaming** digest API where the core operation
/// semantics (`update`, `finalize`) are separated from resource lifecycle management.
///
/// # Design Philosophy
///
/// The traditional `owned::DigestContext` bundles two concerns:
/// 1. **Operation semantics** — what the API does (update/finalize)
/// 2. **Resource lifecycle** — typestate enforcement for controller recovery
///
/// This coupling creates friction:
/// - Software backends have no resources to recover — the pattern is overhead
/// - Servers manage sessions via handles — typestate is redundant
/// - Wrapping HAL controllers requires adapter layers
///
/// The streaming design separates these:
///
/// ```text
/// ┌─────────────────────────────────────────────────────────────────┐
/// │  streaming::DigestContext (&mut self)                           │
/// │  - Pure operation semantics                                     │
/// │  - Implements: update, finalize, reset                          │
/// │  - Used by: servers, software backends                          │
/// └────────────────────────┬────────────────────────────────────────┘
///                          │
///              ┌───────────▼───────────┐
///              │  Owned<T> wrapper     │
///              │  - Typestate pattern  │
///              │  - Resource recovery  │
///              │  - Used by: baremetal │
///              └───────────────────────┘
/// ```
///
/// # Usage
///
/// ## Direct use (servers, software backends)
///
/// ```rust,ignore
/// let mut ctx = controller.begin::<Sha256>()?;
/// ctx.update(b"hello")?;
/// ctx.update(b" world")?;
/// let len = ctx.finalize(&mut output)?;
/// ctx.reset();  // Reuse without recovering controller
/// ```
///
/// ## With typestate wrapper (baremetal safety)
///
/// ```rust,ignore
/// let owned_ctx = Owned::new(controller.begin::<Sha256>()?, controller);
/// let owned_ctx = owned_ctx.update(b"hello")?;
/// let (output, controller) = owned_ctx.finalize()?;  // Controller recovered
/// ```
pub mod streaming {
    use super::DigestAlgorithm;

    // Re-export error traits for convenience
    pub use super::{Error, ErrorKind, ErrorType};

    /// Core digest context trait with `&mut self` methods.
    ///
    /// This trait defines the minimal operation semantics for streaming digest
    /// computations without bundling resource lifecycle concerns.
    ///
    /// # Design
    ///
    /// - Uses `ErrorType` supertrait for consistent error handling
    /// - `&mut self` allows reuse without ownership transfer
    /// - `reset()` enables context reuse without controller recovery
    /// - No associated `Controller` type — separated from ownership pattern
    ///
    /// # Error Handling
    ///
    /// Following the HAL error pattern, `DigestContext` extends `ErrorType`
    /// which provides `Self::Error`. The error type must implement `Error`
    /// trait with `fn kind(&self) -> ErrorKind` for generic error handling.
    ///
    /// # Implementors
    ///
    /// - Hardware digest controllers (HACE, etc.)
    /// - Software backends (RustCrypto wrappers)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use openprot_hal_blocking::digest::{Error, ErrorKind, ErrorType};
    /// use openprot_hal_blocking::digest::streaming::DigestContext;
    ///
    /// #[derive(Debug)]
    /// struct MyError(ErrorKind);
    ///
    /// impl Error for MyError {
    ///     fn kind(&self) -> ErrorKind { self.0 }
    /// }
    ///
    /// struct MyDigestContext { /* internal state */ }
    ///
    /// impl ErrorType for MyDigestContext {
    ///     type Error = MyError;
    /// }
    ///
    /// impl DigestContext for MyDigestContext {
    ///     const OUTPUT_SIZE: usize = 32;
    ///
    ///     fn update(&mut self, data: &[u8]) -> Result<(), Self::Error> {
    ///         // Process data into internal state
    ///         Ok(())
    ///     }
    ///
    ///     fn finalize(&mut self, output: &mut [u8]) -> Result<usize, Self::Error> {
    ///         // Write final digest, return bytes written
    ///         Ok(32)
    ///     }
    ///
    ///     fn reset(&mut self) {
    ///         // Reset internal state for reuse
    ///     }
    /// }
    /// ```
    pub trait DigestContext: ErrorType {
        /// Size of the digest output in bytes.
        const OUTPUT_SIZE: usize;

        /// Update the digest state with input data.
        ///
        /// Can be called multiple times to process data incrementally.
        fn update(&mut self, data: &[u8]) -> Result<(), Self::Error>;

        /// Finalize the digest and write the result to `output`.
        ///
        /// # Parameters
        ///
        /// - `output`: Buffer to receive the digest. Must be at least `OUTPUT_SIZE` bytes.
        ///
        /// # Returns
        ///
        /// Number of bytes written to `output`.
        ///
        /// # Note
        ///
        /// After finalization, the context is in an undefined state.
        /// Call `reset()` before reusing.
        fn finalize(&mut self, output: &mut [u8]) -> Result<usize, Self::Error>;

        /// Reset the context to its initial state for reuse.
        ///
        /// This allows the same context to perform multiple digest operations
        /// without needing to recover and re-initialize a controller.
        fn reset(&mut self);
    }

    /// Factory trait for creating digest contexts.
    ///
    /// Separates context creation from context operations, enabling
    /// flexible initialization patterns without typestate.
    pub trait DigestContextFactory<A: DigestAlgorithm>: ErrorType {
        /// The context type produced by this factory.
        type Context: DigestContext<Error = Self::Error>;

        /// Create a new digest context for the specified algorithm.
        fn create_context(&mut self, algo: A) -> Result<Self::Context, Self::Error>;
    }

    /// Optional wrapper providing typestate-based resource recovery.
    ///
    /// Wraps a [`DigestContext`] with move semantics that return the
    /// underlying controller on finalization or cancellation.
    ///
    /// # Use Case
    ///
    /// Baremetal applications where compile-time enforcement of
    /// controller recovery is essential for correctness.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Create owned context with controller
    /// let owned = Owned::new(ctx, controller);
    ///
    /// // Move-based API — each call consumes and returns
    /// let owned = owned.update(b"hello")?;
    /// let owned = owned.update(b" world")?;
    ///
    /// // Finalize recovers the controller
    /// let (digest, controller) = owned.finalize()?;
    ///
    /// // Controller can be reused
    /// let new_ctx = controller.create_context(Sha256)?;
    /// ```
    pub struct Owned<T, C> {
        context: T,
        controller: C,
    }

    impl<T, C> Owned<T, C> {
        /// Create a new owned context wrapping a digest context and its controller.
        pub fn new(context: T, controller: C) -> Self {
            Self { context, controller }
        }

        /// Get a reference to the underlying context.
        pub fn context(&self) -> &T {
            &self.context
        }

        /// Get a mutable reference to the underlying context.
        pub fn context_mut(&mut self) -> &mut T {
            &mut self.context
        }
    }

    impl<T: DigestContext, C> Owned<T, C> {
        /// Update the digest state with input data.
        ///
        /// Consumes self and returns a new `Owned` with updated state.
        pub fn update(mut self, data: &[u8]) -> Result<Self, (C, T::Error)> {
            match self.context.update(data) {
                Ok(()) => Ok(self),
                Err(e) => Err((self.controller, e)),
            }
        }

        /// Finalize the digest and recover the controller.
        ///
        /// Returns the digest output and the recovered controller.
        ///
        /// # Parameters
        ///
        /// - `output`: Buffer to receive the digest.
        ///
        /// # Returns
        ///
        /// On success: `(bytes_written, controller)`
        /// On error: `(controller, error)` — controller is still recovered
        pub fn finalize(mut self, output: &mut [u8]) -> Result<(usize, C), (C, T::Error)> {
            match self.context.finalize(output) {
                Ok(len) => Ok((len, self.controller)),
                Err(e) => Err((self.controller, e)),
            }
        }

        /// Cancel the operation and recover the controller.
        ///
        /// Resets the context and returns the controller.
        pub fn cancel(mut self) -> C {
            self.context.reset();
            self.controller
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digest_output_conversions() {
        // Test safe conversion methods on Digest type
        let sha256_digest = Digest::<8> {
            value: [1, 2, 3, 4, 5, 6, 7, 8],
        };

        // Test into_array() method
        let array = sha256_digest.into_array();
        assert_eq!(array, [1, 2, 3, 4, 5, 6, 7, 8]);

        // Test as_array() method
        let sha256_digest = Digest::<8> {
            value: [1, 2, 3, 4, 5, 6, 7, 8],
        };
        let array_ref = sha256_digest.as_array();
        assert_eq!(array_ref, &[1, 2, 3, 4, 5, 6, 7, 8]);

        // Test as_bytes() method
        let bytes = sha256_digest.as_bytes();
        assert_eq!(bytes.len(), 32); // 8 words * 4 bytes each

        // Verify the bytes match the expected layout (little endian)
        let expected_bytes = [
            1, 0, 0, 0, // word 1
            2, 0, 0, 0, // word 2
            3, 0, 0, 0, // word 3
            4, 0, 0, 0, // word 4
            5, 0, 0, 0, // word 5
            6, 0, 0, 0, // word 6
            7, 0, 0, 0, // word 7
            8, 0, 0, 0, // word 8
        ];
        assert_eq!(bytes, &expected_bytes);
    }

    #[test]
    fn test_output_type_sizes() {
        use core::mem;

        // Verify that digest output types have correct sizes for IPC
        assert_eq!(mem::size_of::<Digest<8>>(), 32); // SHA-256: 8 words * 4 bytes
        assert_eq!(mem::size_of::<Digest<12>>(), 48); // SHA-384: 12 words * 4 bytes
        assert_eq!(mem::size_of::<Digest<16>>(), 64); // SHA-512: 16 words * 4 bytes

        // Test alignment requirements
        assert_eq!(mem::align_of::<Digest<8>>(), 4); // Aligned to u32
    }

    #[test]
    fn test_digest_new_constructor() {
        let array = [
            0x12345678, 0x9abcdef0, 0x11111111, 0x22222222, 0x33333333, 0x44444444, 0x55555555,
            0x66666666,
        ];
        let digest = Digest::new(array);
        assert_eq!(digest.value, array);
        assert_eq!(digest.into_array(), array);
    }

    // Tests for the streaming module
    mod streaming_tests {
        use super::super::streaming::{DigestContext, Owned};
        use super::super::{Error, ErrorKind, ErrorType};

        /// Mock error type implementing the Error trait
        #[derive(Debug)]
        struct MockError(#[allow(dead_code)] &'static str);

        impl Error for MockError {
            fn kind(&self) -> ErrorKind {
                ErrorKind::UpdateError
            }
        }

        /// Mock digest context for testing (using fixed buffer for no_std)
        struct MockDigestContext {
            data: [u8; 64],
            len: usize,
            finalized: bool,
        }

        impl MockDigestContext {
            fn new() -> Self {
                Self {
                    data: [0u8; 64],
                    len: 0,
                    finalized: false,
                }
            }
        }

        impl ErrorType for MockDigestContext {
            type Error = MockError;
        }

        impl DigestContext for MockDigestContext {
            const OUTPUT_SIZE: usize = 4;

            fn update(&mut self, data: &[u8]) -> Result<(), Self::Error> {
                if self.finalized {
                    return Err(MockError("already finalized"));
                }
                for &b in data {
                    if self.len < 64 {
                        self.data[self.len] = b;
                        self.len += 1;
                    }
                }
                Ok(())
            }

            fn finalize(&mut self, output: &mut [u8]) -> Result<usize, Self::Error> {
                if output.len() < Self::OUTPUT_SIZE {
                    return Err(MockError("buffer too small"));
                }
                // Simple "hash": sum of bytes mod 256 repeated
                let sum: u8 = self.data[..self.len].iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
                output[..4].fill(sum);
                self.finalized = true;
                Ok(4)
            }

            fn reset(&mut self) {
                self.data = [0u8; 64];
                self.len = 0;
                self.finalized = false;
            }
        }

        /// Mock controller for testing resource recovery
        #[derive(Debug, PartialEq)]
        struct MockController {
            id: u32,
        }

        #[test]
        fn test_core_digest_context_direct_use() {
            let mut ctx = MockDigestContext::new();
            ctx.update(b"hello").unwrap();
            ctx.update(b" world").unwrap();

            let mut output = [0u8; 4];
            let len = ctx.finalize(&mut output).unwrap();
            assert_eq!(len, 4);

            // Reuse via reset
            ctx.reset();
            ctx.update(b"test").unwrap();
            let len = ctx.finalize(&mut output).unwrap();
            assert_eq!(len, 4);
        }

        #[test]
        fn test_owned_wrapper_update_finalize() {
            let ctx = MockDigestContext::new();
            let controller = MockController { id: 42 };

            let owned = Owned::new(ctx, controller);
            let owned = owned.update(b"hello").unwrap();
            let owned = owned.update(b" world").unwrap();

            let mut output = [0u8; 4];
            let (len, recovered) = owned.finalize(&mut output).unwrap();

            assert_eq!(len, 4);
            assert_eq!(recovered.id, 42); // Controller recovered
        }

        #[test]
        fn test_owned_wrapper_cancel() {
            let ctx = MockDigestContext::new();
            let controller = MockController { id: 99 };

            let owned = Owned::new(ctx, controller);
            let owned = owned.update(b"partial data").unwrap();
            let recovered = owned.cancel();

            assert_eq!(recovered.id, 99); // Controller recovered
        }

        #[test]
        fn test_owned_wrapper_error_recovers_controller() {
            let ctx = MockDigestContext::new();
            let controller = MockController { id: 123 };

            let owned = Owned::new(ctx, controller);
            let owned = owned.update(b"data").unwrap();

            // Finalize with buffer too small — should still recover controller
            let mut output = [0u8; 2]; // Too small!
            let result = owned.finalize(&mut output);

            match result {
                Err((recovered, _err)) => {
                    assert_eq!(recovered.id, 123); // Controller recovered despite error
                }
                Ok(_) => panic!("expected error"),
            }
        }
    }
}
