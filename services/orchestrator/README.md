# Resiliency Orchestrator Service

**Location:** `services/orchestrator/`

The resiliency orchestrator is the system-level control-plane function that
enforces consistent trust state across all domains — the host-side Hardware
Root of Trust (HROT) and any host-side boot-managed firmware — covering the
full lifecycle:

```
PowerOn → verify → [recover if needed] → release boot holds → runtime → attest
```

## Components

The orchestrator is composed of two components with a contract between them:

- **State machine (SM)** (`services/orchestrator/sm/`) — a library component
  containing pure policy logic. No platform I/O, no hardware dependencies.
  Fully testable without hardware.
- **Runner** (`target/<target>/orchestrator/`) — a per-target executable
  component that owns the SM, wires it to driver services, and executes
  effects through the platform impl.

Between them sits the **platform contract** (`ResiliencyPlatform` trait) — an
interface boundary that decouples the runner's hardware-specific code from the
SM's effect vocabulary.

```
SM (library)  ←── ResiliencyPlatform trait ──→  Runner (executable)
```

## Documentation

- **Architecture** — component decomposition, domain model, Verifier, extension
  points, vendor portability: `docs/src/architecture.md`
- **Design** — state hierarchy, state diagrams, effect catalog, SM internals:
  `docs/src/design/orchestrator.md`
- **Specification** — service-level spec within the OpenPRoT specification:
  `docs/src/specification/services/orchestrator.md`
- **AST10x0 runner** — concrete runner implementation for AST10x0:
  `target/ast10x0/orchestrator/README.md`

## File layout

```
services/orchestrator/
  README.md                        ← this file
  sm/
    src/
      lib.rs                       ← Orchestrator struct, states, superstates, actions
      effect.rs                    ← Effect enum + BootTarget
      platform.rs                  ← ResiliencyPlatform trait + NoopPlatform test stub
    BUILD.bazel

target/<target>/orchestrator/      ← per-target runner (one per chip)
  README.md                        ← target-specific runner spec
  src/
    runner.rs                      ← event loop, event ingestion
    platform.rs                    ← impl ResiliencyPlatform for the target
  BUILD.bazel
```
