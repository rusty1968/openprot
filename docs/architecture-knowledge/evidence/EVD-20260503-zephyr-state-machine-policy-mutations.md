# Evidence Card: State machine performs monitor lifecycle and policy mutations

- Evidence ID: EVD-20260503-zephyr-state-machine-policy-mutations
- Date captured: 2026-05-03
- Captured by: copilot
- Source type: code
- Source links:
  - aspeed-zephyr-project/apps/aspeed-pfr/src/AspeedStateMachine/AspeedStateMachine.c#L264
  - aspeed-zephyr-project/apps/aspeed-pfr/src/AspeedStateMachine/AspeedStateMachine.c#L1126
  - aspeed-zephyr-project/apps/aspeed-pfr/src/AspeedStateMachine/AspeedStateMachine.c#L1131
  - aspeed-zephyr-project/apps/aspeed-pfr/src/AspeedStateMachine/AspeedStateMachine.c#L1158
- Related decision IDs:
  - DEC-20260503-spi-monitor-control-plane

## Claim

Lifecycle logic currently invokes monitor initialization and conditionally modifies monitor filtering/enforcement behavior at runtime.

## Observed Data

- Raw observation: spim_irq_init called during initialization; unprovisioned branch configures read/write privilege ranges and disables monitor across spim@1..spim@4.
- Environment/context: platform state-machine boot and unprovisioned flow.
- Measurement window: static code inspection.

## Interpretation

- What this evidence strongly supports: monitor policy behavior is lifecycle-driven and not purely static.
- What this evidence does not prove: the final ownership split needed in the microkernel implementation.

## Confidence

- Level: high
- Reason: explicit function calls in central lifecycle engine.
- Recency: current checked-in Zephyr source.

## Counter-Evidence

- Known conflicting evidence IDs: none
- Notes on conflict resolution: n/a

## Reuse Tags

- Components: state-machine, spimonitor
- Risk area: lifecycle correctness
- Keywords: init, bypass, policy mutation
