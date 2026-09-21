# orchestrator: treat DeviceFatal boot failures differently from timeouts

Follow-up to #471.

Since #471, a failed boot walk comes in as `Event::BootFailed { id, checkpoint, kind }`,
where `kind` tells us *why* it failed: the device just never checked in
(`TimedOut`), it reported a failure worth trying again (`DeviceRetriable`),
or it reported a failure that a retry can't fix (`DeviceFatal`).

Right now the state machine ignores that distinction. `BootFailed` and the
older, plainer `Timeout(id)` event both hit the exact same code path and get
treated exactly the same way — every failure burns through the same retry
budget before we give up on the component.

That's not unsafe (we never let anything boot unverified, no matter what),
but it's wasteful: if a device tells us straight out "this image is bad,
trying again won't help," we still make it fail `max_retry` times the same
as a component that just had a slow boot, before we finally isolate it or
lock down. We're ignoring information the device is handing us for free.

Also, the `checkpoint` name (which checkpoint it died at — bl1, kernel,
etc.) gets passed along in the event but nobody reads it. It's not logged
or surfaced anywhere yet.

## Where the three `kind`s actually come from

Each poll of the boot walk (`CheckpointWalk::poll`, in
`services/orchestrator/adapters/walk/src/walk.rs`) checks two things, in
order, for whichever checkpoint it's currently waiting on:

1. **Did the window run out?** If so, that's `TimedOut` — and it has
   nothing to do with what the device says. The device could be frozen,
   powered off, or just slow. We simply gave up waiting. Nobody told us
   anything; this is silence.
2. **If there's still time, what does the device say?** We probe it (a
   GPIO pin, a progress register, a heartbeat — whatever the board wired
   up), and it can answer "still booting," "this step passed," or actively
   report a failure of its own:
   - `FailedRetriable` → the device is saying "I hit a problem, but it's
     the kind that might not happen again" (e.g. a transient self-test
     miss). The walk stops early — before the deadline — with `kind:
     DeviceRetriable`.
   - `FailedFatal` → the device is saying "I hit a problem, and running the
     same image again isn't going to fix it" (e.g. a corrupt image). Same
     early stop, `kind: DeviceFatal`.

So the three kinds really represent three different *sources* of bad news:

| Kind | Who said so | What it means |
|---|---|---|
| `TimedOut` | Nobody — silence | Ran out of patience, no evidence either way |
| `DeviceRetriable` | The device itself | "I failed, but retry me" |
| `DeviceFatal` | The device itself | "I failed, and retrying won't help" |

That distinction is exactly what the state machine throws away today by
handling all three the same way.

## What we should do

- When a `DeviceFatal` failure comes in, skip the retry loop and go straight
  to the isolate/cascade/lockdown decision instead of waiting out the full
  retry cap.
- Log or report the `checkpoint` name somewhere so a failure is easier to
  diagnose (right now you only know *that* something failed, not *where*).

## What we're NOT doing here

- Not touching the Isolable/Cascading/Required policy itself, just how fast
  we get to it for a DeviceFatal failure.
- Not touching the older fleet-wide `Timeout` watchdog — that one's still
  not even wired up to a real run loop yet, separate problem.

This was called out as a known gap in #471 itself — the PR added the richer
event on purpose but left "actually use it to change behavior" for later.
