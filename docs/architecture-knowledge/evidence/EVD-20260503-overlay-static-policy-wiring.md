# Evidence Card: Board overlay already carries static SPI monitor policy

- Evidence ID: EVD-20260503-overlay-static-policy-wiring
- Date captured: 2026-05-03
- Captured by: copilot
- Source type: code
- Source links:
  - aspeed-zephyr-project/apps/aspeed-pfr/boards/ast1060_prot.overlay#L189
  - aspeed-zephyr-project/apps/aspeed-pfr/boards/ast1060_prot.overlay#L217
  - aspeed-zephyr-project/apps/aspeed-pfr/boards/ast1060_prot.overlay#L230
  - aspeed-zephyr-project/apps/aspeed-pfr/boards/ast1060_prot.overlay#L242
  - aspeed-zephyr-project/apps/aspeed-pfr/boards/ast1060_prot.overlay#L247
  - aspeed-zephyr-project/apps/aspeed-pfr/boards/ast1060_prot.overlay#L296
- Related decision IDs:
  - DEC-20260503-spi-monitor-control-plane

## Claim

A static-first policy model is feasible because monitor topology and policy fields are already declaratively encoded in board configuration.

## Observed Data

- Raw observation: spi-monitor phandle wiring from SPI chip selects to spim nodes; spim nodes define allow-cmds and write-forbidden-regions.
- Environment/context: board overlay for AST1060 platform.
- Measurement window: static DTS overlay inspection.

## Interpretation

- What this evidence strongly supports: policy contents can be loaded from immutable board profiles at boot.
- What this evidence does not prove: runtime lock/attestation ownership can be removed.

## Confidence

- Level: high
- Reason: direct declarative policy properties present in source.
- Recency: current checked-in board overlay.

## Counter-Evidence

- Known conflicting evidence IDs: none
- Notes on conflict resolution: n/a

## Reuse Tags

- Components: dts, spimonitor
- Risk area: configuration correctness
- Keywords: allow-cmds, forbidden-regions, phandle
