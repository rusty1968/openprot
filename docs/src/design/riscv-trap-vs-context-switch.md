# Trap frames vs. context-switch frames on RISC-V

Background for the `pw_kernel` RISC-V arch port: what the two stack frames
are, why they are complements, and why performing a context switch from
inside a trap handler abandons the trap frame.

Line references are to unpatched upstream pigweed `79c3ff65f`.

---

## Two frames, two halves of the register file

The RISC-V calling convention splits registers into two groups, and
`pw_kernel` has one frame type for each.

### `ContextSwitchFrame` — callee-saved only

`pw_kernel/arch/riscv/threads.rs:41`

```
ra, s0, s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11
```

13 words. That is `ra` plus the callee-saved `s` registers, and nothing else.

It can be this small because a *voluntary* switch happens at a function call
site. By the calling convention, the compiler has already spilled any
caller-saved register (`a0`-`a7`, `t0`-`t6`) whose value it still needed
across the call. Saving them again would be redundant.

### `TrapFrame` — caller-saved, plus trap state

`pw_kernel/arch/riscv/exceptions.rs:192`

```
epc, status, ra,
a0, a1, a2, a3, a4, a5, a6, a7,
t0, t1, t2, t3, t4, t5, t6,
tp, gp, sp,          // only set when trapping from user space
pad[3]               // 16-byte stack alignment
```

`0x60` bytes, statically asserted.

It must be this large because an *involuntary* trap strikes at an arbitrary
instruction. Nothing was spilled, no calling convention applies, so the
caller-saved half has to be preserved explicitly — along with `epc` and
`status`, the machine state needed to resume.

### The complement

|  | `ContextSwitchFrame` | `TrapFrame` |
|---|---|---|
| Saves | callee-saved (`ra`, `s0`-`s11`) | caller-saved (`a0`-`a7`, `t0`-`t6`) + `epc`/`status` |
| Created by | a voluntary call to `riscv_context_switch` | the trap entry stub |
| Why the other half is absent | compiler already spilled it | no compiler involvement; nothing was spilled |
| Exits via | `ret` | `csrw mepc` + `mret` |

Between them they cover the whole register file. Neither is sufficient alone.

---

## How each one is unwound

### A context switch ends in `ret`

`riscv_context_switch` (`pw_kernel/arch/riscv/threads.rs:295`) is three steps:

1. Push `ra` + `s0`-`s11` onto the current stack; store `sp` into `*a0`, the
   outgoing thread's saved frame pointer.
2. Load `ra` + `s0`-`s11` from `a1`, the incoming thread's frame; set `sp`
   past it.
3. `ret` — jump to the **newly loaded** `ra`.

So the function is entered on one thread's stack and returns on another's.
From each thread's point of view it is an ordinary call that took a long time
to return.

### A trap ends in `mret`

Returning from a trap is not a `ret`. It means unwinding back out through the
entry stub:

1. Reload the caller-saved registers from the `TrapFrame`.
2. `csrw mepc` with the saved `epc`.
3. `mret` — jump to `mepc` and restore the previous interrupt-enable state
   from `mstatus.MPIE`, atomically.

`mret` is a privileged instruction that does something `ret` cannot: it
restores privilege level and interrupt-enable state as part of the jump.

---

## What "abandoning the trap frame" means

Calling `riscv_context_switch` from inside `trap_handler` mixes the two.
Control enters through the trap path but leaves through the context-switch
path — it ends in `ret`, jumping to the incoming thread.

The `TrapFrame` is left sitting on the outgoing thread's stack, never
consumed. Concretely:

- **`mepc` is never written back.** The interrupted PC is dropped.
- **`mret` never executes.** `mstatus.MIE` is never restored, so the CPU stays
  in the interrupt-entry state it was in.
- **`a0`-`a7` and `t0`-`t6` are never reloaded.** They hold whatever the
  incoming thread leaves in them.

That last point is the sharp edge. The `ret`-based switch preserves exactly
the callee-saved half — and the trap frame exists precisely to protect the
*other* half. Using it on a trap-entered stack drops the registers the trap
frame was there to save.

There is a second casualty. Because the outgoing stack is frozen mid-call
rather than unwound, every RAII guard on it is frozen too: destructors never
run, so any lock the interrupt handler's call chain still holds is never
released. On a single-hart spinlock, the next acquirer trips the
recursive-lock assertion.

### Why the symptom is far from the cause

Nothing faults at the moment of the bad switch. The damage surfaces later,
when the abandoned thread is eventually rescheduled and resumes through a
corrupted return path — typically as an **instruction access fault at PC=0**,
in code with no visible relationship to the interrupt that caused it.

---

## The rule

> A `ret`-based context switch is only valid from an ordinary function-call
> context.

Two conditions have to hold, and interrupt context satisfies neither:

1. The compiler has spilled the caller-saved registers around the call site.
   Not true of a trap, which strikes at an arbitrary instruction.
2. No unrelated RAII guards are live on the stack. Not true of a trap handler,
   whose call chain holds its own guards.

The fix is not to make the switch safe in trap context — it cannot be. It is
to **defer** it: record that a switch is owed, let the interrupt handler
return normally through `mret` so its frame is consumed and its guards unwind,
and perform the switch afterward from a context where both conditions hold.

`Arch::context_switch`'s documented contract already allows exactly this
(`pw_kernel/kernel/lib.rs:68-72`):

> If `switched` is `false`, the implementation guarantees that forward
> progress will be made. For example, a context switch may be deferred to an
> interrupt handler (like PendSV) which is pending. The caller does not need
> to retry or take further action to ensure the switch occurs.

Cortex-M implements this by deferring to PendSV. RISC-V has no PendSV or
tail-chaining hardware, so the equivalent has to be built in software:
complete the deferred switch at the tail of the trap handler, once the
handler has fully returned and every guard it took has unwound.
