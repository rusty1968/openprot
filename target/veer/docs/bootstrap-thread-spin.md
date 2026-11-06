# The spinning bootstrap thread: a suspected root cause of the IPC cost and the 15 ms gaps

Status 2026-10-26: **hypothesis from reading the kernel and matching measured numbers; not yet tested on the
board.** Two test images are built and waiting for a board run. Everything marked [INF] below is inference.

## 1. What we saw (measured)

| observation | where | number |
|---|---|---|
| one IPC round trip to the MCTP server, cheapest op | `fpga-test-debug.md` 17.1 | 385,369 cycles = 21.5 ms (syscall floor 6,325 cycles = 0.35 ms) |
| removing the FD process boundary | 19.1 | 65.0 s -> 52.2 s (-20%), 38 ms per chunk |
| gaps of about 15 ms at arbitrary points in a chunk | 22.1 | 164 gaps in 8.7 s, median 14.95 ms, about 5 per chunk, spacing about 22.7 ms |
| a "wait until now" returns only at the next scheduler tick | 24.1 | every pass 202,160 cycles = 11.29 ms (88.6 Hz) |
| a tight 3-instruction loop of 60,000 instructions | 12.4 | 3,499,487 cycles = "58 cycles per instruction" |

## 2. What the kernel does (read from the source)

All paths are in the pigweed checkout `pw_kernel` (commit `3ada25a6`).

- `kernel/lib.rs`: `bootstrap_thread_entry` runs `kernel.init()`, starts the idle thread, then calls
  `target::main()`. The bootstrap thread has `Priority::DEFAULT_PRIORITY` (Level15); the idle thread has
  `IDLE_PRIORITY` (Level0).
- `target/veer/tests/*/target.rs` (12 files): `Target::main() { codegen::start(); loop {} }`. After
  `codegen::start()` creates the app processes and threads, the bootstrap thread **spins forever**.
- `tooling/system_generator/system_config.rs:350`: a thread without an explicit `priority` gets
  `DEFAULT_PRIORITY`. Our `system.json5.tmpl` files set none, so app threads have the **same priority** as
  the spinning bootstrap thread.
- `arch/riscv/timer/mtime.rs`: a periodic tick, `SCHEDULER_TICK_HZ = 100` (`config/lib.rs:20`), re-armed
  at a fixed interval (`set_next_monotonic_tick`), not at the earliest deadline. The FPGA config assumes a
  20 MHz timer (`target/veer/config.rs:46`) on a core that runs at 17.91 MHz, so the real period is longer
  than 10 ms: measured 11.29 ms.
- `kernel/scheduler/core.rs`: `tick` processes the timer queue and calls `try_reschedule(Ticked)`, so
  threads of equal priority are time-sliced round-robin at every tick.
- Pigweed's own reference targets (`target/pw_rp2350/ipc/user/target.rs`, `clock/user/target.rs`) end
  `main` with `loop { let _ = kernel::sleep_until(Arch, Instant::MAX); }`: their bootstrap thread
  **sleeps**. The `threads/kernel` and `interrupts/kernel` tests use `loop {}` but they are kernel-thread
  tests.

## 3. The hypothesis

A thread that is ready and has the same priority as a spinner does not run until the spinner's slice ends
at a tick. Consequences [INF]:

1. **Wake-up latency.** When an app thread becomes runnable (an interrupt, a message, the end of a
   blocking call) and the spinner is running, the app thread can wait up to one tick (11.3 ms).
2. **Lost slices while running.** When the app thread has the CPU and a tick arrives, `try_reschedule`
   hands the CPU to the spinner for a full slice. Gaps of about one tick, about every second tick
   (the app thread runs a slice, the spinner a slice): that is a spacing of 2 x 11.29 = 22.6 ms, which
   matches the trace's 22.7 ms, and a gap length of about 1.3 ticks, which matches the median 14.9 ms.
   About five per chunk is what a chunk of 60 to 80 ms of runnable time would give.
3. **The "IPC cost".** A round trip is two thread switches (client blocks, server runs, client resumes).
   385k cycles is 1.9 ticks: the cost looks like two scheduling latencies, not about 6,500 instructions.
   The tightness of the measurement (min 385,295, max 388,967 over 64 samples) fits a tick-locked
   mechanism better than an instruction-count one.
4. **The isolation price.** The co-located FD saves 38 ms per chunk (19.1). If most of an IPC is
   scheduling latency caused by the spinner, most of that 20% was never the price of isolation.
5. **"58 cycles per instruction".** A 3.5M-cycle loop spans about 17 ticks; if the spinner took alternate
   slices, the true cost per instruction could be lower. Not tested. The syscall floor (6.3k cycles, no
   thread switch) is unaffected.

What it does not explain: the first-principles cost of the instructions themselves (uncached SRAM is still
slow), and whether an interrupt wake is also delayed to a tick (the kernel's interrupt path wakes the
thread directly; whether it preempts an equal-priority spinner immediately is not verified).

## 4. What would contradict it

- No change in chunk time when the spinner is removed.
- An IPC round trip still about 385k cycles with a sleeping bootstrap thread.
- Gaps still present with the same count and size.

## 5. The test

Change `Target::main()` of `pldm_throughput_colo` and `pldm_throughput_fdcolo` (uncommitted) to what
Pigweed's targets do:

```rust
fn main() -> ! {
    codegen::start();
    // The bootstrap thread has nothing left to do. Sleep rather than spin: a spinning thread at the same
    // priority as the app threads takes a time slice at every scheduler tick and delays every wake-up
    // behind it.
    loop {
        let _ = kernel::sleep_until(Arch, kernel::Instant::MAX);
    }
}
```
with `use arch_riscv::Arch;`. Emulator: `pldm_throughput_fdcolo_test` and its `no_panics_test` pass.

Board images (clean, no trace; include the I3C driver changes and the PIC gateway patch, so the
comparison is not bit-for-bit with the earlier runs):

| image | file | sha256 |
|---|---|---|
| FD in its own process | `phase2/openprot_pldm_throughput_colo_nospin.bin` | `b47b7ab157f21e079e0767555059c0099eac46fe9c25f834591df617d066faba` |
| FD co-located | `phase2/openprot_pldm_throughput_fdcolo_nospin.bin` | `2f21cb30ca55bc93c0a4c90ac7876d5ba5cd1dfc3c30d02bb701c852762a44b5` |

Reference results to compare with:

| configuration | download |
|---|---|
| native | 40.7 s |
| FD in its own process, event-driven (`528517cc`) | 65.0 s |
| FD co-located (`ffba59be`) | 52.2 s |

## 6. How to read the outcome

- **Both much faster, and a small gap between them:** confirmed. Then: IPC is cheap, the isolation price
  was mostly the spinner, and the earlier IPC and isolation conclusions in `fpga-test-debug.md` (17.1,
  18.1, 19.1) and the Slack draft need correcting. The right fix is in the target (all 12 veer targets),
  with a short note upstream-style in the commit.
- **Both faster, a large gap remains:** partly confirmed; the remainder is real IPC and scheduling cost.
- **No change:** refuted. The 15 ms gaps and the IPC cost are something else; back to the gap detector
  design (a pure clock-read spin, no `object_wait`, which blocks until the next tick).

## 7. If confirmed: follow-ups

- Apply the fix to all 12 `target.rs` files under `target/veer/tests/` (and any other veer target), not
  just these two.
- Re-measure one IPC round trip (the `get_eid` microbenchmark, `phase2/ipc-timing-pldm_fd.patch`).
- Re-run the trace (`phase2/fd_colo_trace.rs`) to see how many ~15 ms gaps remain.
- Re-check the 58 cycles per instruction (`cpi cal` loop) and the IPC timing explanations in the notes.
- Consider raising the app threads' priority above the bootstrap thread, or lowering the bootstrap
  thread's, as a second safeguard (`priority` in the thread objects of `system.json5`).
- Check the native caliptra-mcu-sw runtime does not have an equivalent spinner (it is a Tock kernel with
  a different scheduler; not examined).

## 8. Result (2026-10-26): confirmed for the spinner, refuted for the isolation claim

| configuration | spinning bootstrap | sleeping bootstrap | spinner cost |
|---|---|---|---|
| FD in its own process | 65.0 s (194 ms/chunk) | 49.0 s (146 ms/chunk) | -16.0 s, 48 ms/chunk |
| FD co-located | 52.2 s (156 ms/chunk) | 37.3 s (111 ms/chunk) | -14.9 s, 45 ms/chunk |
| FD process boundary | 12.8 s, 38 ms/chunk | 11.7 s, 35 ms/chunk | |

Native: 40.7 s. With the spinner removed the co-located build is 8% faster than native and the isolated-FD
build is 1.20x native.

- **Confirmed:** the spinning bootstrap thread cost about 45 to 48 ms per chunk (a quarter to a third of the
  download) in both configurations.
- **Refuted:** section 3.4 / 6 above ("the isolation price was mostly the spinner"). The process boundary costs
  about the same with and without the spinner (35 to 38 ms per chunk), so it is real, not an artifact.
- **Not yet re-measured:** one IPC round trip (385k cycles) and "58 cycles per instruction" (sections 3.3, 3.5);
  both were taken with the spinner running.
