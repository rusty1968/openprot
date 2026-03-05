# Executor Design

## Overview

The async runtime consists of two cooperating layers:

- **Executor** (`executor/`) — task scheduler. Polls ready tasks, manages wakers, and calls an idle strategy when no work is pending.
- **Reactor** (`reactor/`) — kernel I/O multiplexer. Registers kernel object handles into a WaitGroup and blocks until one fires, then wakes the corresponding task.

Together they implement the classic reactor pattern on top of pw-kernel's `object_wait` / WaitGroup syscall interface.

```
┌─────────────────────────────────┐        ┌──────────────────────────┐
│         User Mode (U-mode)      │        │   Kernel Mode (M-mode)   │
│                                 │        │                          │
│  Applications                   │        │  Kernel Objects          │
│    ↓            ↓               │        │  (IPC, IRQ, Timer)       │
│  Service APIs  Async Executor   │        │    ↓                     │
│    ↓            ↓  (WaitGroup)  │        │  Drivers                 │
│  Syscall Interface ─────────────┼──────→ │    ↓                     │
│                          ecall  │        │  VeeR RISC-V Core        │
└─────────────────────────────────┘        └──────────────────────────┘
```

| Aspect | Tock | pw-kernel (this design) |
|--------|------|------------------------|
| Async mechanism | `yield_wait` suspends; kernel calls `upcall` to resume | `object_wait` ecall blocks until WaitGroup fires; no upcall |
| Kernel extension model | Capsules — in-kernel Rust objects implementing the `Driver` trait; addressed by driver number in the fixed `command`/`allow`/`subscribe` syscall ABI | Capability handles — kernel creates typed objects (IPC channels, IRQ, Timers) and distributes unforgeable handles to tasks; userspace addresses objects only by handle, never by name |
| Event delivery | Kernel pushes result via upcall (callback into userspace) | Kernel pulls: userspace blocks on WaitGroup, kernel signals membership |
| Async runtime coupling | Tock libtock-rs provides `yield_wait` future; executor is library-provided | Reactor wraps `object_wait` / WaitGroup; executor is independent of kernel ABI |
| Notification direction | Bidirectional (ecall down, upcall up) | Unidirectional (ecall down, blocking return up) |
| Multiplexed wait | Not native — requires one syscall per object | WaitGroup: single `object_wait` covers up to 16 objects |

---

## System Diagram

```
┌──────────────────────────────────────────────────────────────┐
│                     Userspace (U-mode)                       │
│                                                              │
│  ┌────────────┐  ┌────────────┐  ┌──────────────────────┐   │
│  │  Task A    │  │  Task B    │  │  Task C              │   │
│  │ (SPDM resp)│  │ (MCTP recv)│  │ (periodic timer)     │   │
│  └─────┬──────┘  └─────┬──────┘  └──────────┬───────────┘   │
│        │ .await         │ .await              │ .await        │
│  ┌─────▼────────────────▼─────────────────────▼───────────┐  │
│  │                       Reactor                          │  │
│  │  object_wait futures → WaitGroup slots (MAX 16)        │  │
│  │  Waker stored per slot                                 │  │
│  │  wait_for_events() blocks on WaitGroup                 │  │
│  └────────────────────────────┬────────────────────────────┘  │
│                               │ idle closure                 │
│  ┌────────────────────────────▼────────────────────────────┐  │
│  │                       Executor                          │  │
│  │  Embassy raw::Executor                                  │  │
│  │  SIGNAL_WORK atomic ◄── __pender() ◄── waker.wake()    │  │
│  │  poll → check SIGNAL_WORK → idle or loop                │  │
│  └────────────────────────────┬────────────────────────────┘  │
│                               │ syscalls                     │
│  ┌────────────────────────────▼────────────────────────────┐  │
│  │           Kernel trait (pw-kernel bridge)               │  │
│  │  object_wait()  wait_group_add/remove()  interrupt_ack()│  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
                          │ ecall / syscall
┌─────────────────────────▼────────────────────────────────────┐
│                     pw-kernel (M-mode)                       │
│  WaitGroup → kernel objects (IPC endpoints, IRQs, timers)   │
│  Drivers   → VeeR RISC-V core / hardware                    │
└──────────────────────────────────────────────────────────────┘
```

---

## Executor

### Key Types

| Type | Location | Purpose |
|------|----------|---------|
| `Executor` | `executor/src/lib.rs` | Wrapper around Embassy `raw::Executor` |
| `SIGNAL_WORK` | `executor/src/lib.rs` | Global `AtomicBool`; set by `__pender`, cleared by poll loop |
| `__pender` | `executor/src/lib.rs` | Embassy callback invoked when any task is woken |
| `YieldOnce` | `executor/src/lib.rs` | Future that yields once then completes |

### Construction

`Executor::new()` is a runtime constructor — `embassy_executor::raw::Executor::new()` is not
`const` in version 0.9.x. Two construction paths exist:

1. **`start_async(init)`** — allocates the executor on the stack, `transmute`s it to `&'static`,
   and calls `run()` with a spin-loop idle. Diverges (`-> !`). Use for simple entry points.
2. **`Executor::new()` + `run()`** — construct explicitly and supply a custom idle closure.
   Use this when wiring up the reactor.

The `!Send` marker (`PhantomData<*mut ()>`) prevents the executor from being sent across threads,
reinforcing the single-threaded invariant.

### Poll Loop

`Executor::run()` implements the following loop:

```
init(spawner)               // spawn initial tasks

loop {
    raw::poll()             // poll all ready tasks (Embassy)

    critical_section {
        if SIGNAL_WORK {    // a waker fired during poll?
            SIGNAL_WORK = false
            // loop immediately — more work available
        } else {
            idle()          // no work — block via reactor or spin
        }
    }
}
```

The check-and-clear inside a `critical_section` prevents the race where:
1. `poll()` finishes with no ready tasks
2. An interrupt fires and sets `SIGNAL_WORK`
3. `idle()` is called, blocking forever

On `riscv32imc` (no atomics extension) `AtomicBool::swap` is unavailable; the load + store pair
inside the critical section is used instead.

### Singleton Enforcement

There is exactly one `__pender` symbol per binary (enforced by `#[export_name]`). Two concurrent
executor instances would share `SIGNAL_WORK` and steal each other's wake signals. Structural
enforcement:

- `start_async` diverges — the call site can never construct a second instance.
- `!Send` prevents the executor from being moved off the initial thread.
- No global `static` instance is provided; users who need one must use `static mut` with `unsafe`
  and accept the single-instance responsibility explicitly.

---

## Reactor

### Key Types

| Type / Const | Location | Purpose |
|---|---|---|
| `Reactor<K>` | `reactor/src/lib.rs` | Multiplexes up to 16 kernel objects via WaitGroup |
| `MAX_REACTOR_SLOTS` | `reactor/src/lib.rs` | 16 — bitmask fits in `u16` |
| `Kernel` trait | `reactor/src/lib.rs` | Abstraction over pw-kernel syscalls |
| `Signals` | `reactor/src/lib.rs` | 32-bit readiness bitmask |
| `WaitReturn` | `reactor/src/lib.rs` | Return value from `object_wait`: `user_data` + `pending_signals` |

### Registration Flow

When a task `.await`s a kernel object future:

1. `Future::poll` calls `reactor.register(handle, signals, waker)`.
2. `register` finds the lowest free bit in `used: u16`.
3. `K::wait_group_add(wg_handle, handle, signals, slot)` — kernel adds the object to the WaitGroup.
4. The `Waker` is stored in `wakers[slot]` (inside `UnsafeCell<Option<Waker>>`).
5. The slot bit is set in `used` and the slot index is returned.

On subsequent polls (waker may have changed), `update_waker(slot, waker)` refreshes the stored
waker without re-registering with the kernel.

On cancel or completion, `deregister(slot, handle)` calls `K::wait_group_remove` and clears the
slot.

### `wait_for_events()`

Called from the executor's idle closure when no tasks are ready:

```
if used == 0 → spin_loop hint (no objects registered)

object_wait(wg_handle, Signals::READABLE, Deadline::MAX)
  Ok(wr) → wakers[wr.user_data].wake_by_ref()
  Err(_) → wake all registered wakers (spurious wakeup / error recovery)
```

The `user_data` field in `WaitReturn` is the slot index, mapping the kernel's readiness
notification directly to the correct `Waker`.

### Capacity

`MAX_REACTOR_SLOTS = 16` fits in a `u16` bitmask. This bounds the number of concurrently
in-flight kernel object waits. For caliptra-mcu-sw the active set is small (IPC endpoints for
SPDM, MCTP, telemetry, storage — well under 16).

### Safety Model

Interior mutability via `UnsafeCell` is sound because:
- Single-threaded execution (`!Send` executor, cooperative scheduling)
- `poll()` is never re-entered (the pender only sets a flag)
- `wait_for_events()` is only called from the idle branch — never concurrently with `poll()`

The `unsafe impl Sync for Reactor<K>` is required to place the reactor in a `static`. The
implementation comment documents the invariant.

---

## End-to-End Data Flow

The sequence for an IPC receive future resolving:

```
1. Task polls IPC future → Poll::Pending
   reactor.register(ipc_handle, READABLE, waker) → slot N

2. Executor: no ready tasks → calls idle closure

3. idle closure calls reactor.wait_for_events()
   → object_wait(wg_handle, ..., MAX) blocks

4. pw-kernel: IPC message arrives, WaitGroup fires with user_data=N

5. object_wait returns WaitReturn { user_data: N, pending_signals: READABLE }

6. reactor.wakers[N].wake_by_ref()
   → __pender() called → SIGNAL_WORK = true

7. wait_for_events() returns

8. Executor: critical_section sees SIGNAL_WORK=true, clears it, loops

9. raw::poll() re-polls the task

10. Task polls IPC future → register returns Pending but this time
    object_wait(ipc_handle, READABLE, MIN) returns Ok immediately
    → Poll::Ready(Ok(payload))

11. reactor.deregister(N, ipc_handle)
```

---

## Kernel Trait Implementations

| Target | Crate / Module | Notes |
|--------|----------------|-------|
| pw-kernel (production) | `kernel_bridge/` | Wraps pw-kernel userspace syscall ABI |
| Bare-metal mock | `platform/impls/baremetal/mock/` | Stub for unit tests |
| QEMU async e2e | `target/qemu_virt_riscv32/async_e2e/` | Integration test harness |

The `Kernel` trait requires only four methods: `object_wait`, `wait_group_add`,
`wait_group_remove`, and `interrupt_ack`. Porting to a new kernel means implementing
those four functions.

---

## Idle Strategy Summary

| Strategy | Code | When to use |
|----------|------|-------------|
| Spin | `\|\| core::hint::spin_loop()` | Testing, tasks that self-wake via `YieldOnce` |
| Reactor | `\|\| reactor.wait_for_events()` | Production — blocks until kernel I/O |
| Single object | `\|\| K::object_wait(h, s, MAX)` | Simple single-object blocking |

---

## Requirement Traceability

| Requirement | How it is met |
|-------------|---------------|
| EXEC-1: Single executor instance | `__pender` is unique per binary; `start_async` diverges preventing re-entry |
| EXEC-2: Executor lifecycle | `start_async` (create + run forever); `Spawner` for dynamic spawn |
| EXEC-3: Poll loop | `run()` loop: poll → check `SIGNAL_WORK` → idle |
| EXEC-4: Waker integration | `__pender` sets `SIGNAL_WORK`; critical_section prevents race |
| EXEC-5: Task spawning | `Spawner: Copy` (Embassy); tasks hold copies freely |
| EXEC-6: Bounded task count | Embassy arena allocator; compile-time task count |
| KERN-1: Pluggable kernel backend | `Kernel` trait; zero runtime penalty (static dispatch) |
| KERN-2: Required kernel capabilities | `object_wait`, `wait_group_add/remove`, `interrupt_ack` |
| PLAT-1: `no_std` | `#![no_std]` in both crates; no heap |
| PLAT-2: Single-threaded | `!Send` executor; `unsafe impl Sync` with documented invariant |
| PLAT-3: RISC-V 32-bit | `portable_atomic` for `AtomicBool` on `riscv32imc`; critical_section HAL |
| PLAT-4: Cooperative scheduling | Embassy poll loop; tasks yield at `.await` |
| PLAT-5: Constrained resources | Fixed-size `[UnsafeCell<Option<Waker>>; 16]`; no `Vec`/`Box`/`HashMap` |
