# Architecture Decision Dashboard

Keep this file updated so reviewers can quickly understand open and settled decisions.

## Open Decisions

| Decision ID | Title | Owner | Status | Due | Top Unknown |
|---|---|---|---|---|---|
| DEC-20260503-spi-monitor-control-plane | Keep SPI monitor as control-plane service | platform architecture | accepted | 2026-08-03 | update-window token auth model |

## Accepted Decisions

| Decision ID | Title | Accepted On | Revisit Date | Outcome |
|---|---|---|---|---|
| DEC-20260503-spi-monitor-control-plane | Keep SPI monitor as control-plane service | 2026-05-03 | 2026-08-03 | pending |

## Evidence Backlog

| Evidence ID | Claim | Source Type | Confidence | Linked Decision |
|---|---|---|---|---|
| EVD-20260503-runtime-lifecycle-required | Runtime lifecycle ownership is required | spec | high | DEC-20260503-spi-monitor-control-plane |
| EVD-20260503-zephyr-irq-log-handling | Runtime IRQ/log handling exists in Zephyr monitor | code | high | DEC-20260503-spi-monitor-control-plane |
| EVD-20260503-zephyr-state-machine-policy-mutations | State machine mutates monitor policy at runtime | code | high | DEC-20260503-spi-monitor-control-plane |
| EVD-20260503-overlay-static-policy-wiring | Board overlay encodes static monitor policy | code | high | DEC-20260503-spi-monitor-control-plane |

## Active Experiments

| Experiment ID | Hypothesis | Owner | End Date | Status |
|---|---|---|---|---|
| EXP-20260503-update-window-authz-model | Signed token model blocks unauthorized policy windows | security + platform | 2026-05-17 | running |

## Notes

- Prefer small, verifiable decisions over broad one-shot migrations.
- Every accepted decision should have at least one linked outcome review.
