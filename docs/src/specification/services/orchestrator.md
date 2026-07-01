# Resiliency Orchestrator

Status: Draft

## Overview

The Resiliency Orchestrator is the system-level control-plane service that
enforces consistent firmware trust state across all platform domains. It
coordinates the full resiliency lifecycle for each domain the PRoT manages:

```
PowerOn → verify → [recover if needed] → release boot holds → runtime → attest
```

This service is the implementation of the PRoT Resiliency and Connected Device
Resiliency requirements described in
[Firmware Resiliency](../firmware_resiliency.md).

## Security guarantees

- No unauthorized firmware executes before it is verified.
- The system maintains a consistent and auditable trust state at all times.
- Recovery behavior is deterministic and bounded.
- Attestation reflects post-verification, post-recovery state.
- Individual devices may reach operational state independently — the
  orchestrator does not require globally uniform state prior to enabling
  execution of individual devices (Non-Uniform System State Principle).

## Domains

A **domain** is a hardware component whose trust state the orchestrator is
responsible for — verifying its firmware, holding it in reset until
verification passes, recovering it on failure, and releasing it when safe.

| Domain | Role |
|--------|------|
| `RoT` | The host-side Hardware Root of Trust (HROT) managed by the PRoT. Verified during `Init`; recovered via `RotRecovery` if verification fails. |
| `HostTarget` | The primary host boot path. Held in reset during `BootGate`; released when verification passes. |

The domain set is extensible. Additional boot-managed or attestable domains
can be added without modifying the SM's state transition logic. Adding a new
domain is an architectural change — it extends the domain set and requires
design review.

## Relationship to the Verifier

The orchestrator depends on the Verifier service (see
[Attestation](./attestation.md)) to appraise domain firmware Evidence against
Reference Values and produce Attestation Results. The orchestrator acts on
the Verifier's verdict — it never makes appraisal decisions itself.

The PRoT acts as:
- A **local Verifier** — appraising Evidence from HROT and HostTarget domains.
- A **Lead Attester** — conveying Attestation Results upstream to a Layer N−1
  Verifier in the broader RATS topology (RFC 9334).

## Interface

The orchestrator exposes its behavior through a normalized event/effect
vocabulary shared across all platform targets:

**Events (inputs to the SM):**
- `PowerOn` — system power-on detected
- `InitComplete { hrot_ok }` — HROT initialization result
- `VerifyComplete { result }` — Verifier appraisal result
- `RecoveryComplete { result }` — recovery operation result
- `UpdateComplete { result }` — firmware update result
- `WatchdogTimeout { target }` — domain watchdog expiry
- `RebootRequested` — platform reboot request
- `SeamlessUpdateRequested` — seamless update request (config-gated)
- `ResetDetected { target }` — domain reset-detect line assertion

**Effects (outputs from the SM):**
- `StartVerification(BootTarget)` — request Verifier to appraise domain
- `HoldBoot(BootTarget)` — assert boot-hold for domain
- `ReleaseBoot(BootTarget)` — deassert boot-hold for domain
- `ArmWatchdog` / `DisarmWatchdog` — start/stop domain watchdog timers
- `ArmMonitors` / `DisarmMonitors` — enable/disable reset monitors
- `SetPlatformState(s)` — publish platform state
- `LogPanic` — record last-panic cause
- `Reboot` — issue hardware reboot
- `HaltBoot` — hold all domains; enter lockdown state

## Implementation

The orchestrator is implemented in `services/orchestrator/`. Each hardware
target provides a concrete runner and platform impl in
`target/<target>/orchestrator/`.

For the component architecture and extension points see
`docs/src/architecture.md`. For the state machine design see
`docs/src/design/orchestrator.md`.

## Normative requirements

| ID | Requirement |
|----|-------------|
| OR-1 | The orchestrator SHALL verify all domain firmware before releasing boot holds. |
| OR-2 | The orchestrator SHALL NOT require globally uniform state prior to enabling individual device execution. |
| OR-3 | Recovery SHALL be deterministic and bounded by a maximum retry count. |
| OR-4 | Attestation results SHALL reflect post-verification, post-recovery state only. |
| OR-5 | The SM transition logic SHALL NOT be modified to port to a new hardware target. |
| OR-6 | Policy decisions (appraisal) SHALL reside in the Verifier, not the SM. |
