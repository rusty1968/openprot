# Platform-side handling of `RecoverComponent`

The orchestrator SM hands the platform a **component** id
(`Effect::RecoverComponent(ComponentId)`); the platform does the region lookup
and the **joint restore**, and the SM never sees the region. This document gives
pseudocode for that handler and traces the shared-region scenario end to end.

The contract is `Platform::execute`
(`services/orchestrator/sm/src/lib.rs`):

```rust
pub trait Platform {
    fn execute(&mut self, effect: Effect) -> Result<(), EffectError>;
}
```

## Handler pseudocode

```text
// Platform-side config: the mapping the SM deliberately does NOT hold.
// Keyed by ComponentId, dereferenced only when actuating RecoverComponent.
regions: Map<ComponentId, RegionId>          // component -> its region
members: Map<RegionId, Set<ComponentId>>     // region -> all its components
alt_image: Map<RegionId, ImageHandle>        // region -> its alternate image/slot

fn execute(effect) -> Result<(), EffectError> {
    match effect {

        // --- the joint restore lives here ---
        RecoverComponent(failed, attempt) => {
            region = regions[failed]           // component -> region lookup

            // No untried source left for this region: report exhaustion as an
            // EVENT, not an EffectError. RecoveryUnavailable routes through
            // the SM's FailurePolicy (Isolable/Cascading gate-and-skip,
            // Required locks) — an EffectError here would instead fail-closed
            // to Locked unconditionally, regardless of policy.
            if no_untried_source(region, attempt) {
                emit_event(RecoveryUnavailable(failed))
                return Ok(())
            }

            image = next_untried_image(region, attempt)  // e.g. slot A on
                                                           // attempt 0, slot B
                                                           // on 1, golden on 2

            // ONE physical swap. It rewrites on-device firmware for EVERY
            // member of the region together — not just `failed`. This is the
            // "joint restore": siblings sharing the image are restored too.
            swap_to_alternate_image(region, image)?   // fail-closed on error:
                                                       // a swap FAULT (bus
                                                       // error, hardware
                                                       // fault) is the only
                                                       // thing that returns
                                                       // EffectError here.

            // NB: do NOT release or re-verify anything here. The SM owns
            // sequencing. It will next drive PreSupervision, which re-holds
            // (AssertReset) and re-verifies the whole chain at rest. The
            // platform's job ends at "the bits are swapped".
            emit_event(Restored(failed))
            Ok(())
        }

        // --- hold / release / verify are per-component, SM-sequenced ---
        AssertReset(id) => {
            assert_reset_line(id)?      // hold: keep it non-executing, no pulse
            Ok(())
        }
        ReleaseReset(id) => {
            deassert_reset_line(id)?    // only reached after a fresh VerifyFirmware
            Ok(())
        }
        VerifyFirmware(id) => {
            // Read the CURRENT bits at rest and measure/verify them. After a
            // region swap, `id`'s bits may have changed underneath it — this is
            // exactly what re-verifies a sibling like C1 post-swap.
            ok = measure_and_verify(id)
            // Report the verdict back as an event on the SM's event stream:
            emit_event(if ok { VerificationOk(id) } else { VerificationFailed(id) })
            Ok(())
        }

        // ... other effects (StageUpdate, LatchLockdown, etc.) ...
    }
}
```

## Scenario trace: C0 and C1 share a region

Shows C1 is **not** trusted behind the SM's back even though the swap rewrote it.

```text
SM: Recovering(C0)  → execute(RecoverComponent(C0))
                        platform: region = {C0, C1}; swap_to_alternate_image
                        → C0 AND C1 firmware rewritten (joint restore)

SM: → PreSupervision entry
       quiesce_all:     execute(AssertReset(C0)), execute(AssertReset(C1)), ...
                        → every live component (incl. C1) is now HELD
       re-walk top→down: execute(VerifyFirmware(C0)) → VerificationOk/Failed
                         execute(VerifyFirmware(C1)) → re-verifies C1's NEW bits
       only on a fresh Ok: execute(ReleaseReset(Cx))
```

## Contract obligations the platform must honor

- **`AssertReset` holds, it does not pulse.** The component must stay quiesced
  until its matching `ReleaseReset`, so `VerifyFirmware` covers code that cannot
  run or rewrite its own flash between the check and the release.
- **Fail-closed.** Return `EffectError` on any failure (e.g.
  `swap_to_alternate_image` fails); the driver injects `Event::EffectFailed` and
  the SM latches `Locked`.

## Why this keeps the SM region-blind

`RegionId`, `members`, and `alt_image` appear **only** inside the platform's
`execute`. Nothing region-shaped ever crosses back into the SM — which is the
thesis of `region-id-is-platform-side.md`: the SM names the failed component and
re-verifies the whole chain, so it stays correct regardless of how the platform
draws region boundaries.
