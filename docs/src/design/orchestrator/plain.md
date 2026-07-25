# The Orchestrator in Plain Language

This document explains how the orchestrator state machine works using plain
language and analogies. Nothing here is normative — for the precise rules, see
the [State Machine reference](./orchestrator-machine.md).

---

## The job

The eRoT is the first thing that powers on. Its job is to make sure nothing else
runs unless it has been checked. Think of it as a customs officer at an airport:
every passenger (platform component) must pass through the checkpoint before they
are allowed through.

---

## Two zones, never both at once

The machine is always in one of two zones.

**The checkpoint** — `PreSupervision`. The eRoT is standing at the gate,
checking documents one by one. It is focused entirely on the queue. It is not
answering questions from people already through — that is not its job right now.

**Airside** — `SupervisingPlatform`. At least one decision has been made about
the queue: a passenger was cleared, or one was flagged for a problem. The eRoT
is now managing the airside area: answering challenges from auditors, responding
to incidents. This is where most of the machine's life is spent.

Once the eRoT steps away from the checkpoint and into the airside role, it does
not go back — unless a recovery incident forces a full evacuation and re-check
(see below).

---

## Walking the queue

At the checkpoint the eRoT processes the queue one at a time.

- **Symbiont component** (a NIC, a storage controller — no root of trust of its
  own): the eRoT checks its firmware signature, clears it through, and
  immediately calls the next person in the queue. The eRoT *stays at the
  checkpoint* — this is the self-loop in the diagram.

- **Active component** (a BMC or CPU with its own security processor): the eRoT
  checks the firmware signature and clears it through, but this passenger also
  has to clear their own internal check before they are truly settled. The eRoT
  moves them to a holding gate (`AwaitingReady`) and waits for the component's
  own root of trust to report in. At this point the eRoT has stepped airside —
  supervision has started — even though the rest of the queue is still waiting.

---

## Why `Recovering` is airside

This is the part that surprises people.

Suppose a passenger is flagged at the checkpoint — their documents fail. The eRoT
moves them to a holding area and starts recovery: trying to restore a known-good
image and re-check. During this time:

- Auditors are still walking the airside area: *"Prove to me that this platform
  is in a known state."* The eRoT must still answer.
- A second component might corrupt while the first is being fixed. The eRoT must
  still act on it.

The supervision contract — *always answer attestation challenges, always act on
corruption* — cannot have a gap. So `Recovering` sits inside `SupervisingPlatform`:
the eRoT is dealing with a problem, but it is still on duty.

Crucially, this means the eRoT can step airside *before anyone is actually
through the checkpoint*. If the very first passenger in the queue fails, the eRoT
immediately enters recovery — and is therefore in `SupervisingPlatform` — even
though zero components have been released. The supervision contract starts the
moment `PreSupervision` exits, for any reason.

---

## The evacuation

There is one moment when supervision is explicitly suspended.

When recovery has done what it can — restored a golden image — the eRoT must
re-verify the whole chain from scratch. To do that it gates *all* components back
into reset: the equivalent of evacuating the building and locking the doors. It
then walks back to the checkpoint and starts the queue from the top.

During an evacuation the eRoT does not answer auditors. It is busy re-checking
credentials. Once the first component is cleared through again, the eRoT steps
back airside and the supervision contract resumes.

In state-machine terms: `Recovering → PreSupervision` is the evacuation;
the next `PreSupervision` exit re-enters `SupervisingPlatform`.

---

## Terminal lockdown

If a component cannot be recovered after all retries, the machine emits
`RecoveryFailed` and transitions to `Locked`. The building is evacuated and
the doors are physically bolted. Every component is held in reset permanently.
No further event has any effect. The only way out is out-of-band intervention.

The same fate applies if the eRoT itself fails its self-check at power-on: there
is no point running a checkpoint if the officer cannot be trusted.

---

## One-line summary of each state

| State | Plain meaning |
|---|---|
| `PowerOnReset` | Waiting to find out if the officer passed their own check |
| `PreSupervision` | Standing at the checkpoint, working through the queue |
| `AwaitingReady` | A passenger is through the eRoT gate but still clearing their own internal check |
| `Ready` | Everyone is through; the platform is up |
| `Updating` | A passenger is swapping to a new version of their documents |
| `Recovering` | A passenger failed; attempting to restore their documents before re-checking |
| `Locked` | Building evacuated, doors bolted, no further admittance |
