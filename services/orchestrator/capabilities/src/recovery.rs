// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! The [`Recovery`] restore capability contract.

/// Restore capability: rewrite one managed device's active image from its
/// configured recovery source.
///
/// What the recovery source is (a golden image in protected flash, a mux
/// flip to a known-good part, a fetch over a sideband) is board wiring and
/// never leaks through this seam. The orchestrator only asks for the
/// mechanism to run; it holds the device in reset before asking and
/// re-verifies the image on the walk that follows.
///
/// # Contract
///
/// - **`Ok(Restored)` means the mechanism completed, not that the image is
///   good.** Judging the restored image belongs to the verifier on the
///   re-walk; a restore must not forge a verdict by checking it here.
/// - **`Ok(SourcesExhausted)` means this device has no source left for
///   `attempt`** — not a fault, a report. The core's retry cap and a board's
///   source count are independent; when the board runs out first, the
///   orchestrator gates the component by its failure policy
///   (`Isolable`/`Cascading` skip, `Required` locks) instead of assuming the
///   mechanism is broken.
/// - **`Err` is an actuation fault only** (source unreachable, write
///   failed). The orchestrator treats it fail-closed, unconditionally —
///   never route a merely-exhausted device through `Err`, or a
///   `Isolable`/`Cascading` component locks the whole platform down over
///   nothing worse than running out of images.
/// - **Repeatable.** Each recovery attempt calls `restore` again with the
///   next `attempt`; a partial earlier restore must not stop a later call
///   from producing a complete image.
/// - **The caller counts the attempts.** `attempt` comes from the core's own
///   retry counter, the same value its retry cap is measured against, so an
///   implementor that keeps a count of its own would drift: it never sees
///   which attempt succeeded.
pub trait Recovery {
    /// The error type of this device's restore mechanism.
    ///
    /// Bounded by [`core::error::Error`] so the orchestrator gets `Display`
    /// and a `source()` cause chain, not just a `Debug` dump. Error
    /// categories are implementation-defined. Reserved for actuation
    /// faults — see [`RestoreOutcome::SourcesExhausted`] for running out of
    /// sources.
    type Error: core::error::Error;

    /// Rewrites the device's active image from the recovery source.
    ///
    /// `attempt` is this device's consecutive-recovery count, `0` on the
    /// first try of a recovery cycle. Implementors that hold more
    /// than one source pick per attempt (slot A on `0`, slot B on `1`,
    /// golden on `2`); implementors with a single source ignore it and
    /// always answer `Restored`, letting the core's own retry cap be the
    /// only limit.
    fn restore(&mut self, attempt: u8) -> Result<RestoreOutcome, Self::Error>;
}

/// The verdict of one [`Recovery::restore`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The mechanism completed; the image is unjudged until the re-walk.
    Restored,
    /// No source left for this `attempt`. Not a fault: the orchestrator
    /// gates the component by its failure policy rather than fail closed.
    SourcesExhausted,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Implements Recovery with no HAL dependency — the contract must be
    // satisfiable from any stack (mock, IPC proxy, simulator). A HAL-bound
    // `Error` type would stop this compiling.
    struct MockRecovery {
        attempts: [u8; 4],
        restores: usize,
        /// Attempt number the source faults on, so a test can make one
        /// restore fail and the next one succeed.
        fail_on: Option<u8>,
        /// Attempts `>=` this have no source left. `None` means unlimited.
        sources: Option<u8>,
    }

    impl MockRecovery {
        fn healthy() -> Self {
            MockRecovery {
                attempts: [0; 4],
                restores: 0,
                fail_on: None,
                sources: None,
            }
        }

        fn faulting_on(attempt: u8) -> Self {
            MockRecovery {
                attempts: [0; 4],
                restores: 0,
                fail_on: Some(attempt),
                sources: None,
            }
        }

        fn with_sources(count: u8) -> Self {
            MockRecovery {
                attempts: [0; 4],
                restores: 0,
                fail_on: None,
                sources: Some(count),
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct MockFault;

    impl core::fmt::Display for MockFault {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("mock restore fault")
        }
    }

    impl core::error::Error for MockFault {}

    impl Recovery for MockRecovery {
        type Error = MockFault;

        fn restore(&mut self, attempt: u8) -> Result<RestoreOutcome, MockFault> {
            if self.fail_on == Some(attempt) {
                return Err(MockFault);
            }
            if self.sources.is_some_and(|sources| attempt >= sources) {
                return Ok(RestoreOutcome::SourcesExhausted);
            }
            self.attempts[self.restores] = attempt;
            self.restores += 1;
            Ok(RestoreOutcome::Restored)
        }
    }

    /// The orchestrator's shape: run the mechanism, judge nothing here.
    /// `attempt` rides in from `Effect::RecoverComponent`, never from a
    /// count the device keeps.
    fn recover<R: Recovery>(dev: &mut R, attempt: u8) -> Result<RestoreOutcome, R::Error> {
        dev.restore(attempt)
    }

    #[test]
    fn contract_is_implementable_without_the_hal() {
        let mut dev = MockRecovery::healthy();

        assert_eq!(recover(&mut dev, 0), Ok(RestoreOutcome::Restored));
        assert_eq!(recover(&mut dev, 1), Ok(RestoreOutcome::Restored));

        assert_eq!(dev.restores, 2);
    }

    #[test]
    fn each_attempt_reaches_the_implementor() {
        let mut dev = MockRecovery::healthy();

        // Out of order and with a gap, so a device that recorded its own
        // call count instead of the argument fails here.
        for attempt in [2, 0, 7] {
            recover(&mut dev, attempt).expect("restore failed");
        }

        assert_eq!(&dev.attempts[..3], &[2, 0, 7]);
    }

    #[test]
    fn a_failed_restore_does_not_block_the_next_attempt() {
        let mut dev = MockRecovery::faulting_on(0);

        recover(&mut dev, 0).expect_err("expected the first attempt to fault");
        recover(&mut dev, 1).expect("the next attempt must still restore");

        assert_eq!(dev.restores, 1);
    }

    // Running out of sources answers `Ok`, not `Err`: it is a report the
    // orchestrator gates by failure policy, not a fault it fails closed on.
    #[test]
    fn sources_exhausted_is_not_an_error() {
        let mut dev = MockRecovery::with_sources(2);

        assert_eq!(recover(&mut dev, 0), Ok(RestoreOutcome::Restored));
        assert_eq!(recover(&mut dev, 1), Ok(RestoreOutcome::Restored));
        assert_eq!(recover(&mut dev, 2), Ok(RestoreOutcome::SourcesExhausted));

        assert_eq!(dev.restores, 2, "no source consumed on the exhausted attempt");
    }

    #[test]
    fn errors_surface_through_the_generic_seam() {
        let mut dev = MockRecovery::faulting_on(0);

        let err = recover(&mut dev, 0).expect_err("expected the restore fault");

        // Display comes from the core::error::Error bound, not a Debug dump.
        assert_eq!(err.to_string(), "mock restore fault");
    }
}
