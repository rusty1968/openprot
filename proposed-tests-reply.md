# Reply: chrysh's three proposed tests

Assessment of the three tests suggested in the review, mapped against the
current suite in `services/orchestrator/sm/src/tests.rs`. Conclusion: none need
to be added — two are already covered, and the third asserts behavior that
conflicts with a deliberate, tested design choice.

## 1. `chain_exhaustion_does_not_bypass_irot_gate`

**Proposed:** after the final verdict arrives while C0's `ComponentReady` is
still outstanding, the machine must stay in `AwaitingReady(Some(C0))`.

**Why we're not adding it:** this asserts a gate we intentionally don't
implement. When the eRoT walk finishes, the machine goes straight to `Ready`
regardless of a pending active `ComponentReady`. This is established and tested:

- `single_active_chain_goes_directly_to_ready` — a lone active component reaches
  `Ready` on `VerificationPassed` alone, never entering `AwaitingReady`.

Active-readiness after the walk completes is enforced *asynchronously* by the
per-component boot watchdog (`Timeout → Recovering`), not by blocking the walk:

- `timeout_awaited_enters_recovering`
- `passive_boot_timeout_in_ready_enters_recovering`

So the invariant chrysh cares about (a released active that never reports is not
silently accepted) already holds — via the watchdog backstop, not by parking in
`AwaitingReady`. Adding this test as written would fail against the intended
design.

## 2. `replayed_verdict_does_not_release_isolated_component`

**Proposed:** a replayed `VerificationPassed` for a gated component must not
re-release it (`ReleaseReset` count == 1).

**Already covered by:**

- `isolable_runtime_corruption_holds_across_rewalk` — asserts exactly
  `ReleaseReset(C1)` count == 1 across a re-walk after C1 is gated.
- The cursor guard in the `VerificationPassed` handler
  (`chain[cursor] != id → drop`), which rejects any out-of-turn or replayed
  verdict.
- `property_verify_before_release_holds_under_random_sequences` — the fuzz test
  that exercises release-after-verification under random event orderings.

## 3. `component_ready_during_rewalk_clears_watchdog`

**Proposed:** a stale watchdog after a re-walk must not drag a healthy component
into recovery.

**Already covered by:**

- `passive_booted_clears_watchdog_then_timeout_is_stale`
- `recovery_rewalk_reverifies_live_sibling_at_rest`
- `recovery_rewalk_quiesces_all_live_siblings`

Clarification on the mechanism: the watchdog is cleared by `quiesce_all` on
re-walk entry, not by the `ComponentReady` report — an active's `ComponentReady`
arriving in `PreSupervision` is discarded. The stale-`Timeout` invariant holds,
just via a different path than the test name implies.

## Draft comment to post

> Thanks — I went through all three against the current suite:
>
> - **replayed_verdict_does_not_release_isolated_component**: already covered by
>   `isolable_runtime_corruption_holds_across_rewalk` (asserts `ReleaseReset`
>   fires exactly once across the re-walk), plus the `chain[cursor] != id` guard
>   in the `VerificationPassed` handler and the `property_verify_before_release_*`
>   fuzz test.
> - **component_ready_during_rewalk_clears_watchdog**: covered by
>   `passive_booted_clears_watchdog_then_timeout_is_stale` and the
>   `recovery_rewalk_*` quiesce tests. One clarification: the watchdog is cleared
>   by `quiesce_all` on re-walk entry, not by the `ComponentReady` report (which
>   is discarded in `PreSupervision`) — so the stale-`Timeout` invariant holds
>   via a different mechanism than the test name suggests.
> - **chain_exhaustion_does_not_bypass_irot_gate**: this asserts we block in
>   `AwaitingReady(Some(C0))`, but that's intentionally not the design. A
>   completed eRoT walk goes straight to `Ready` (see
>   `single_active_chain_goes_directly_to_ready`); a released active's iRoT
>   readiness is backstopped asynchronously by the per-component boot watchdog
>   (`Timeout → Recovering`, per `timeout_awaited_enters_recovering`), not by
>   blocking the walk. So I'd rather not add this one as written — happy to
>   document that watchdog-backstop invariant more explicitly if it's unclear.
