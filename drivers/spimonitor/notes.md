# AST10x0 SPI Monitor: Rust Migration Decision and Target Design

## Purpose

This document defines whether SPI monitor logic is still needed while migrating
from Zephyr to a microkernel architecture with MPU-isolated user-space drivers,
and with three Rust flash-driver instances (one per controller).

Decision summary:

- Keep three flash data-plane drivers (FMC, SPI1, SPI2).
- Keep a SPI monitor component as a separate control-plane service/driver.
- Make policy data mostly static, but keep a small runtime manager for
  initialization, verification, locking, telemetry, and controlled transitions.

## Why A SPI Monitor Component Is Still Needed

Even if most policy is static, production PFR lifecycle needs runtime ownership
for at least these operations:

- boot-time policy program and readback verification
- lock sequencing at runtime handoff
- lock-status attestation to orchestrator
- blocked access IRQ/log handling for detection evidence
- tightly controlled temporary policy windows for authenticated update/recovery

Without a runtime monitor component, these guarantees become difficult to prove,
especially for lifecycle transitions and attestation.

## What Can Be Static

The following should be modeled as immutable profile data by default:

- command allow-list per monitor instance
- read forbidden regions
- write forbidden regions
- passthrough and mux defaults
- normal runtime lock plan

Store these as declarative board policy inputs and load them once during boot.

## What Must Remain Dynamic

Dynamic does not mean frequently mutable. It means runtime code still owns and
enforces state transitions:

- apply policy at boot and verify register readback
- execute one-way lock transition at T-Zero equivalent
- export lock/health status to orchestrator
- process blocked-event telemetry
- optionally enter/exit authenticated update profile

## Current Zephyr Evidence (Grounding)

Current repository usage proves monitor runtime behavior exists today:

- app callback and deferred log parsing:
  apps/aspeed-pfr/src/platform_monitor/spim_monitor.c
- state-machine initialization hook:
  apps/aspeed-pfr/src/AspeedStateMachine/AspeedStateMachine.c
- utility wrappers used by app flows:
  apps/aspeed-pfr/src/spi_filter/spim_util.c

Board policy wiring exists in overlays and bindings:

- overlay topology example: apps/aspeed-pfr/boards/ast1060_prot.overlay
- SPI NOR monitor phandle property:
  ../zephyr/dts/bindings/mtd/jedec,spi-nor.yaml
- SPI controller topology properties:
  ../zephyr/dts/bindings/spi/aspeed,spi-controller.yaml
- SPI monitor child-node policy properties:
  ../zephyr/dts/bindings/spi/aspeed,spi-monitor-controller.yaml

## Target Runtime Split

### 1) Flash drivers (three Rust instances)

- one instance per controller (FMC, SPI1, SPI2)
- data movement only (read/write/erase)
- no direct SPI monitor policy ownership

### 2) SPI monitor service (single Rust service managing all instances)

- owns policy lifecycle for SPIM instances
- owns lock and attestation state
- owns blocked-event collection/export

### 3) Orchestrator

- requests lifecycle transitions via monitor service
- never writes monitor MMIO directly

## Minimal SPI Monitor API (Recommended)

Keep interface small and capability-gated.

- init_apply(policy_set)
- verify_readback()
- lock_runtime_policy()
- get_lock_status()
- get_blocked_events(cursor)
- clear_blocked_events(cursor)
- enter_update_window(profile_id, token)
- exit_update_window(token)

Capability classes:

- MonitorRead
- MonitorConfigure
- MonitorLock
- MonitorEventSubscribe

Only orchestrator-trusted identity should hold MonitorLock.

## Theory Of Operation (Jargon Decoder)

Use this mental model: SPI monitor is a hardware firewall, while flash drivers
are the data movers.

- flash drivers issue read/write/erase traffic
- monitor service enforces command and address policy
- orchestrator controls lifecycle transitions
- capabilities decide who can read, configure, lock, and subscribe to events

### End-to-End Flow

1. Boot setup
- call init_apply(policy_set)
- service programs monitor policy tables

2. Integrity check
- call verify_readback()
- service compares expected policy with register readback

3. Runtime seal
- call lock_runtime_policy()
- service sets one-way lock bits for runtime protection

4. Runtime attestation
- call get_lock_status()
- orchestrator verifies lock state before normal operation

5. Runtime detection loop
- call get_blocked_events(cursor) to fetch new blocked-access events
- call clear_blocked_events(cursor) to advance acknowledgement state

6. Controlled update or recovery
- call enter_update_window(profile_id, token)
- service applies a temporary pre-validated relaxed profile
- perform authorized update steps
- call exit_update_window(token)
- service restores hardened runtime profile and re-establishes attestable state

### API Meanings

- init_apply(policy_set)
  Programs monitor instances from declarative policy input.

- verify_readback()
  Confirms hardware state matches intended policy.

- lock_runtime_policy()
  Irreversibly locks selected monitor controls for runtime.

- get_lock_status()
  Returns lock state by monitor instance and scope.

- get_blocked_events(cursor)
  Returns blocked command/address events after a cursor position.

- clear_blocked_events(cursor)
  Acknowledges consumed events so polling can continue incrementally.

- enter_update_window(profile_id, token)
  Opens a temporary policy window after authorization checks.

- exit_update_window(token)
  Closes the temporary window and restores hardened policy state.

### Capability Meanings

- MonitorRead
  Permission to query monitor status, policy snapshot, and health.

- MonitorConfigure
  Permission to apply approved policy profiles.

- MonitorLock
  Permission to execute lock transitions and lock-affecting operations.

- MonitorEventSubscribe
  Permission to receive blocked-access event stream notifications.

### Practical Rule

- static by default: policy contents come from predefined board profiles
- dynamic only by lifecycle: transitions are explicit, authorized, and auditable
- no ad-hoc edits in normal runtime

## Lifecycle Contract

### Boot

- load static board policy
- apply to monitor instances
- verify readback

### Runtime handoff

- apply hardened runtime profile (if different from boot profile)
- lock command/address/control scopes per plan
- attest lock state to orchestrator

### Runtime steady state

- deny policy mutation by default
- continue telemetry/reporting

### Authenticated update or recovery window

- temporary bounded profile under explicit authorization
- automatic restoration to runtime profile
- re-attest lock state when window closes

## Design Guidance: Static-First, Runtime-Minimal

Use this rule set:

- policy contents are static by default
- runtime code may transition only among pre-validated profiles
- ad-hoc policy edits are disallowed in normal operation
- lock transition is explicit and observable

This keeps complexity low while preserving security and auditability.

## Concurrency Model

- single writer per monitor instance for mutating operations
- snapshot reads can be concurrent
- blocked event stream is asynchronous

This aligns with existing per-instance synchronization concepts while fitting
an MPU-isolated microkernel process model.

## Migration Plan

### Phase 0: Parity baseline

- freeze current Zephyr policy by board
- capture expected lock and blocked-event behavior on hardware

### Phase 1: Rust API compatibility layer

- wrap current monitor control calls behind target API
- keep behavior unchanged

### Phase 2: Isolated monitor service

- move monitor runtime to dedicated MPU-isolated user-space service
- enforce capabilities on all mutating calls

### Phase 3: Full static-first policy model

- load policy from declarative profiles only
- disable non-lifecycle policy edits
- add attestation checkpoints in orchestrator state transitions

## Validation Checklist

1. Policy readback equals expected profile for each monitor instance.
2. Runtime lock bits are asserted and immutable as expected.
3. Blocked command/address events are generated and consumed correctly.
4. Update window can open/close only with valid authorization.
5. Post-window runtime policy and lock status are restored and attested.

## Final Answer To The Architecture Question

With three Rust flash-driver instances, you should still implement a SPI
monitor component. Most policy data can be static, but a minimal runtime monitor
service is still required for secure lifecycle control, lock attestation, and
monitor telemetry.