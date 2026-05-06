# Evidence Card: Runtime lifecycle ownership is required

- Evidence ID: EVD-20260503-runtime-lifecycle-required
- Date captured: 2026-05-03
- Captured by: copilot
- Source type: spec
- Source links:
  - drivers/spimonitor/notes.md#L18
  - drivers/spimonitor/notes.md#L21
  - drivers/spimonitor/notes.md#L27
  - drivers/spimonitor/notes.md#L42
  - drivers/spimonitor/notes.md#L74
- Related decision IDs:
  - DEC-20260503-spi-monitor-control-plane

## Claim

Even with static-first policy data, runtime monitor ownership is required for boot verification, lock transitions, attestation, blocked-event handling, and authenticated update windows.

## Observed Data

- Raw observation: note enumerates required runtime operations and warns that guarantees become hard to prove without runtime monitor component.
- Environment/context: architecture migration plan for AST10x0 SPI monitor.
- Measurement window: design-time document review.

## Interpretation

- What this evidence strongly supports: runtime service ownership is an architectural requirement, not optional implementation detail.
- What this evidence does not prove: final API wire format or token cryptographic design.

## Confidence

- Level: high
- Reason: direct, explicit requirement statements in source architecture note.
- Recency: current migration documentation.

## Counter-Evidence

- Known conflicting evidence IDs: none
- Notes on conflict resolution: n/a

## Reuse Tags

- Components: spimonitor, orchestrator
- Risk area: security lifecycle
- Keywords: lock, attestation, update window, telemetry
