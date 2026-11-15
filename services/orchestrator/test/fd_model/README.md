<!-- Licensed under the Apache-2.0 license -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# fd model (`openprot_fd_model`)

The firmware-device side of the PLDM-FD-as-IPC-server design (#464), as an
executable model. In that design the PLDM Firmware Device (FD) serves the
orchestrator over IPC. The FD runs the update protocol with the Update Agent
(UA); at each phase it parks, raises a nudge, and waits for the orchestrator to
order or refuse the phase. This crate is that FD as a pure state machine:
`#![no_std]`, no dependencies, no kernel code, no clock, no heap.

It exists so the design's claims can be run instead of read. The host tests pin
the rules of the protocol. A QEMU service can wrap the model to test the
kernel-level claims: signals, one transaction per channel, deadlines.

The design is by Christina Quast (`chrysh`). This crate encodes that design as
written; it copies no code from it.

## Public seams

- `FdModel::new(FdConfig)` builds an FD. `FdConfig` sets how long each phase
  takes, the FD's window for the orchestrator's answer, the idle timer, and
  whether verify and apply succeed.
- `apply(op, generation, now)` takes an orchestrator order or refusal (`Op`) and
  answers at once, with a `Refusal` when the FD is in the wrong phase, the
  request came too late, or it named a stale generation.
- `query_status(now)` answers a status query. It clears the nudge, then reads the
  state. `status()` reads the state with no side effects.
- `nudge()` is the level the FD raises when it hands a state to the orchestrator.
  `nudges_raised()` and `nudges_merged()` count raises, and the raises that found
  it already up (each is a state the orchestrator may never see).
- `ua_offer`, `ua_activate`, `ua_cancel_update`, `ua_cancel_update_component` and
  `ua_security_revision` are the Update Agent's requests. `take_ua_message()`
  drains what the FD told the Update Agent, stamped with when it was due.
- `poll(now)` advances time, and `next_event_at()` says when the next change is
  due, so an event loop can sleep until then.

## Invariants

- **Operations belong to one phase each.** An operation in the wrong phase is
  refused and changes nothing.
- **The nudge is a level.** It stays up until a query is answered, however many
  changes happen meanwhile.
- **Silence is a refusal.** The FD keeps its own window for the orchestrator's
  answer and fails the phase to the Update Agent when it runs out.
- **A stored activation order is revocable** until the Update Agent asks for it,
  and a refusal that races the request loses.
- **Once apply has succeeded the FD refuses a cancel.**
- **Changes are stamped with when they were due,** not when a late caller noticed.
- **Nothing panics.** Time and counters saturate, and a clock that goes backwards
  leaves nothing due.

## Assumptions where the design is silent

Writing the model forced a choice wherever #464 says nothing. The seven choices
are listed in the crate documentation (`src/lib.rs`), and each is a question for
the design.

## Host tests

```sh
bazel test //services/orchestrator/test/fd_model:fd_model_test
```

Each test names the sentence of the design it pins.
