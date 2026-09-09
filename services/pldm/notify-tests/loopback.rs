// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host end-to-end: `PldmLink` (real marshalling) -> `LoopbackTransport` ->
//! `notify_server::dispatch` against a shared `NotifyState`. No kernel, no
//! QEMU. This is the regression guard for "the notify encoders/decoders must
//! be host-testable".

use notify_api::{Decision, Pending, Transport, TransportError};
use notify_client::{ClientError, PldmLink};
use notify_server::loopback::LoopbackTransport;
use notify_server::NotifyState;

/// Test A: subscribe -> server latches `UpdateRequested` -> `poll()` drains
/// it -> `decide(Accepted)`.
#[test]
fn subscribe_poll_decide_flow() {
    let mut state = NotifyState::new();
    {
        let mut link = PldmLink::new(LoopbackTransport::new(&mut state));
        link.subscribe().unwrap();
        assert_eq!(link.poll().unwrap(), None);
    }

    // Simulate the (Phase 2) terminus loop latching a UA event and nudging
    // the peer signal; Phase 1 has no wire op for this, so drive it directly.
    state.latch(Pending::UpdateRequested);

    let mut link = PldmLink::new(LoopbackTransport::new(&mut state));
    assert_eq!(link.poll().unwrap(), Some(Pending::UpdateRequested));
    link.decide(Decision::Accepted).unwrap();
}

/// Test B (race): an event latched *before* `poll()` is still delivered by
/// the next `poll()` — proves the level-latched semantics (no lost wakeup).
#[test]
fn event_latched_before_poll_is_still_delivered() {
    let mut state = NotifyState::new();
    state.latch(Pending::Offer {
        target: 0x8000_0000,
        total: 65536,
    });

    let mut link = PldmLink::new(LoopbackTransport::new(&mut state));
    assert_eq!(
        link.poll().unwrap(),
        Some(Pending::Offer {
            target: 0x8000_0000,
            total: 65536
        })
    );
    // Drained: a second poll sees nothing pending.
    assert_eq!(link.poll().unwrap(), None);
}

/// Test C (timeout): a `Transport` that never answers surfaces a
/// timeout/error variant the runtime can later treat as "unhealthy peer".
struct DeadTransport;

impl Transport for DeadTransport {
    fn transact(&mut self, _req: &[u8], _resp: &mut [u8]) -> Result<usize, TransportError> {
        Err(TransportError::Timeout)
    }
}

#[test]
fn silent_peer_surfaces_timeout() {
    let mut link = PldmLink::new(DeadTransport);
    assert_eq!(
        link.poll(),
        Err(ClientError::Transport(TransportError::Timeout))
    );
}
