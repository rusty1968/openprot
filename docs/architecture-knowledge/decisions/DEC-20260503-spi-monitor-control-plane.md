# Decision Record: Keep SPI monitor as control-plane service

- Decision ID: DEC-20260503-spi-monitor-control-plane
- Status: accepted
- Owner: platform architecture
- Date: 2026-05-03
- Reviewers: security, firmware, platform
- Related decisions: none

## 1. Decision Question

Should SPI monitor logic be removed during Rust migration (three flash drivers), or retained as a dedicated runtime control-plane service?

## 2. Context And Constraints

- Technical context: migration from Zephyr to microkernel with MPU-isolated user-space drivers.
- Security constraints: runtime policy lock and attestation must remain auditable.
- Reliability constraints: blocked-access events must be captured and consumable by orchestrator.
- Performance constraints: flash data path should remain in dedicated flash drivers.
- Delivery constraints: preserve behavior parity before policy-model hardening.
- Non-goals: redesigning flash protocol behavior in this decision.

## 3. Options Considered

### Option A: Flash drivers only, no runtime SPI monitor service

- Summary: all monitor policy treated as static boot configuration; no runtime manager.
- Benefits: less runtime surface and fewer components.
- Costs: lifecycle transitions and lock attestability become weak/implicit.
- Main risks: no owned API for update-window transitions and blocked-event lifecycle.
- Reversibility: medium (would require adding service later under pressure).

### Option B: Keep dedicated SPI monitor control-plane service (chosen)

- Summary: flash drivers remain data plane; monitor service owns lifecycle, locks, and telemetry.
- Benefits: explicit authorization boundaries, attestation flow, and auditable transitions.
- Costs: additional service and capability management.
- Main risks: token/auth design and event backpressure details still need specification.
- Reversibility: high (can simplify API later while preserving architecture).

### Option C: Merge monitor ownership into orchestrator directly

- Summary: orchestrator writes monitor MMIO and controls policy transitions.
- Benefits: fewer processes than Option B.
- Costs: broader orchestrator trust and larger blast radius.
- Main risks: weak separation of duties and harder least-privilege modeling.
- Reversibility: medium.

## 4. Evidence Summary

| Evidence ID | Supports | Confidence | Notes |
|---|---|---|---|
| EVD-20260503-runtime-lifecycle-required | Runtime ownership still required for lock/attestation/update windows | high | Derived from architecture note with explicit operation list |
| EVD-20260503-zephyr-irq-log-handling | Runtime IRQ and deferred blocked-event handling already exists | high | Zephyr callback + workqueue parsing |
| EVD-20260503-zephyr-state-machine-policy-mutations | Lifecycle code currently performs runtime monitor state changes | high | State machine init and unprovisioned bypass branch |
| EVD-20260503-overlay-static-policy-wiring | Static policy inputs are already board-declarative | high | Overlay includes allow-list and forbidden-region policy |

## 5. Scorecard Result

- Scorecard file: docs/architecture-knowledge/decisions/DEC-20260503-spi-monitor-control-plane-scorecard.md
- Winner by score: Option B
- Human override applied: no
- If override, rationale: n/a

## 6. Decision

- Chosen option: Option B
- Rationale: preserves security lifecycle guarantees while keeping flash data path simple and isolated.
- Why now: migration decisions taken now determine authority boundaries and future auditability.

## 7. Risks And Mitigations

| Risk | Likelihood | Impact | Mitigation | Owner |
|---|---|---|---|---|
| Update-window authorization design is underspecified | med | high | define token format, issuer, ttl, and revoke behavior in next design sprint | security + platform |
| Blocked-event cursor/retention semantics unclear | med | med | implement explicit ring-buffer contract and overflow telemetry | firmware |
| Service/orchestrator capability mapping drifts | low | high | enforce typed capability checks and integration tests | platform |

## 8. Validation Plan

- Required tests/benchmarks: parity tests for init, lock, blocked-event, update-window enter/exit.
- Required security checks: capability misuse tests and lock immutability checks.
- Required rollout checks: boot-to-runtime handoff attestation must pass in CI and hardware smoke.
- Exit criteria: all parity checks green; no unauth monitor mutation paths.

## 9. Revisit Trigger

- Revisit date: 2026-08-03
- Early revisit signals: blocked-event loss, lock mismatch on attestation, or urgent auth model changes.

## 10. Copilot Audit Prompt

"Review this decision for unsupported claims, stale evidence, and missing failure modes. Provide concrete gaps with file links and suggested fixes."
