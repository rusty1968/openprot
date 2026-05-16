// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! ECDSA error definitions.

/// Errors surfaced by the ECDSA device layer.
///
/// Only the wait-policy failure (`Timeout`) is defined here: it is the typed,
/// bounded failure of the cooperative-yield poll seam. The verify-result and
/// input-validation variants land with the verify semantics under the
/// `peripheral-parity-port` workflow — hence `#[non_exhaustive]`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EcdsaError {
    /// Operation did not complete before the poll budget was exhausted.
    Timeout,
}
