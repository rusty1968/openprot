# Evidence Card: Zephyr monitor handles IRQ and deferred blocked-event logs

- Evidence ID: EVD-20260503-zephyr-irq-log-handling
- Date captured: 2026-05-03
- Captured by: copilot
- Source type: code
- Source links:
  - aspeed-zephyr-project/apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L35
  - aspeed-zephyr-project/apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L68
  - aspeed-zephyr-project/apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L113
  - aspeed-zephyr-project/apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L120
  - aspeed-zephyr-project/apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L140
- Related decision IDs:
  - DEC-20260503-spi-monitor-control-plane

## Claim

Current implementation already depends on runtime ISR callback installation and deferred log parsing to process blocked SPI monitor events.

## Observed Data

- Raw observation: callback installed with spim_isr_callback_install; ISR submits work item; worker reads log RAM and parses blocked command/read/write contexts.
- Environment/context: Zephyr app monitor runtime.
- Measurement window: static code inspection.

## Interpretation

- What this evidence strongly supports: blocked-event processing is active runtime behavior and needs a live owner component.
- What this evidence does not prove: exact throughput limits under worst-case event bursts.

## Confidence

- Level: high
- Reason: direct implementation code paths.
- Recency: current checked-in Zephyr source.

## Counter-Evidence

- Known conflicting evidence IDs: none
- Notes on conflict resolution: n/a

## Reuse Tags

- Components: spimonitor, irq, telemetry
- Risk area: detection coverage
- Keywords: workqueue, callback, log parser
