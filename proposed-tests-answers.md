# Proposed tests — concise answers

## 1. `chain_exhaustion_does_not_bypass_irot_gate`

**Answer:** We don't block in `AwaitingReady`; a finished walk goes straight to `Ready`, and a released active that never reports is caught by the boot watchdog, not by waiting.

**Covered by:** `single_active_chain_goes_directly_to_ready`, `timeout_awaited_enters_recovering`

## 2. `replayed_verdict_does_not_release_isolated_component`

**Answer:** A gated component is never re-released: only the component at the cursor can be released, and a replayed verdict for an isolated one is dropped, so `ReleaseReset` fires once.

**Covered by:** `isolable_runtime_corruption_holds_across_rewalk`, `property_verify_before_release_holds_under_random_sequences`

## 3. `component_ready_during_rewalk_clears_watchdog`

**Answer:** A re-walk clears each live component's watchdog via `quiesce_all` on entry, so a stale timeout afterward is dropped and a healthy component is never dragged into recovery.

**Covered by:** `passive_booted_clears_watchdog_then_timeout_is_stale`, `recovery_rewalk_reverifies_live_sibling_at_rest`
