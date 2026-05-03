# SMC Checkpoint

Date: 2026-05-02

## Goal Reminder

Enable full interrupt-driven behavior for AST10x0 SMC integration with the Pigweed kernel as a userspace driver service.

## Checkpoint Summary

1. We have not drifted from the architecture goal.
2. We are in a transitional implementation stage where PIO and polling still carry most active behavior.
3. Interrupt ownership and routing design is in place, but end-to-end interrupt-completion behavior is not yet fully implemented and validated.

## Why PIO Is Still Needed

PIO is intentional, not a detour.

1. Bring-up baseline: It provides the simplest and most deterministic path to validate mapping, offsets, and controller setup.
2. Small-transfer efficiency: DMA setup and interrupt overhead can exceed useful work for short reads.
3. Compatibility path: It preserves a known-good fallback while DMA/IRQ completion paths are hardened.
4. Separation of concerns: NOR write/erase completion still depends on status/WIP progression policy, which remains valid independent of transport mode.

## Polling vs Interrupts: Current State

1. Interrupt scaffolding exists (controller interrupt config + interrupt decode model).
2. Active completion behavior is still primarily polling-based in key flows.
3. Async interrupt-centric test coverage is still incomplete.

This is implementation lag on the interrupt-completion path, not architectural drift.

## TDD Assessment

TDD is the right choice here if scoped to risk.

Use strict TDD for control-plane behavior:
- IRQ ownership and routing
- In-flight request state transitions
- Completion correlation to requester
- Abort/write-protect/error classification
- Timeout and retry/fail policy

Use lighter coverage for data-plane behavior:
- PIO correctness checks
- DMA smoke/perf checks
- Boundary/size thresholds

Conclusion: risk-focused TDD is appropriate for this SMC phase and is not overkill.

## Immediate Next Focus

1. Implement DMA completion/status-clear API and connect it to IRQ-driven completion flow.
2. Add interrupt-first tests for ownership, completion correlation, and ambiguous status decoding.
3. Keep PIO as baseline and fallback, but enforce DMA+IRQ path for large-transfer scenarios.
