// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! A keyed map of one-shot deadlines.
//!
//! [`DeadlineMap`] holds up to `N` absolute deadlines, one per key, and
//! answers two questions for an event loop: when is the nearest deadline (to
//! pass to a blocking wait), and which key is due now. It owns no clock and no
//! policy. The caller supplies absolute deadlines and the current time in the
//! same unit, so the map stays `no_std`, allocation-free and host-testable.
//! What a deadline means, and what to do when one fires, stay with the
//! caller.
//!
//! Semantics, which callers rely on:
//!
//! - **One entry per key.** [`arm`](DeadlineMap::arm) replaces an existing
//!   entry, so restarting a window never stacks a second one.
//! - **Fixed capacity.** A new key beyond `N` fails with [`Full`] and is never
//!   dropped silently. Replacing an existing key needs no free slot.
//! - **One-shot.** [`take_due`](DeadlineMap::take_due) removes the entry it
//!   reports. A fired deadline is gone until armed again.
//! - **Due means `now >= deadline`.** The earliest deadline is reported
//!   first. Entries with equal deadlines come out in no particular order.
//! - **Cancel is a no-op for an unarmed key**, so it needs no care about a
//!   deadline that already fired.
//!
//! The lookup is a linear scan, which suits the small `N` this is meant for.

#![no_std]
#![forbid(unsafe_code)]

/// The map is full: a new key would exceed `N`. Returned so the caller can
/// escalate instead of losing a deadline silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

struct Entry<K, T> {
    key: K,
    at: T,
}

/// Up to `N` one-shot deadlines, keyed by `K`, in the caller's time type `T`.
pub struct DeadlineMap<K, T, const N: usize> {
    entries: heapless::Vec<Entry<K, T>, N>,
}

impl<K: Copy + Eq, T: Copy + Ord, const N: usize> DeadlineMap<K, T, N> {
    /// An empty map.
    pub const fn new() -> Self {
        Self {
            entries: heapless::Vec::new(),
        }
    }

    /// Arm `key` to fire at `deadline`, replacing its existing deadline if it
    /// has one.
    ///
    /// Fails with [`Full`] only when `key` is new and the map already holds
    /// `N` keys. Replacing an existing key never fails.
    pub fn arm(&mut self, key: K, deadline: T) -> Result<(), Full> {
        match self.entries.iter_mut().find(|e| e.key == key) {
            Some(e) => {
                e.at = deadline;
                Ok(())
            }
            None => self
                .entries
                .push(Entry { key, at: deadline })
                .map_err(|_| Full),
        }
    }

    /// Remove `key`'s deadline. Does nothing if `key` is not armed.
    pub fn cancel(&mut self, key: K) {
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            self.entries.swap_remove(i);
        }
    }

    /// The nearest armed deadline, or `None` when nothing is armed. Feed this
    /// to the loop's blocking wait as its deadline.
    pub fn next_deadline(&self) -> Option<T> {
        self.entries.iter().map(|e| e.at).min()
    }

    /// Remove and return the key with the earliest deadline that is due at
    /// `now`, or `None` when nothing is due.
    ///
    /// Call in a loop until `None` to drain everything that has passed.
    pub fn take_due(&mut self, now: T) -> Option<K> {
        let (i, _) = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.at <= now)
            .min_by_key(|(_, e)| e.at)?;
        Some(self.entries.swap_remove(i).key)
    }
}

impl<K: Copy + Eq, T: Copy + Ord, const N: usize> Default for DeadlineMap<K, T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u8 = 0;
    const B: u8 = 1;
    const C: u8 = 2;

    type Map = DeadlineMap<u8, u64, 4>;

    #[test]
    fn empty_has_no_deadline_and_nothing_due() {
        let mut m = Map::new();
        assert_eq!(m.next_deadline(), None);
        assert_eq!(m.take_due(100), None);
    }

    #[test]
    fn next_deadline_is_the_nearest() {
        let mut m = Map::new();
        m.arm(A, 50).unwrap();
        m.arm(B, 30).unwrap();
        m.arm(C, 40).unwrap();
        assert_eq!(m.next_deadline(), Some(30));
    }

    #[test]
    fn expiry_fires_once_and_not_before_the_deadline() {
        let mut m = Map::new();
        m.arm(A, 100).unwrap();
        assert_eq!(m.take_due(99), None);
        assert_eq!(m.take_due(100), Some(A));
        // one-shot: gone until armed again
        assert_eq!(m.take_due(100), None);
        assert_eq!(m.next_deadline(), None);
    }

    #[test]
    fn a_deadline_already_past_is_due() {
        let mut m = Map::new();
        m.arm(A, 10).unwrap();
        assert_eq!(m.take_due(500), Some(A));
    }

    #[test]
    fn cancel_disarms_and_is_a_no_op_when_unarmed() {
        let mut m = Map::new();
        m.arm(A, 10).unwrap();
        m.cancel(A);
        m.cancel(A);
        m.cancel(B);
        assert_eq!(m.take_due(10), None);
        assert_eq!(m.next_deadline(), None);
    }

    #[test]
    fn arm_replaces_the_deadline_and_never_stacks() {
        let mut m = Map::new();
        m.arm(A, 10).unwrap();
        m.arm(A, 50).unwrap();
        assert_eq!(m.take_due(10), None); // old deadline is gone
        assert_eq!(m.take_due(50), Some(A));
        assert_eq!(m.take_due(50), None); // only one entry ever existed
    }

    #[test]
    fn arm_can_move_a_deadline_earlier() {
        let mut m = Map::new();
        m.arm(A, 50).unwrap();
        m.arm(A, 10).unwrap();
        assert_eq!(m.next_deadline(), Some(10));
    }

    #[test]
    fn full_map_reports_full_on_a_new_key() {
        let mut m = DeadlineMap::<u8, u64, 2>::new();
        m.arm(A, 10).unwrap();
        m.arm(B, 10).unwrap();
        assert_eq!(m.arm(C, 10), Err(Full));
        // the failed arm left the others alone
        assert!(m.take_due(10).is_some());
        assert!(m.take_due(10).is_some());
        assert_eq!(m.take_due(10), None);
    }

    #[test]
    fn replacing_an_existing_key_needs_no_free_slot() {
        let mut m = DeadlineMap::<u8, u64, 2>::new();
        m.arm(A, 10).unwrap();
        m.arm(B, 10).unwrap();
        assert_eq!(m.arm(A, 20), Ok(()));
    }

    #[test]
    fn a_cancelled_slot_can_be_reused() {
        let mut m = DeadlineMap::<u8, u64, 1>::new();
        m.arm(A, 10).unwrap();
        assert_eq!(m.arm(B, 10), Err(Full));
        m.cancel(A);
        assert_eq!(m.arm(B, 10), Ok(()));
    }

    #[test]
    fn drains_all_due_in_deadline_order() {
        let mut m = Map::new();
        m.arm(A, 15).unwrap();
        m.arm(B, 10).unwrap();
        m.arm(C, 99).unwrap();

        let mut drained = heapless::Vec::<u8, 4>::new();
        while let Some(k) = m.take_due(20) {
            drained.push(k).unwrap();
        }
        assert_eq!(drained.as_slice(), &[B, A]);
        // the one not yet due is still armed
        assert_eq!(m.next_deadline(), Some(99));
    }

    #[test]
    fn equal_deadlines_are_all_reported_exactly_once() {
        let mut m = Map::new();
        m.arm(C, 20).unwrap();
        m.arm(A, 20).unwrap();
        m.arm(B, 20).unwrap();

        let mut seen = [false; 3];
        for _ in 0..3 {
            let k = m.take_due(20).unwrap();
            assert!(!seen[k as usize], "reported twice");
            seen[k as usize] = true;
        }
        assert_eq!(seen, [true; 3]);
        assert_eq!(m.take_due(20), None);
    }

    #[test]
    fn an_earlier_deadline_is_reported_before_a_later_one() {
        let mut m = Map::new();
        m.arm(A, 30).unwrap();
        m.arm(C, 20).unwrap();
        assert_eq!(m.take_due(30), Some(C));
        assert_eq!(m.take_due(30), Some(A));
    }

    #[test]
    fn keys_need_only_equality_and_the_time_type_is_the_callers() {
        // A key with no ordering, like an opaque id, and a non-u64 time.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Key {
            Boot(u8),
            Commit,
        }
        let mut m = DeadlineMap::<Key, u32, 4>::new();
        m.arm(Key::Commit, 20).unwrap();
        m.arm(Key::Boot(0), 10).unwrap();
        assert!(m.take_due(20) == Some(Key::Boot(0)));
        assert!(m.take_due(20) == Some(Key::Commit));
    }
}
