# Experiment Plan: Validate update-window authorization model

- Experiment ID: EXP-20260503-update-window-authz-model
- Related decision ID: DEC-20260503-spi-monitor-control-plane
- Owner: security + platform
- Start date: 2026-05-03
- End date target: 2026-05-17

## Hypothesis

If update-window entry requires a short-lived signed token bound to profile_id, caller identity, and nonce, then unauthorized policy relaxation attempts will fail while authorized maintenance flow remains usable.

## Why This Matters

Decision DEC-20260503-spi-monitor-control-plane has high impact but remaining uncertainty in token/auth lifecycle details.

## Method

- Setup: implement prototype enter_update_window(profile_id, token) verifier in monitor service stub.
- Workload/scenario:
  - valid token enter/exit sequence
  - expired token
  - replayed token (same nonce)
  - wrong identity/capability
  - wrong profile_id binding
- Instrumentation: structured audit logs and failure counters.
- Data collection approach: run scripted negative and positive tests in CI.

## Success Criteria

- Metric 1 threshold: 100 percent rejection for invalid/expired/replayed tokens.
- Metric 2 threshold: 100 percent success for valid authorized flow.
- Failure trigger: any unauthorized entry accepted.

## Safety Guardrails

- Rollback condition: disable update-window feature gate if verifier mismatch appears.
- Blast radius limit: test mode only; no production profile mutation during experiment.

## Results

- Outcome: inconclusive
- Summary data: pending
- Unexpected observations: pending

## Decision Impact

- Confidence change: pending
- Recommended action: pending
