// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Concrete [`BootWatch`] that walks a device's boot checkpoints in order,
//! polling an [`EvidenceReader`] for each one and judging the per-checkpoint
//! windows against the caller-injected `now_millis`.
//!
//! Generic over the reader (`R`) and signal vocabulary (`G`), so the same
//! walker serves GPIO-backed boards, register-backed SoCs, and test
//! doubles. Board wiring constructs one per component and hands them to
//! the platform driver as `Board::boot_watches`.

#![cfg_attr(not(test), no_std)]

use orchestrator_capabilities::{BootStatus, BootWatch, EvidenceReader, FailureCause, WalkVerdict};
use orchestrator_config::BootCheckpoint;

/// Walks a device's [`BootCheckpoint`]s in declaration order, reading
/// evidence from an [`EvidenceReader`] at each poll. The checkpoint list
/// comes from the device table and is `&'static`: lifetimes match the
/// board config.
///
/// Construction panics on an empty checkpoint list (a device with no
/// checkpoints is unwatchable). A read error is treated as silence
/// (`Booting`), so a transient bus glitch does not kill a healthy boot.
/// A lapsed window is a timeout with no final read: a device that has not
/// reported cannot be judged on a race.
pub struct CheckpointWalk<R, G: 'static> {
    reader: R,
    checkpoints: &'static [BootCheckpoint<G>],
    phase: Phase,
}

enum Phase {
    /// Not currently walking: either never armed, or the walk already
    /// reached a terminal verdict (`Complete`/`Failed`). The two cases need
    /// no distinction — both mean "no further work until the next `arm`" —
    /// so `poll` treats them identically (`Waiting { u64::MAX }`).
    Idle,
    /// `arm` was called but no `poll` has run yet, so there is no `now_millis`
    /// to compute the first checkpoint's deadline from. `arm` has no clock of
    /// its own (see [`BootWatch::arm`]); the first `poll` after this resolves
    /// the deadline and moves to `Walking`.
    Armed,
    /// Waiting on `checkpoints[cursor]`, due by `deadline_millis`.
    Walking { cursor: usize, deadline_millis: u64 },
}

impl<R, G> CheckpointWalk<R, G> {
    /// Binds a reader and its checkpoint list into a walk. The checkpoints
    /// are walked in declaration order; each one's signal is resolved by
    /// the reader.
    ///
    /// # Panics
    ///
    /// Panics if `checkpoints` is empty.
    pub fn new(reader: R, checkpoints: &'static [BootCheckpoint<G>]) -> Self {
        assert!(!checkpoints.is_empty(), "checkpoint list must not be empty");
        Self {
            reader,
            checkpoints,
            phase: Phase::Idle,
        }
    }
}

impl<R: EvidenceReader<G>, G> BootWatch for CheckpointWalk<R, G> {
    fn arm(&mut self) {
        self.phase = Phase::Armed;
    }

    fn poll(&mut self, now_millis: u64) -> WalkVerdict {
        if let Phase::Armed = self.phase {
            let timeout_millis = self.checkpoints[0].timeout().as_millis() as u64;
            let deadline = now_millis.saturating_add(timeout_millis);
            self.phase = Phase::Walking {
                cursor: 0,
                deadline_millis: deadline,
            };
        }

        let (cursor, deadline) = match self.phase {
            Phase::Walking {
                cursor,
                deadline_millis,
            } => (cursor, deadline_millis),
            _ => {
                return WalkVerdict::Waiting {
                    deadline_millis: u64::MAX,
                };
            }
        };

        if now_millis >= deadline {
            self.phase = Phase::Idle;
            return WalkVerdict::Failed {
                checkpoint: self.checkpoints[cursor].name(),
                cause: FailureCause::TimedOut,
            };
        }

        let status = self
            .reader
            .read(self.checkpoints[cursor].signal())
            .unwrap_or(BootStatus::Booting);

        match status {
            BootStatus::Booting => WalkVerdict::Waiting {
                deadline_millis: deadline,
            },
            BootStatus::Booted => {
                let next = cursor + 1;
                if next == self.checkpoints.len() {
                    self.phase = Phase::Idle;
                    WalkVerdict::Complete
                } else {
                    let timeout_millis = self.checkpoints[next].timeout().as_millis() as u64;
                    let new_deadline = now_millis.saturating_add(timeout_millis);
                    self.phase = Phase::Walking {
                        cursor: next,
                        deadline_millis: new_deadline,
                    };
                    WalkVerdict::Waiting {
                        deadline_millis: new_deadline,
                    }
                }
            }
            BootStatus::FailedRetriable => {
                self.phase = Phase::Idle;
                WalkVerdict::Failed {
                    checkpoint: self.checkpoints[cursor].name(),
                    cause: FailureCause::DeviceRetriable,
                }
            }
            BootStatus::FailedFatal => {
                self.phase = Phase::Idle;
                WalkVerdict::Failed {
                    checkpoint: self.checkpoints[cursor].name(),
                    cause: FailureCause::DeviceFatal,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use core::time::Duration;

    const BL1: BootCheckpoint<u8> = BootCheckpoint::new("bl1", 1, Duration::from_millis(100));
    const KERNEL: BootCheckpoint<u8> = BootCheckpoint::new("kernel", 2, Duration::from_millis(200));
    const CHECKPOINTS: &[BootCheckpoint<u8>] = &[BL1, KERNEL];

    // A progress-register reader: signal N is Booted once progress >= N.
    // Mirrors the ProgressReader archetype in the evidence tests.
    //
    // State lives in a `Cell` the test owns and borrows into the reader,
    // rather than behind a `CheckpointWalk::reader_mut()` escape hatch: the
    // walk never exposes its reader, so tests drive the simulated device the
    // same way a real caller would have to — through a handle they set up
    // themselves before construction, not by reaching back into the walk.
    #[derive(Clone, Copy, Default)]
    struct ReaderState {
        level: u8,
        fault: Option<BootStatus>,
        fail_read: bool,
    }

    fn set_level(state: &Cell<ReaderState>, level: u8) {
        state.set(ReaderState {
            level,
            ..state.get()
        });
    }

    fn set_fault(state: &Cell<ReaderState>, fault: Option<BootStatus>) {
        state.set(ReaderState {
            fault,
            ..state.get()
        });
    }

    fn set_fail_read(state: &Cell<ReaderState>, fail_read: bool) {
        state.set(ReaderState {
            fail_read,
            ..state.get()
        });
    }

    struct ProgressReader<'a> {
        state: &'a Cell<ReaderState>,
    }

    #[derive(Debug)]
    struct ReadFault;

    impl core::fmt::Display for ReadFault {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("read fault")
        }
    }

    impl core::error::Error for ReadFault {}

    impl<'a> EvidenceReader<u8> for ProgressReader<'a> {
        type Error = ReadFault;

        fn read(&mut self, signal: &u8) -> Result<BootStatus, ReadFault> {
            let s = self.state.get();
            if s.fail_read {
                return Err(ReadFault);
            }
            if let Some(fault) = s.fault {
                return Ok(fault);
            }
            Ok(if s.level >= *signal {
                BootStatus::Booted
            } else {
                BootStatus::Booting
            })
        }
    }

    fn walk(state: &Cell<ReaderState>) -> CheckpointWalk<ProgressReader<'_>, u8> {
        CheckpointWalk::new(ProgressReader { state }, CHECKPOINTS)
    }

    // ── Happy path ──────────────────────────────────────────────────────

    #[test]
    fn two_checkpoint_walk_completes_when_both_pass() {
        let state = Cell::new(ReaderState::default());
        set_level(&state, 2);
        let mut w = walk(&state);
        w.arm();

        let v = w.poll(0);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 200
            },
            "bl1 passed, now waiting on kernel"
        );

        let v = w.poll(0);
        assert_eq!(v, WalkVerdict::Complete);
    }

    #[test]
    fn single_checkpoint_walk_completes_in_one_poll() {
        let one = &[BL1] as &[_];
        // leak to get 'static
        let one: &'static [BootCheckpoint<u8>] = Box::leak(one.to_vec().into_boxed_slice());
        let state = Cell::new(ReaderState::default());
        set_level(&state, 1);
        let mut w = CheckpointWalk::new(ProgressReader { state: &state }, one);
        w.arm();

        assert_eq!(w.poll(0), WalkVerdict::Complete);
    }

    #[test]
    fn progress_between_polls_advances_the_walk() {
        let state = Cell::new(ReaderState::default());
        let mut w = walk(&state);
        w.arm();

        let v = w.poll(0);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 100
            }
        );

        set_level(&state, 1);
        let v = w.poll(50);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 250
            },
            "bl1 passed at t=50, kernel deadline = 50 + 200"
        );

        set_level(&state, 2);
        let v = w.poll(100);
        assert_eq!(v, WalkVerdict::Complete);
    }

    // ── Timeout ─────────────────────────────────────────────────────────

    #[test]
    fn first_checkpoint_times_out_when_device_is_silent() {
        let state = Cell::new(ReaderState::default());
        let mut w = walk(&state);
        w.arm();

        let v = w.poll(0);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 100
            }
        );

        let v = w.poll(100);
        assert_eq!(
            v,
            WalkVerdict::Failed {
                checkpoint: "bl1",
                cause: FailureCause::TimedOut,
            }
        );
    }

    #[test]
    fn second_checkpoint_times_out_after_first_passes() {
        let state = Cell::new(ReaderState::default());
        set_level(&state, 1);
        let mut w = walk(&state);
        w.arm();

        let v = w.poll(0);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 200
            }
        );

        let v = w.poll(200);
        assert_eq!(
            v,
            WalkVerdict::Failed {
                checkpoint: "kernel",
                cause: FailureCause::TimedOut,
            }
        );
    }

    // ── Device-reported failures ────────────────────────────────────────

    #[test]
    fn retriable_failure_ends_the_walk_early() {
        let state = Cell::new(ReaderState::default());
        set_fault(&state, Some(BootStatus::FailedRetriable));
        let mut w = walk(&state);
        w.arm();

        let v = w.poll(0);
        assert_eq!(
            v,
            WalkVerdict::Failed {
                checkpoint: "bl1",
                cause: FailureCause::DeviceRetriable,
            }
        );
    }

    #[test]
    fn fatal_failure_ends_the_walk_early() {
        let state = Cell::new(ReaderState::default());
        set_fault(&state, Some(BootStatus::FailedFatal));
        let mut w = walk(&state);
        w.arm();

        let v = w.poll(0);
        assert_eq!(
            v,
            WalkVerdict::Failed {
                checkpoint: "bl1",
                cause: FailureCause::DeviceFatal,
            }
        );
    }

    // ── Read errors ─────────────────────────────────────────────────────

    #[test]
    fn read_error_treated_as_silence() {
        let state = Cell::new(ReaderState::default());
        set_fail_read(&state, true);
        let mut w = walk(&state);
        w.arm();

        let v = w.poll(0);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 100
            },
            "bus glitch does not kill a healthy boot"
        );

        // Clear the fault and advance: the walk continues.
        set_fail_read(&state, false);
        set_level(&state, 2);
        let v = w.poll(10);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 210
            }
        );
        assert_eq!(w.poll(10), WalkVerdict::Complete);
    }

    // ── arm() rewinds ───────────────────────────────────────────────────

    #[test]
    fn arm_rewinds_to_the_first_checkpoint() {
        let state = Cell::new(ReaderState::default());
        set_level(&state, 2);
        let mut w = walk(&state);
        w.arm();
        assert_eq!(
            w.poll(0),
            WalkVerdict::Waiting {
                deadline_millis: 200
            }
        );
        assert_eq!(w.poll(0), WalkVerdict::Complete);

        // Re-arm: back to checkpoint 0.
        set_level(&state, 0);
        w.arm();
        let v = w.poll(1000);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 1100
            },
            "fresh deadline from the re-arm"
        );
    }

    #[test]
    fn arm_mid_walk_restarts_from_the_beginning() {
        let state = Cell::new(ReaderState::default());
        set_level(&state, 1);
        let mut w = walk(&state);
        w.arm();
        w.poll(0); // passes bl1, now at kernel

        w.arm(); // restart
        set_level(&state, 0);
        let v = w.poll(500);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 600
            },
            "restarted at bl1 with a fresh deadline"
        );
    }

    // ── Unarmed / idle ─────────────────────────────────────────────────

    #[test]
    fn unarmed_walk_waits_indefinitely() {
        let state = Cell::new(ReaderState::default());
        let w = walk(&state);
        // Deliberately not calling arm().
        let mut w = w;
        assert_eq!(
            w.poll(0),
            WalkVerdict::Waiting {
                deadline_millis: u64::MAX
            }
        );
    }

    #[test]
    fn idle_after_terminal_waits_indefinitely() {
        let state = Cell::new(ReaderState::default());
        let mut w = walk(&state);
        w.arm();
        w.poll(0); // Armed -> Walking, deadline = 100
        let v = w.poll(100); // now >= deadline -> TimedOut
        assert!(matches!(v, WalkVerdict::Failed { .. }));

        assert_eq!(
            w.poll(200),
            WalkVerdict::Waiting {
                deadline_millis: u64::MAX
            }
        );
    }

    // ── Deadline arithmetic ─────────────────────────────────────────────

    #[test]
    fn deadline_is_relative_to_first_poll_not_arm() {
        let state = Cell::new(ReaderState::default());
        let mut w = walk(&state);
        w.arm();
        // First poll at t=1000: deadline should be 1000 + 100, not 0 + 100.
        let v = w.poll(1000);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 1100
            }
        );
    }

    #[test]
    fn next_checkpoint_deadline_is_relative_to_the_passing_poll() {
        let state = Cell::new(ReaderState::default());
        set_level(&state, 1);
        let mut w = walk(&state);
        w.arm();

        // bl1 passes at t=50, kernel deadline = 50 + 200.
        let v = w.poll(50);
        assert_eq!(
            v,
            WalkVerdict::Waiting {
                deadline_millis: 250
            }
        );
    }

    // ── Decision 3: lapsed window = timeout, no last-chance read ────────

    #[test]
    fn booted_at_expiry_is_still_timeout() {
        let state = Cell::new(ReaderState::default());
        let mut w = walk(&state);
        w.arm();
        w.poll(0); // Armed -> Walking, deadline = 100

        set_level(&state, 1);
        let v = w.poll(100); // device ready, but window already lapsed
        assert_eq!(
            v,
            WalkVerdict::Failed {
                checkpoint: "bl1",
                cause: FailureCause::TimedOut,
            },
            "no last-chance read: lapsed is lapsed"
        );
    }

    // ── Fault at a later checkpoint names it correctly ────────────────

    #[test]
    fn device_fault_at_second_checkpoint_names_it() {
        let state = Cell::new(ReaderState::default());
        set_level(&state, 1);
        let mut w = walk(&state);
        w.arm();
        w.poll(0); // bl1 passes, now at kernel

        set_fault(&state, Some(BootStatus::FailedFatal));
        let v = w.poll(10);
        assert_eq!(
            v,
            WalkVerdict::Failed {
                checkpoint: "kernel",
                cause: FailureCause::DeviceFatal,
            }
        );
    }

    // ── Construction ────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "checkpoint list must not be empty")]
    fn empty_checkpoints_panic_at_construction() {
        let empty: &'static [BootCheckpoint<u8>] = &[];
        let state = Cell::new(ReaderState::default());
        CheckpointWalk::new(ProgressReader { state: &state }, empty);
    }
}
