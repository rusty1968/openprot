// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! ECDSA device-layer constants (poll budget + advisory yield window).

/// Default completion-poll budget. Mirrors the HACE device layer's generous
/// default; the exhaustion bound is in *poll iterations*, not wall-clock time.
/// Choosing this value is a documented tuning obligation (see the
/// `cooperative-yield-bounded-poll-device` catalog entry): too low → spurious
/// timeouts on slow silicon; too high → long stalls before a wedged engine is
/// detected.
pub const DEFAULT_POLL_BUDGET: u32 = 1_000_000;

/// Suggested wait window, in nanoseconds, passed to the cooperative `yield_fn`
/// between completion polls. Mirrors the reference ECDSA driver's 5 µs poll
/// interval (`aspeed-rust/src/ecdsa.rs`, `self.delay.delay_ns(5000)`). Advisory
/// only: the injected strategy decides whether/how to honor it (`spin_loop`
/// ignores it; an async/RTOS strategy may sleep for it).
pub const POLL_YIELD_NS: u32 = 5_000;
