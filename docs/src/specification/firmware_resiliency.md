# Firmware Resiliency

Firmware resiliency is a critical concept in modern cybersecurity, particularly
as outlined in the NIST SP 800-193 specification. NIST SP 800-193 addresses
firmware security concerns by providing a comprehensive framework for enhancing
the security and resiliency of platform firmware across three main pillars:

- **Protection** — preventing unauthorized firmware modifications using
  cryptographic authentication of updates and images.
- **Detection** — identifying unauthorized changes quickly through integrity
  checks and monitoring mechanisms.
- **Recovery** — restoring firmware to a known-good state after an attack or
  corruption, ensuring continued secure operation.

OpenPRoT implements these pillars at two distinct scopes: the PRoT itself, and
the connected devices the PRoT manages.

---

## PRoT Resiliency

PRoT Resiliency covers the protection, detection, and recovery of the PRoT's
own firmware — the firmware running on the Platform Root of Trust itself.

The PRoT is the trust anchor for the entire platform. Its own firmware integrity
is therefore a prerequisite for everything else: if the PRoT firmware is
compromised, no downstream verification or recovery is trustworthy. PRoT
resiliency is established before any connected device is touched.

### Boot sequence and HROT verification

On power-on the PRoT executes its boot ROM (immutable, hardware-rooted) which
measures and authenticates the PRoT firmware stack. Once the PRoT firmware is
running, its first act is to verify the host-side Hardware Root of Trust (HROT)
— a separate security subsystem (e.g. Intel CSME, AMD PSP, or a dedicated
security IC) that the PRoT is responsible for appraising before the host is
allowed to boot.

The Resiliency Orchestrator coordinates this sequence:

```
Boot → Init → [HROT verification] → BootGate → OperationalPhase
```

If HROT verification fails at `Init`, the system enters `RotRecovery` before
proceeding. If recovery succeeds the system reboots cleanly; if it fails the
system enters `SystemLockdown`.

### Orchestrator

The Resiliency Orchestrator is the system-level control-plane function that
enforces consistent trust state across all domains — the HROT and any
host-side boot-managed firmware — covering the full lifecycle:

```
PowerOn → verify → [recover if needed] → release boot holds → runtime → attest
```

The orchestrator is structured in three layers:

1. **State machine (SM)** — pure policy logic. Defines what happens, in what
   order, and under what conditions. Never calls hardware directly.
2. **Platform contract** (`ResiliencyPlatform` trait) — abstracts
   target-specific side effects. Each hardware target provides its own
   implementation.
3. **Runner** — the per-target integration loop. Ingests events from driver
   services, feeds them into the SM one at a time, and dispatches the
   resulting effects through the platform impl.

**Security guarantees the orchestrator enforces:**
- No unauthorized firmware executes before it is verified.
- The system maintains a consistent and auditable trust state at all times.
- Recovery behavior is deterministic and bounded.
- Attestation reflects post-verification, post-recovery state.

### The Verifier

The Verifier is a RATS Verifier in the sense of RFC 9334: an entity that
appraises Evidence against Reference Values and Endorsements, producing
Attestation Results.

**Inputs:** Evidence from devices, pre-provisioned Reference Values
(CoRIM/CoMID), Endorsements, and an Appraisal Policy.

**Processing:** Compares Claims in Evidence against Reference Values, evaluates
Endorsements, and applies the Appraisal Policy.

**Output:** Attestation Results consumed by the Orchestrator SM as
`VerifyComplete { result }`.

Policy enforcement — deciding whether a firmware measurement is acceptable — is
the Verifier's responsibility, not the SM's. The SM acts only on the Verifier's
verdict.

The PRoT acts simultaneously as:
- A **local Verifier** — appraising Evidence from the HROT and host-side
  firmware domains behind itself.
- A **Lead Attester** — conveying Attestation Results upstream to a Layer N−1
  Verifier in the broader RATS topology.

For the complete orchestrator architecture, state hierarchy, extension points,
and vendor portability model, see `services/orchestrator/README.md`.

---

## Connected Device Resiliency

Connected Device Resiliency covers the protection, detection, and recovery of
the firmware of devices managed by the PRoT — the domains the orchestrator
controls. These are hardware components whose trust state the PRoT is
responsible for: verifying their firmware, holding them in reset until
verification passes, recovering them on failure, and releasing them when safe.

### Domains

A **domain** is a hardware component whose trust state the orchestrator is
responsible for. The current domain set is:

| Domain | Role |
|--------|------|
| `RoT` | The host-side Hardware Root of Trust (HROT) managed by the PRoT. Verified during `Init`; recovered via `RotRecovery` if verification fails. |
| `HostTarget` | The primary host boot path. Held in reset during `BootGate`; released when verification passes. |

The domain set is intentionally extensible. Additional boot-managed or
attestable domains (e.g. network controllers, storage controllers, accelerators)
can be added as new `BootTarget` variants without modifying the SM's state
transition logic.

### Boot-hold mechanism

The PRoT asserts a boot-hold signal (via a dedicated GPIO line) on each domain
at power-on, keeping it in reset. The orchestrator releases the hold only after
the Verifier has produced a passing Attestation Result for that domain. This
ensures no unauthorized firmware executes before verification is complete.

The hold is selective: on a fresh boot both domains are held; if the system
re-enters `BootGate` from a runtime watchdog timeout, only the domain that
timed out is re-held.

### Physical flash architectures

Connected device firmware resiliency depends on the physical flash arrangement
between the PRoT and the upstream device. Three legacy admissible architectures
are supported:

**Dual-Flash Side-by-Side (preferred):** The upstream device has direct access
to one of two physical SPI NOR Flash chips, selected by a PRoT-controlled mux.
Supports atomic A/B updates. Both chips must be the same part.

**Direct-Connect:** The PRoT acts as an SPI interposer between the upstream
device and a single SPI NOR Flash. Examines each SPI opcode and decides whether
to pass through, intercept, or block. Supports A/B updates by diverting reads
to either half of a double-sized flash.

**TPM:** Direct connection to a standard TPM-SPI interface. No backing SPI NOR
Flash required. Typically single-mode SPI only.

### Verification flow

On entry to `BootGate`, the orchestrator:

1. Asserts boot-hold on all domains selected by `hold_mask`.
2. Emits `StartVerification(BootTarget)` — the platform impl sends this to the
   Verifier service.
3. The Verifier appraises Evidence (firmware measurements) against Reference
   Values (CoRIM/CoMID) and Endorsements, applying the Appraisal Policy.
4. The Verifier pushes `VerifyComplete { result }` back to the orchestrator
   runner.
5. The SM transitions based on the result:

| Result | SM transition |
|--------|--------------|
| `Valid` (provisioned) | → `Runtime` (boot-hold released) |
| `Valid` (not provisioned) | → `Unprovisioned` (boot-hold released) |
| `Recoverable` | → `FirmwareRecovery` |
| `UpdatePending` | → `FirmwareUpdate` |
| `Fatal` | → `SystemLockdown` |

### Recovery flow

If the Verifier returns `Recoverable`, the SM enters `FirmwareRecovery`. The
platform impl applies a known-good firmware image (from the A/B flash
partition, the PRoT-hosted recovery image, or a PRoT-managed staging area
depending on the flash architecture). On completion the Verifier re-appraises
the recovered image:

- `RecoveryComplete (ok)` → `FirmwareVerify` (re-verify the recovered image)
- `RecoveryComplete (fail, retries left)` → `FirmwareRecovery` (retry)
- `RecoveryComplete (fail, no retries)` → `SystemLockdown`

Recovery attempt counting is tracked in the SM context (`recovery_attempts`)
and is bounded — the system cannot loop indefinitely.

### Update flow

If the Verifier returns `UpdatePending`, the SM enters `FirmwareUpdate`. The
platform impl applies the pending update (delivered via PLDM Type 5 firmware
update). On completion:

- `UpdateComplete (ok, not rot)` → `FirmwareVerify` (re-verify)
- `UpdateComplete (ok, rot_updated)` → `SystemReboot` (reboot to activate)
- `UpdateComplete (fail)` → `FirmwareRecovery`

Firmware updates are delivered via PLDM for Firmware Update (DSP0267 v1.3.0).
See `docs/src/specification/services/fwupdate.md` for the update package
format and PLDM protocol details.

### Runtime monitoring

Once in `OperationalPhase`, the orchestrator arms monitors (`ArmMonitors`) and
starts domain watchdog timers (`ArmWatchdog`). If a domain's watchdog expires
at runtime:

1. The SM exits `OperationalPhase` (disarming monitors and watchdog).
2. Re-enters `BootGate`, holding only the domain that timed out.
3. Re-runs the full verification and recovery flow for that domain.

This ensures that a domain that becomes unhealthy at runtime is re-verified
before it is allowed to resume operation.

### Seamless updates

For platforms that support seamless firmware updates (updates applied without
a full reboot), the SM supports a `SeamlessUpdate` sub-flow within
`OperationalPhase`:

```
Runtime → SeamlessUpdate → SeamlessVerify → Runtime (if valid)
                                          → FirmwareRecovery (if not valid)
```

The update is applied while the host continues to run. After application the
Verifier re-appraises the updated image. If verification fails, the SM falls
back to `FirmwareRecovery` to restore the previous known-good image.

### Attestation of connected devices

Connected device attestation follows the RATS model (RFC 9334). The PRoT acts
as an SPDM Requester, collecting Evidence from connected devices via SPDM
GET_MEASUREMENTS. The Local Verifier appraises this Evidence against CoRIM
reference values. The resulting Attestation Results are:

- Used locally by the orchestrator to gate boot-hold release.
- Conveyed upstream as part of platform composition attestation to an
  external Relying Party.

Attestation always reflects post-verification, post-recovery state —
the orchestrator does not attest to a domain until the SM has reached
`OperationalPhase`. See `docs/src/specification/services/attestation.md`
for the full attestation architecture.
