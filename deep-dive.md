# Deep dive: what the orchestrator integration test actually exercises

Review of `target/ast10x0/tests/orchestrator/runtime/main.rs` (5 scenarios, run in
QEMU under the real kernel). The question this doc answers: **are we sure it
exercises all the components?** — read two ways:

1. the **four subcomponents** wired together (the pure core `orchestrator-sm`,
   the server runtime `orchestrator-server`/`BootWatchdogs`, the watchdog keeper
   `orchestrator-timer`/`TimerManager`, and the board device table
   `orchestrator-config`/`DeviceConfig`), and
2. the **supervised component slots** in the chain (`C0`, `C1`).

Verdict up front: **yes for the wiring and both slots along the
boot / timeout / commit-timeout paths**, with explicit gaps called out in
§5 so the claim is not overstated.

---

## 1. The four subcomponents are all on the live path

Every scenario runs the same real seam, not mocks of it:

```
DeviceConfig (window)  ──►  BootWatchdogs.arm_boot(relative)  ──►  TimerManager (absolute deadline)
                                     │                                      │
      object_wait(wait_deadline()) ◄─┘                                      │
                                     │                                      ▼
                       poll_expired() ──► Event ──► Orchestrator.dispatch ──► State/Effect
```

- **config → runtime**: boot windows are *not* hardcoded in the shell. They come
  from `SOC.checkpoints()[k].timeout()` (a `core::time::Duration` from the device
  table), converted by `window()` and handed to `BootWatchdogs::arm_boot`. If the
  table were empty or malformed, `DeviceConfig::new`/`BootCheckpoint::new` would
  panic at const-eval — so the table is genuinely constructed and read.
- **runtime → timer**: `BootWatchdogs` owns a `TimerManager<Instant, ComponentId, N>`.
  Every `arm_boot`/`cancel_boot`/`arm_commit`/`wait_deadline`/`poll_expired` call
  is a real call into `TimerManager`, against the kernel `SystemClock`.
- **timer → kernel**: `wait_deadline()` is the *actual* argument to the
  `syscall::object_wait` the test blocks on; expiries are produced by the real
  monotonic clock passing the armed deadline, not by a simulated tick.
- **runtime → sm**: `poll_expired()` returns the exact `Event` value fed into
  `Orchestrator::dispatch`; the shell does no event mapping of its own.

So the integration is real end to end: a window declared in the device table
becomes a kernel deadline and, on lapse, an orchestrator state transition.

## 2. Per-scenario trace

| # | Scenario | Drives | Proves |
|---|----------|--------|--------|
| 1 | `scenario_checkpoint_confirmed` | C0 walks `bl1`→`kernel`, re-armed each checkpoint via the runtime, then confirms | inner checkpoint walk boots; `Booted` keeps core `Ready`; a **stale `Timeout` after cancel is a no-op** |
| 2 | `scenario_checkpoint_timeout` | C0 never signals; first window lapses | runtime surfaces `Timeout(C0)`; core → `Recovering(C0)` |
| 3 | `scenario_chain_all_confirm` | C0+C1 both armed, both report in | two independent watchdogs; both `Booted`; core stays `Ready` |
| 4 | `scenario_chain_one_timeout` | C0 confirms, C1 goes quiet | **nearest-of-many deadline is C1's; `poll_expired` attributes `Timeout(C1)`**; core → `Recovering(C1)` (correct id) |
| 5 | `scenario_commit_timeout` | commit watchdog armed, left to lapse | `Expired::Commit` → `Event::CommitTimeout`; one-shot (does not fire twice) |

## 3. Supervised component-slot coverage (C0 / C1)

| Slot | Released | Boot watchdog armed | Confirmed (`Booted`) | Timed out (`Timeout`) | Recovered |
|------|:---:|:---:|:---:|:---:|:---:|
| C0 | 1,2,3,4 | 1,2,3,4 | 1,3,4 | 2 | 2 |
| C1 | 3,4 | 3,4 | 3 | 4 | 4 |

Both slots reach every boot-supervision outcome across the suite: released,
armed, confirmed, timed out, and recovered. Multi-component behaviour (two live
deadlines, correct-id attribution) is covered by scenarios 3–4.

## 4. Subcomponent API surface actually hit

**`orchestrator-config`** — `DeviceConfig::new`, `BootCheckpoint::new`,
`.checkpoints()`, `.timeout()`. ✔ construction + read path.

**`orchestrator-server` (`BootWatchdogs`)** — `new`, `arm_boot`, `cancel_boot`,
`arm_commit`, `wait_deadline`, `poll_expired`. ✔ full boot+commit run loop.

**`orchestrator-timer` (`TimerManager`)** — reached transitively: `arm_boot`,
`cancel_boot`, `arm_commit`, `next_deadline`, `poll`; both `Expired::Boot` and
`Expired::Commit` produced. ✔ ordering/tie-break/one-shot via the deadline race.

**`orchestrator-sm` (`Orchestrator`)** — `Chain` `TryFrom` validation,
`ComponentAttrs::passive_required`, `dispatch`, `state`. Events consumed:
`PowerGood`, `VerificationPassed`, `Booted`, `Timeout`. States reached:
`PowerOnReset` → `PreSupervision` → `Ready`, and `Recovering(id)`. Effect
observed: `ReleaseReset(id)`.

## 5. Gaps — what is NOT exercised (so the claim stays honest)

These are out of scope for this test and covered elsewhere or not yet:

- **Active-tier components.** Both slots are `passive_required`; the
  `Event::ComponentReady` / `State::AwaitingReady(Some(id))` active-readiness path
  is not driven. Only the passive `Booted` tier is.
- **CommitTimeout into the core.** Scenario 5 asserts the *runtime* surfaces
  `Event::CommitTimeout`, but it is **not dispatched into `Orchestrator`**, so the
  core's commit-timeout → `State::Locked` handling is not exercised here.
- **Update lifecycle.** `UpdateRequest`/`UpdateVerified`/`UpdateRejected`/
  `BootConfirmed`, `State::Updating`, and the `ActivateUpdate`/`CommitSvnFloor`
  effects are untouched.
- **Fail-closed paths.** `Event::EffectFailed` and `State::Locked` are never
  reached; `FakePlatform::execute` always returns `Ok`.
- **Unused runtime/timer API.** `BootWatchdogs::cancel_commit` and the `Default`
  impl are not called; the `Full` capacity error is mapped but never triggered
  (`N = 4`, at most 2 watchdogs armed).
- **Effect inspection.** Only `ReleaseReset` is asserted; other emitted effects
  are accepted without inspection.

## 6. Conclusion

The test **is** a true four-subcomponent integration test: config windows flow
through the server runtime and timer keeper onto the kernel `object_wait`
deadline and back into the sm core, for both component slots, across confirm /
timeout / multi-component / commit-timeout paths. It is **not** a full
state-machine coverage suite — the update, lock, active-tier, and
effect-failure paths are deliberately left to the `orchestrator-sm` unit tests.
Within its stated scope (the runtime binding and boot supervision), coverage is
complete; the §5 gaps are the honest boundary of that scope.
