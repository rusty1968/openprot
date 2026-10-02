# RISC-V `Arch::context_switch` has no interrupt-context guard, and the path is untested

## Summary

`pw_kernel`'s RISC-V `Arch::context_switch` performs its `ret`-based switch
unconditionally, with no check for interrupt context — unlike the Cortex-M
port, which defers to PendSV. If it is ever reached from a trap handler it
never returns through the interrupt's trap frame and never releases the
scheduler lock.

**It could not be reached from a trap handler in testing.** Eight emulator
runs across three wake paths, two pigweed pins, and patched/unpatched kernels
produced no failure: the generic scheduler defers interrupt-context switches
before the arch layer is called (see *Reproduction: not achieved*). The
defects below are therefore a latent hazard, not a demonstrated live failure.

What is demonstrated is the **coverage gap** — no test on any RISC-V target
exercises interrupt-initiated context switching, so if the layer above ever
stops absorbing this, nothing would catch it.

---

## Part 1 — The defects

### 1a. `Arch::context_switch` performs the switch immediately, even from a trap handler

`pw_kernel/arch/riscv/threads.rs:152` (`Arch::context_switch`) unconditionally
performs the `ret`-based context switch `riscv_context_switch` — save
callee-saved registers, swap `sp`, `ret`.

That sequence is only valid from an ordinary function-call context, where the
compiler has saved caller-saved registers around the call site and no
unrelated RAII guards are live on the stack. Neither holds inside
`trap_handler` (`pw_kernel/arch/riscv/exceptions.rs:164`). When an interrupt
handler wakes a higher-priority thread, switching there:

- **never returns through the interrupt's own trap frame** — the `TrapFrame`
  pushed on entry is the only record of what the interrupted thread was doing,
  and returning from the trap means unwinding back through the entry stub to
  `csrw mepc` and `mret`. `riscv_context_switch` ends in `ret` instead, so the
  interrupted PC in `mepc` is dropped, `mstatus.MIE` is never restored, and
  the caller-saved registers the frame exists to protect (`a0`-`a7`,
  `t0`-`t6`) are never reloaded; and
- **never releases the locks the handler's call chain holds**, notably the
  scheduler lock — those are RAII guards that release on drop, and
  `riscv_context_switch` ends in `ret` to the incoming thread, so the outgoing
  stack is frozen mid-call and the destructors never run.

Were this reached, the damage would surface later — when the abandoned thread
resumes through a corrupted return path — rather than at the switch itself.
No such failure was observed in testing; see *Reproduction: not achieved*.

**This violates the documented contract.** `pw_kernel/kernel/lib.rs:68-72`
already anticipates exactly this case:

> If `switched` is `false`, the implementation guarantees that forward
> progress will be made. For example, a context switch may be deferred to an
> interrupt handler (like PendSV) which is pending. The caller does not need
> to retry or take further action to ensure the switch occurs.

The Cortex-M port honors this by deferring to PendSV when in interrupt
context. The RISC-V port never checks.

### 1b. The scheduler lock is held for the entire duration of a voluntary block

`Arch::context_switch` passes its `SpinLockGuard` into `riscv_context_switch`.
The guard is not dropped until the blocked thread is later resumed and its own
call chain unwinds past it — so the lock reads as held for as long as the
thread stays blocked (e.g. the whole time it sits in `wait()`).

The first attempt by anything else to acquire that lock meanwhile — such as an
interrupt handler waking a different thread, i.e. case 1a — immediately trips
the single-hart spinlock's recursive-lock check —
`pw_assert::panic!("recursively locked spinlock")` in `BareSpinLock::lock()`
(`pw_kernel/arch/riscv/spinlock.rs:150`).

Cortex-M drops the lock before its switch and re-acquires a fresh guard on
resume. RISC-V does not.

### Why they compound

1a and 1b are individually reachable but reinforce each other: the natural
trigger for 1b is an interrupt handler waking a blocked thread, which is
precisely 1a. Any fix for one that does not address the other still faults.

### Ordering constraints a fix must respect

Two non-obvious constraints, both easy to get wrong:

1. **The lock must be dropped before `THREAD_LOCAL_STATE` is switched.**
   Dropping the guard also drops its embedded `PreemptDisableGuard`, which
   decrements `preempt_disable_count` via `Arch::thread_local_state()`. If
   `THREAD_LOCAL_STATE` already points at the incoming thread, this decrements
   the *new* thread's fresh, zeroed counter and underflows it.

2. **Interrupt-nesting depth must be decremented before completing a deferred
   switch.** Completing the switch parks the trap frame until the thread is
   scheduled again, so a decrement placed after it would never pair with its
   increment.

---

## Part 2 — The coverage gap

**No test on any upstream RISC-V target exercises a context switch initiated
from interrupt context.** This is the root reason the defects went unnoticed.

### Root cause: no RISC-V target can synthesize an interrupt

`InterruptController::trigger_interrupt` panics on **both** RISC-V controllers:

- `pw_kernel/arch/riscv/plic.rs:274` — `"trigger_interrupt not supported on the PLIC"`
- `pw_kernel/arch/riscv/veer_pic.rs:373` — `"trigger_interrupt not supported on the PIC"`

Consequently:

- `//pw_kernel/tests/interrupts/kernel:test_interrupts` is explicitly
  `target_compatible_with`-incompatible on `@platforms//cpu:riscv32`.
- `//pw_kernel/tests/interrupts/user:test_interrupts` is wired into
  `mps2_an505`, `pw_rp2350`, and `nucleo_f103rb` — all Cortex-M — and no
  RISC-V target. Its `debug_trigger_interrupt` syscall bottoms out in the same
  panicking call.

### `qemu_virt_riscv32` specifically

It is the only target under `//pw_kernel/target/` with no `interrupts/`
directory. Its sole live interrupt source is `MachineTimer` (`mtimer_tick`,
`pw_kernel/arch/riscv/exceptions.rs:148`). The tick path does reschedule, but
usually re-selects the interrupted thread, so it rarely drives a real switch
out of interrupt context.

**Reaching the buggy path requires an interrupt that makes a *different*
thread runnable.** Nothing on this target does that.

Net effect: `k_qemu_virt_riscv32` passing does **not** indicate that
interrupt-initiated context switches work.

### The gap generalizes to RISC-V targets outside this tree

The same shape recurs on out-of-tree RISC-V targets, for the same root cause:

| Controller | `interrupts/` dir | Covers the path? |
|---|---|---|
| PLIC | no | no |
| PLIC (with a UART-driven IRQ test) | no | only via a test gated behind Verilator or FPGA/silicon — not in any default test config |
| VeeR PIC | yes | the interrupt test is single-threaded (spins on an `AtomicBool` for a device IRQ); proves *delivery*, not *switching* |

Note the third row: having an `interrupts/` directory is not sufficient. A
test that only confirms an interrupt was *delivered* — a handler ran, a flag
was set — never schedules a second thread, so it never reaches the defective
path.

A target with a **real device interrupt** sidesteps the `trigger_interrupt`
limitation entirely: an external agent drives a peripheral, and its IRQ wakes a
userspace process blocked in `object_wait`. Tests of that shape were written
and run for this report (they run in an emulator in seconds) — but as recorded
below, they do not reach `Arch::context_switch` either, because the generic
scheduler defers first.

---

## Reproduction: not achieved

**No runtime reproduction was obtained, and the generic scheduler appears to
prevent one.** This section records that negative result, because it bounds
what the defects above actually mean in practice.

Three tests were run on a VeeR target with a real device (I3C) interrupt
source, each patched and unpatched, against two different pigweed pins —
**eight emulator runs, no failure in any of them**:

| Wake path | Result |
|---|---|
| IRQ wakes the thread it interrupted | passes patched and unpatched |
| IRQ wakes a second, already-runnable thread | passes patched and unpatched |
| IRQ wakes a blocked thread in a *different* process (channel `USER` signal) | passes patched and unpatched |

The mechanism that prevents the reproduction is in the generic layer, above
the arch port:

1. `InterruptGuard::new` takes a `PreemptDisableGuard`
   (`pw_kernel/kernel/interrupt_controller.rs`), so `preempt_disable_count`
   is above 1 for the whole handler.
2. `try_reschedule` (`pw_kernel/kernel/scheduler/core.rs`) calls
   `context_switch` **only** when that count is exactly 1. In interrupt
   context it takes the other branch: it sets `needs_reschedule` and returns.
3. The switch is completed later, from `InterruptGuard::drop` →
   `try_deferred_reschedule`, after the handler's guards have unwound.

`pw_kernel/kernel/scheduler.rs` documents this as intended: `try_reschedule`
"may trigger an immediate context switch or it may defer it. This is useful
for code which may or may not be called in an interrupt context."

There is also a same-thread early return in `scheduler::context_switch`
(`current_thread_id == new_thread.id()`) that returns before reaching
`Arch::context_switch` whenever a wake re-selects the running thread.

**Consequence.** On the paths reachable from these tests, `Arch::context_switch`
is not called from trap context at all, so the RISC-V defects above are not
observable. They are a latent hazard — a missing guard that the layer above
currently makes unreachable — rather than a demonstrated live failure. Either
some path not exercised here bypasses `try_reschedule`, or the arch-level
check is defense in depth. This report does not establish which.

---

## Suggested fixes

### For the defects

Port the Cortex-M stash-and-defer pattern to RISC-V. RISC-V has no
PendSV/tail-chaining hardware, so the deferred switch must be completed in
software from `trap_handler`'s tail, once the handler has fully returned and
every RAII guard it took has unwound. Track interrupt context as a nesting
*depth*, and complete the deferred switch only while unwinding the outermost
interrupt — the same condition as Zephyr's
`if (--_current_cpu->nested == 0)`. As on Cortex-M, stash only the outgoing
thread; read the incoming thread back from the scheduler at completion time.

Separately, drop the scheduler lock before the switch and re-acquire a fresh
guard on resume — respecting the ordering constraint above.

A working implementation of both fixes exists. It builds and passes on a
RISC-V target with a real device-interrupt source — but so does the unpatched
kernel, so that is evidence of no regression, not of a fix.

### For the gap

Ranked by value per unit of effort:

1. **Implement software-triggered interrupts in the PLIC driver.** QEMU's
   `virt` machine exposes writable PLIC pending bits, so `trigger_interrupt`
   is implementable there even though it is not on real hardware or the VeeR
   PIC. This unblocks the existing shared suites for `qemu_virt_riscv32`.
2. **Add `target/qemu_virt_riscv32/interrupts/user/`**, mirroring
   `target/mps2_an505/interrupts/user/`. The userspace suite is the better of
   the two: it already covers the case that matters (a blocked thread woken by
   an interrupt handler) and needs no kernel-side triggering support beyond
   the syscall path.

Until at least one lands, no upstream RISC-V target regression-tests this
code, and the defects above can silently return.
