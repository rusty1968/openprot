# SMC Kernel Task Routing Design

Date: 2026-05-01
Status: Draft (design-first)

## 1. Problem Statement

The kernel interrupt model routes each interrupt object to a single owning task/process context.
This creates an architectural constraint for SMC integration:

- We cannot directly fan out a single IRQ event to multiple tasks at kernel level.
- Any multi-consumer behavior must be implemented in userspace/service logic.

For AST10x0 SMC, we must support FMC, SPI1, and SPI2 controllers while preserving deterministic IRQ ownership and completion semantics.

## 2. Design Goals

1. Respect kernel invariant: one IRQ object has one owner task.
2. Preserve clear ownership for in-flight DMA operations.
3. Avoid ambiguous interrupt completion semantics.
4. Keep ISR-side logic minimal and non-blocking.
5. Provide a path to both simple and scalable deployment.

## 3. Non-Goals

1. No kernel changes to support IRQ broadcast/fan-out.
2. No shared mutable DMA state across unrelated tasks without explicit arbitration.
3. No implementation changes in this document; this is architecture only.

## 4. Hardware/Signal Model

Per SMC controller, there is one IRQ line with multiple causes multiplexed in interrupt status bits.

- FMC IRQ number: 39
- SPI1 IRQ number: 65
- SPI2 IRQ number: 66

Cause bits (decoded from interrupt status):

- DMA complete
- Command/DMA abort
- Write protect

Implication: routing is two-stage.

1. Kernel routes IRQ line to one owner task.
2. Owner task demultiplexes cause bits and dispatches software events.

## 5. Recommended Architecture

### 5.1 Preferred Mode: One Driver Task Per Controller

Use one task per controller, each owning its own interrupt object and request queue.

- `smc_fmc_task` owns FMC IRQ object.
- `smc_spi1_task` owns SPI1 IRQ object.
- `smc_spi2_task` owns SPI2 IRQ object.

Why this is preferred:

1. Matches hardware isolation and ownership naturally.
2. Simplifies state machine and timeout logic per controller.
3. Eliminates cross-controller lock contention in interrupt path.
4. Keeps failure domains narrow.

### 5.2 Fallback Mode: Single Router Task

If platform constraints require fewer tasks, use one `smc_router_task` that owns all SMC IRQ objects and dispatches completions/errors to worker/client tasks via IPC.

Tradeoffs:

1. Lower task count.
2. Higher complexity and potential head-of-line blocking.
3. Requires strict fairness and bounded handler work.

## 6. Ownership and State Model

Each controller task owns:

1. Controller MMIO mapping.
2. IRQ object for that controller.
3. In-flight operation table (can start with single in-flight operation).
4. Completion channel(s) to clients.

Invariant:

- A completion event is consumed only by the task that created the request.
- IRQ owner task must map status bits to the active request context before acknowledging the IRQ.

## 7. Interrupt-to-Task Flow

Per controller:

1. Client sends request (`dma_read`, later `dma_write/erase`) to controller task.
2. Controller task validates and programs registers.
3. Controller task marks request in-flight.
4. Controller task blocks on wait group/object wait for:
   - request channel readable
   - controller IRQ signal
5. On IRQ signal, task reads interrupt status and decodes cause.
6. Task resolves request outcome:
   - `DmaComplete` -> success completion
   - `DmaError` or `CommandAbort` -> error completion
   - `WriteProtected` -> policy-defined fault/error path
7. Task acknowledges/clears interrupt and transitions state to ready.
8. Task responds to originating client.

## 8. Handling the "One IRQ -> One Task" Constraint

Key principle:

- Kernel routing remains one-to-one.
- Software dispatch handles one-to-many semantics.

Mechanisms:

1. Dedicated owner task per IRQ object.
2. Internal request IDs for correlation.
3. Per-client response channels or callbacks through IPC.
4. Explicit completion state transitions before client notification.

This preserves kernel simplicity while still enabling multiple clients of the same controller task.

## 9. Error Semantics

The decoder must avoid ambiguous mapping:

1. Abort status with DMA in-flight -> `DmaError`.
2. Abort status with no DMA in-flight -> `CommandAbort`.
3. DMA complete has higher priority when both bits are observed.

Controller task policy:

1. Always tie decoded cause to current in-flight state.
2. Reject new DMA request when one is already in-flight (initial design).
3. Return deterministic errors to clients.

## 10. Concurrency Policy

Phase 1:

1. Single in-flight operation per controller task.
2. Additional requests receive Busy/WouldBlock.

Phase 2 (optional):

1. Add bounded queue per controller.
2. Preserve FIFO ordering.
3. Keep IRQ handling constant-time; move heavy work outside IRQ branch.

## 11. Security and Robustness Considerations

1. Validate all offsets, lengths, and DRAM destination constraints before programming DMA.
2. Guard interrupt ack/clear ordering to avoid lost events.
3. Apply per-request timeout and fail-safe abort path.
4. Ensure one controller task cannot mutate another controller state.

## 12. Test Plan Requirements (Design-Level)

1. IRQ ownership test: only owning task receives controller signal.
2. Correlation test: completion is delivered to initiating client.
3. Ambiguity test: abort bit with DMA in-flight maps to `DmaError`.
4. Contention test: second DMA request rejected while first in-flight.
5. Fault test: write-protect and abort paths generate deterministic error responses.

## 13. Migration Plan

1. Start with one controller task (FMC) and prove routing/completion model.
2. Clone pattern for SPI1 and SPI2 with identical service contract.
3. Introduce optional router mode only if platform constraints demand it.

## 14. Open Questions

1. Do we need strict priority among FMC/SPI1/SPI2 services?
2. Should completion transport be synchronous reply or asynchronous notification channel?
3. What timeout budget is acceptable for each operation class?

## 15. Decision Summary

Given the kernel routing constraint, the recommended design is:

1. One driver task per controller.
2. One IRQ object per controller, owned by that task only.
3. Software demultiplexing of cause bits and client completions inside the owning task.

This is the simplest architecture that is correct, scalable, and compatible with the existing kernel interrupt model.