<!-- Licensed under the Apache-2.0 license -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# orchestrator platform driver (`openprot_orchestrator_driver`)

The effect-executing layer around the orchestrator state machine. `PlatformDriver`
implements the SM's `Platform` seam: one executor per `Effect`, each
documenting its obligation from the platform-boundary contract
([orchestrator-model.md §6](../../../docs/src/design/orchestrator/orchestrator-model.md)).
An effect whose capability is not composed yet returns `EffectError` from
`execute`; the SM fail-closes on it.

Everything device-specific arrives through the seams in `board.rs`
(`ImageSource`, `AsyncVerifier` (or a fast local `Verifier` wrapped in
`Immediate`), `BootControl`, `BootWatch`, bundled in `Board`). Nothing the
driver executes waits on a slow service. Short local calls (reset lines, the
SVN floor, reports) complete inside `execute`; the two long reads, the
verification verdict and the boot-walk verdicts, are polled. There is no
driver-side event queue.

`VerifyFirmware` starts a check and returns. The verdict arrives from
`poll_verify` as `VerificationPassed` or `VerificationFailed`; a verifier that
ran past its deadline reports `VerifyStatus::TimedOut`, which the driver
reports as `VerificationFailed` so the machine recovers the component. Starting
a check, or re-staging the image, cancels any check still outstanding, and its
answer is discarded. `ReleaseReset` arms the component's boot walk. The run
loop polls and dispatches until quiet, then sleeps until the earliest walk
deadline or the verifier's wake-up:

```rust
orch.dispatch(&mut driver, event);
loop {
    match driver.poll_verify() {
        Ok(Some(ev)) => { orch.dispatch(&mut driver, ev); continue; }
        Ok(None) => {}
        // The check could not be carried through: fail closed, as an
        // executor error does.
        Err(_) => { orch.dispatch(&mut driver, Event::EffectFailed); continue; }
    }
    let poll = driver.poll_boot_walks(now_millis);
    match poll.event {
        Some(ev) => orch.dispatch(&mut driver, ev),
        None => break, // poll.next_deadline_millis = next wake-up
    }
}
```

Implemented executors: `ReadFirmware`, `VerifyFirmware` (starts the check), `ReleaseReset`
(arms the boot walk), `AssertReset` (stops it). Everything else fails closed
until its pillar lands (recovery, update path, attestation, reporting,
lockdown latch).
