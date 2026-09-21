# Boot watchdog on the OS timer — requirements review

Does the boot watchdog built on the OS monotonic timer (`TimerManager` /
`Watchdogs`, driven by `SystemClock` + deadline-bounded `object_wait`) meet the
orchestrator's requirements?

**Yes**, with two config/integration caveats named below.

## Contract

The orchestrator-sm owns no clock. It only *names* watchdog behavior as `Event`s
and relies on the runtime to arm/cancel the timers and inject the resulting
events.

| orchestrator-sm requirement | provided by |
|---|---|
| Per-component boot-progress watchdog → `Event::Timeout(id)`, armed on `ReleaseReset`, cleared on `ComponentReady`/`Booted` (`Event::Timeout` doc, `sm/src/model.rs`) | `arm_boot(id, after)` / `cancel_boot(id)` / `poll → Timeout(id)` |
| Single commit watchdog → `Event::CommitTimeout`, armed on `ActivateUpdate`, cancelled on `CommitSvnFloor` (`Event::CommitTimeout` doc) | `arm_commit` / `cancel_commit` / `poll → CommitTimeout` |
| Stale/late fire must be harmless — the sm drops a `Timeout` for a non-awaiting component and a `CommitTimeout` with nothing pending | best-effort cancel; a late fire degrades to a dropped event, never a false recovery (`timer/src/lib.rs`) |
| Re-arm on recovery re-release restarts the window | `arm_boot` replaces rather than stacks |
| Up to N components awaiting boot at once (speculative Passive walk) | `heapless::Vec<_, N>`, N = chain length |
| Configured per-component boot timeout, device-agnostic | caller supplies the window via `after: Duration`; timer stays policy-free |
| One deadline for the runtime's deadline-bounded `object_wait` | `wait_deadline()` returns the nearest (`server/src/runtime.rs`) |

## Correctness fit

- Uses the **monotonic** SysTick clock (`SystemClock::now`) — the right clock for
  a watchdog; no wall-clock jumps or regressions.
- Overflow saturates to `Instant::MAX` = "wait forever" rather than firing early.
- Validated on-target in QEMU:
  `//target/ast10x0/tests/orchestrator/timer:timer_qemu_test`.

## Caveats

1. **N is a hard invariant, not a soft one.** `arm_boot` silently drops the arm
   when the buffer is full. If N is ever configured below the true max
   concurrent-awaiting count, that component gets *no* timeout — a liveness gap,
   not a false recovery. The requirement holds only while
   `N ≥ max components awaiting boot simultaneously`.

2. **Tick-resolution (soft), not hard real-time.** The timeout fires when
   `object_wait` returns at/after the deadline and the loop next drains
   `poll_expired`. Fine for boot-progress windows (ms–s); it is an approximate
   upper bound, not a cycle-accurate deadline.

## Open item

The remaining work is **wiring, not the mechanism**: the run loop must translate
the sm's effect trace into `arm_boot` / `cancel_boot` / `arm_commit` /
`cancel_commit` (and re-arm across a recovery re-walk). The `Watchdogs` primitive
already exposes exactly those operations; that effect→timer glue is not in a
dispatch loop yet.
