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
//! - [`Digest`] - A generic container for digest output values
//! - [`DigestAlgorithm`] - Trait defining digest algorithm properties
//!
//! ### Scoped API
//! - [`scoped::DigestInit`] - Trait for initializing digest operations (borrowed contexts)
//! - [`scoped::DigestOp`] - Trait for performing digest computations (borrowed contexts)
//! - [`scoped::DigestCtrlReset`] - Trait for resetting digest contexts
//!
//! ### Owned API (Typestate)
//! - [`owned::DigestInit`] - Trait for initializing digest operations (owned contexts)
//! - [`owned::DigestOp`] - Trait for performing digest computations (owned contexts)
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
//! # impl DigestOp for MyContext<'_> {
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
//! ```rust,no_run
//! # use openprot_hal_blocking::digest::*;
//! # use openprot_hal_blocking::digest::owned::{DigestInit, DigestOp};
//! # struct MyDigestController;
//! # impl ErrorType for MyDigestController { type Error = core::convert::Infallible; }
//! # impl DigestInit<Sha2_256> for MyDigestController {
//! #     type Context = MyOwnedContext;
//! #     type Output = Digest<8>;
//! #     fn init(self, _: Sha2_256) -> Result<Self::Context, Self::Error> { todo!() }
//! # }
//! # struct MyOwnedContext;
//! # impl ErrorType for MyOwnedContext { type Error = core::convert::Infallible; }
//! # impl DigestOp for MyOwnedContext {
//! #     type Output = Digest<8>;
//! #     type Controller = MyDigestController;
//! #     fn update(self, _: &[u8]) -> Result<Self, Self::Error> { todo!() }
//! #     fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> { todo!() }
//! #     fn cancel(self) -> Self::Controller { todo!() }
//! # }
//! let controller = MyDigestController;
//! let context = controller.init(Sha2_256)?;
//! let context = context.update(b"hello world")?;
//! let (digest, recovered_controller) = context.finalize()?;
//! // Controller can be reused for new operations
//! # Ok::<(), core::convert::Infallible>(())
//! ```

use core::fmt::Debug;
use core::result::Result;
use zerocopy::IntoBytes;

/// A generic digest output container.
///
/// This structure represents the output of a cryptographic digest operation.
/// It uses a const generic parameter `N` to specify the number of 32-bit words
/// in the digest output, allowing it to accommodate different digest sizes.
///
/// The structure is marked with `#[repr(C)]` to ensure a predictable memory layout,
/// making it suitable for zero-copy operations and hardware interfaces.
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
/// ```
#[derive(Copy, Clone, PartialEq, Eq, IntoBytes)]
#[repr(C)]
pub struct Digest<const N: usize> {
    /// The digest value as an array of 32-bit words
    pub value: [u32; N],
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
/// # impl DigestOp for MyContext<'_> {
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
    /// that can be used to perform the actual digest operations via [`DigestOp`].
    /// The lifetime parameter ensures the context cannot outlive the device that created it.
    type OpContext<'a>: DigestOp<Output = Self::Output>
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
/// # impl DigestOp for MyContext {
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
pub trait DigestOp: ErrorType {
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

    pub use super::{DigestAlgorithm, DigestCtrlReset, DigestInit, DigestOp, ErrorType};
}

pub mod owned {
    //! Owned digest API with move-based resource management
    //!
    //! This module provides a move-based digest API where contexts are owned
    //! rather than borrowed. This enables:
    //! - Persistent session storage
    //! - Multiple concurrent contexts
    //! - IPC boundary crossing
    //! - Resource recovery patterns
    //! - Compile-time prevention of use-after-finalize
    //!
    //! This API is specifically designed for server applications like Hubris
    //! digest servers that need to maintain long-lived sessions.

    use super::{DigestAlgorithm, ErrorType, IntoBytes};
    use core::result::Result;

    /// Trait for initializing digest operations with owned contexts.
    ///
    /// This trait takes ownership of the controller and returns an owned context
    /// that can be stored, moved, and persisted across function boundaries.
    /// Unlike the scoped API, there are no lifetime constraints.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The digest algorithm type that implements [`DigestAlgorithm`]
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use openprot_hal_blocking::digest::*;
    /// # use openprot_hal_blocking::digest::owned::{DigestInit, DigestOp};
    /// # struct MyController;
    /// # impl ErrorType for MyController { type Error = core::convert::Infallible; }
    /// # struct MyContext;
    /// # impl ErrorType for MyContext { type Error = core::convert::Infallible; }
    /// # impl DigestOp for MyContext {
    /// #     type Output = Digest<8>;
    /// #     type Controller = MyController;
    /// #     fn update(self, _: &[u8]) -> Result<Self, Self::Error> { Ok(self) }
    /// #     fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> { todo!() }
    /// #     fn cancel(self) -> Self::Controller { todo!() }
    /// # }
    /// # impl DigestInit<Sha2_256> for MyController {
    /// #     type Context = MyContext;
    /// #     type Output = Digest<8>;
    /// #     fn init(self, _: Sha2_256) -> Result<Self::Context, Self::Error> { todo!() }
    /// # }
    /// let controller = MyController;
    /// let context = controller.init(Sha2_256)?;
    /// // Context can be stored in structs, moved across functions, etc.
    /// # Ok::<(), core::convert::Infallible>(())
    /// ```
    pub trait DigestInit<T: DigestAlgorithm>: ErrorType + Sized {
        /// The owned context type that will handle the digest computation.
        ///
        /// This context has no lifetime constraints and can be stored in structs,
        /// moved between functions, and persisted across IPC boundaries.
        type Context: DigestOp<Output = Self::Output, Controller = Self>;

        /// The output type produced by this digest implementation.
        ///
        /// This type must implement [`IntoBytes`] to allow conversion to byte arrays
        /// for interoperability with other systems and zero-copy operations.
        type Output: IntoBytes;

        /// Initialize a new digest computation context.
        ///
        /// Takes ownership of the controller and returns an owned context.
        /// The controller will be returned when the context is finalized or cancelled.
        ///
        /// # Parameters
        ///
        /// - `init_params`: Algorithm-specific initialization parameters
        ///
        /// # Returns
        ///
        /// An owned context that can be used for digest operations.
        fn init(self, init_params: T) -> Result<Self::Context, Self::Error>;
    }

    /// Trait for performing digest operations with owned contexts.
    ///
    /// This trait uses move semantics where each operation consumes the
    /// context and returns a new context (for `update`) or the final result
    /// with a recovered controller (for `finalize`/`cancel`).
    ///
    /// # Move-based Safety
    ///
    /// The move-based pattern provides compile-time guarantees:
    /// - Cannot use a context after finalization
    /// - Cannot finalize the same context twice
    /// - Controller is always recovered for reuse
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use openprot_hal_blocking::digest::*;
    /// # use openprot_hal_blocking::digest::owned::{DigestInit, DigestOp};
    /// # struct MyContext;
    /// # impl ErrorType for MyContext { type Error = core::convert::Infallible; }
    /// # struct MyController;
    /// # impl DigestOp for MyContext {
    /// #     type Output = Digest<8>;
    /// #     type Controller = MyController;
    /// #     fn update(self, _: &[u8]) -> Result<Self, Self::Error> { Ok(self) }
    /// #     fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error> { todo!() }
    /// #     fn cancel(self) -> Self::Controller { todo!() }
    /// # }
    /// # fn get_context() -> MyContext { todo!() }
    /// let context = get_context(); // MyContext
    /// let context = context.update(b"hello")?;
    /// let context = context.update(b" world")?;
    /// let (digest, controller) = context.finalize()?;
    /// // Controller recovered for reuse
    /// # Ok::<(), core::convert::Infallible>(())
    /// ```
    pub trait DigestOp: ErrorType + Sized {
        /// The digest output type.
        ///
        /// This type represents the final digest value produced by [`finalize`](Self::finalize).
        /// It must implement [`IntoBytes`] to enable zero-copy conversion to byte arrays.
        type Output: IntoBytes;

        /// The controller type that will be recovered after finalization or cancellation.
        ///
        /// This enables resource recovery and reuse patterns essential for server applications.
        type Controller;

        /// Update the digest state with input data.
        ///
        /// This method consumes the current context and returns a new context with
        /// the updated state. This prevents use-after-update bugs at compile time
        /// through move semantics.
        ///
        /// # Parameters
        ///
        /// - `data`: Input data to be processed by the digest algorithm
        ///
        /// # Returns
        ///
        /// A new context with updated state, or an error
        fn update(self, data: &[u8]) -> Result<Self, Self::Error>;

        /// Finalize the digest computation and recover the controller.
        ///
        /// This method consumes the context and returns both the final digest output
        /// and the original controller, enabling resource reuse.
        ///
        /// # Returns
        ///
        /// A tuple containing the digest output and the recovered controller
        fn finalize(self) -> Result<(Self::Output, Self::Controller), Self::Error>;

        /// Cancel the digest computation and recover the controller.
        ///
        /// This method cancels the current computation and returns the controller
        /// in a clean state, ready for reuse. Unlike `finalize`, this cannot fail.
        ///
        /// # Returns
        ///
        /// The recovered controller in a clean state
        fn cancel(self) -> Self::Controller;
    }
}
