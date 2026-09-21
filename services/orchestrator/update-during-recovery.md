# Update request during recovery

## Question

While the orchestrator is recovering device **A** (`State::Recovering(A)`), the
platform receives a firmware-update request — say for device **B**. What does
the reducer do with that `UpdateRequest`, and is that the behavior we want?

## Short answer

The `UpdateRequest` is **silently dropped**. Recovery is not interrupted, no
effect is emitted, and nothing is queued. The requester gets no response at all.

That drop is **deliberate as a policy** (recovery outranks update — "fix first,
then update"), but the *silence* is the part worth a second look: the requester
cannot tell "refused, busy recovering" apart from "lost."

## What the state machine does today

`Event::UpdateRequest` carries **no `ComponentId`**
(`sm/src/model.rs`) — at this layer an update request is not a per-device event.
It only means "someone wants to start an update," and it is only actionable from
`Ready`:

```text
State::Ready:      UpdateRequest -> Transition(State::Updating)
State::Updating:   (handles UpdateVerified / UpdateRejected / CorruptionDetected)
State::Recovering: UpdateRequest -> _ => Outcome::Super
```

In `Recovering`, `UpdateRequest` hits the catch-all `_ => Outcome::Super`. It
then bubbles to `handle_supervising`, which does **not** handle `UpdateRequest`
either, so it falls through to `Super` again. At the top level an unhandled
`Super` is a **no-op**: the state is unchanged and no effect is emitted
(`sm/src/lib.rs`). Net result: the request evaporates.

## The asymmetry is intentional

The machine is deliberately asymmetric about preemption:

- **Corruption can preempt an update.** A `CorruptionDetected` for a `Required`
  component while `Updating` transitions to `Recovering` (and discards the
  staged image). Integrity wins over an in-flight update.
- **An update can never preempt recovery.** There is no path from `Recovering`
  back into `Updating`. Recovery runs to completion (or to `Locked`) first.

This is a single-flight, recovery-priority design: the RoT finishes making the
platform whole before it entertains a new update. That ordering is sound and
defensible.

## The concern: silent drop, no acknowledgement

The problem is not the ordering — it is that the dropped request produces **no
observable result**:

- No effect is emitted, so there is no "busy, try later" signal on the wire.
- The request is not queued, so it will not be retried automatically once
  recovery finishes.
- The requester cannot distinguish a deliberate refusal from a lost message.

For a fire-and-forget internal event this is fine. For a host- or
management-initiated firmware update it is thin: the caller is left guessing.

## CSA alignment

The Composable Security Architecture resiliency chapter
(`RoT_architecture/docs/composable_security_architecture/src/resiliency/README.md`)
does not settle the ordering question directly, but it frames the expectations:

1. **Updates are request/response.** The normative "Authenticated Firmware
   Update" sequence has the RTU reply to the host on every path — *reject*,
   *write error*, or *update accepted*. The CSA models an update as something
   that gets answered, not silently swallowed.

2. **The normative wire protocol has a "busy/retry" mechanism.** The chapter
   names DMTF PLDM for Firmware Update (DSP0267, Type 5) as the normative
   over-the-wire protocol. PLDM already defines completion codes / flow control,
   so a "not ready, retry later" response exists **at the protocol layer** — not
   in the core reducer.

3. **The CSA does not mandate update-vs-recovery arbitration.** Recovery is
   described as autonomous and separate from update; the interaction of an
   in-flight recovery with an incoming update is left to the implementation. Our
   single-flight, recovery-priority choice is therefore a legitimate
   implementation-defined policy, neither required nor forbidden by the CSA.

4. **Degraded mode requires reporting.** The one requester-facing obligation the
   CSA does state on the failure side is that the eRoT must *report* failures
   through the platform management interface. It reinforces the theme: keep
   management software informed rather than fail silently.

## Conclusion / recommendation

- **Keep the core state machine as-is.** Single-flight with recovery priority is
  the right policy, and it is CSA-compatible. The reducer should stay a pure,
  effect-emitting model of that policy.
- **Add the acknowledgement above the reducer, not inside it.** A refused
  `UpdateRequest` during recovery should produce a "busy — recovery in progress,
  retry later" response in the PLDM responder / driver layer (a PLDM Type 5
  completion code), so the requester gets a definite answer. This belongs in the
  driver, not in a new state or transition in the machine.

In short: the *drop* is correct; the *silence* is what to fix, and it belongs in
the driver that speaks PLDM, not in the reducer.

## Related

- `concurrent-corruption-during-update-or-recovery.md` — the mirror-image
  direction (inbound *corruption* while updating/recovering), which surfaces real
  gaps for `Required` components under the single-slot `Recovering(ComponentId)`
  payload.
