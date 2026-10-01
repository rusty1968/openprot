# pw_kernel: channel IPC corrupts a process context under sustained load (veer / RISC-V)

## Summary

On the VeeR-EL2 (RISC-V `rv32imc`) target, a userspace process that drives a
large number of synchronous channel transactions (`channel_transact`) against a
peer handler eventually has its register context corrupted mid-transaction and
jumps to address `0`. The fault is **deterministic**: with a fixed workload it
crashes at the same transaction count every run.

It reproduces with nothing but the kernel channel syscalls — two trivial
processes, no drivers, no interrupts, no protocol code. It is **still present on
pigweed upstream HEAD** (`578e9b00c44350c912c3c93dd52e7f9be7496703`, 2026-09-30)
and was also seen on `f9b83b60c9fa3dc7d0127e687fef170147cd2d3e`.

## Environment

- **Arch / target:** VeeR-EL2, RISC-V `rv32imc` (no A/F/D), `pw_kernel`.
- **Runner:** Caliptra MCU emulator (unified SRAM `0x40000000`, size `0x80000`).
- **pigweed:** `578e9b00c44350c912c3c93dd52e7f9be7496703` (upstream `main` HEAD on
  2026-09-30). Also reproduced on `f9b83b60…`.
- **Build:** `edition = "2024"`, two `rust_process`es in one `system_image`.

## Crash signature

Two userspace processes: `client` does `channel_transact` in a loop, `server`
echoes via `channel_read` + `channel_respond`. After ~3300 round-trips, the
running process takes an **instruction access fault at `epc = 0`** with a
wrecked context, immediately followed by a kernel-mode terminal exception:

```
[INF] channel stress client: tick 3300          <- last progress log before the fault
[INF] Exception frame 0x4000c510:
[INF] ra  0x40002d24 t0 0x18001880 t1  0x4000238e t2  0x00000000
[INF] a0  0x4000da9c a1 0x00000000 a2  0x00000002 a3  0x00000003
[INF] a4  0x00000000 a5 0x00000002 a6  0x00000000 a7  0x00000010
[INF] tp  0x4000c6e8 gp 0x00000000 sp  0x00000000
[INF] mstatus 0x18001800
[INF] mcause 0x00000001        <- Instruction access fault
[INF] mtval  0x00000000
[INF] epc    0x00000000        <- jumped to null

[FTL] Terminal exception in kernel mode
[INF] Exception frame 0x4000c410:
[INF] ra  0x40003954 ...
[INF] mcause 0x00000003         <- breakpoint/ebreak (kernel fatal path)
[INF] epc    0x40003954
[INF] FAIL: 1
```

Key points:

- `epc = 0`, `mcause = 1` (instruction access fault): control transferred to
  address `0`, i.e. a corrupted return address / function pointer.
- `sp = 0` and `gp = 0` (and in other runs `gp = 0xffffffff`, `tp = 0xffffffff`):
  the process's pointer registers are trashed, not just `pc`.
- The second frame (`mcause = 3`, `epc = ra`) is the kernel's own fatal-error
  path (`ebreak`) reached while handling the first fault.

In an earlier investigation of the same crash (a PLDM-over-MCTP-over-I3C download
on `f9b83b60`), the faulting return address resolved to
`kernel::object::buffer::SyscallBuffer::new_in_current_process` (its epilogue),
and the faulting frame held the **sender payload bytes and length together with a
mix of client- and server-owned RAM pointers** — consistent with corruption
during the kernel's cross-process buffer copy for the channel transaction.

## Determinism

With a fixed 180-byte payload, the crash lands in the **same 100-transaction
window every run** (last log `tick 3300`, i.e. between transactions 3300 and
3400). Across `--runs_per_test=3` on `578e9b00`: **3/3 failed, identical point.**

## Steps to reproduce

Two processes on one channel. The client sends a fixed payload and reads the
echo, in a loop; the server echoes. No interrupts, no peripherals.

**client** (initiator):

```rust
const MSG_LEN: usize = 180;             // crashes with other sizes too
const NUM_TRANSACTIONS: u32 = 10_000;   // crash hits ~3300, well before this

let req = [0xa5u8; MSG_LEN];
let mut resp = [0u8; 256];
for i in 0..NUM_TRANSACTIONS {
    if i % 100 == 0 { pw_log::info!("tick {}", i); }
    syscall::channel_transact(handle::CHANNEL, &req[..], &mut resp[..], Instant::MAX)?;
}
// never reached: crashes ~transaction 3300
let _ = syscall::debug_shutdown(Ok(()));
```

**server** (handler):

```rust
syscall::wait_group_add(handle::WG, handle::CHANNEL, Signals::READABLE, 0)?;
let mut buf = [0u8; 256];
loop {
    syscall::object_wait(handle::WG, Signals::READABLE, Instant::MAX)?;
    let n = syscall::channel_read(handle::CHANNEL, 0, &mut buf)?;
    syscall::channel_respond(handle::CHANNEL, &buf[..n])?;   // echo
}
```

**system config:** one app, two processes — `server` with a `wait_group`, a
`channel_handler`, and a thread; `client` with a `channel_initiator` (bound to
the server's handler) and a thread. Empty interrupt table. See the sibling files
in this directory (`system.json5`, `server.rs`, `client.rs`, `target.rs`,
`BUILD.bazel`) for the complete, buildable case.

In this repository:

```bash
bazel test //target/veer/tests/channel_stress:channel_stress_test
# FAILS: client faults at ~transaction 3300 (epc=0, mcause=1), then kernel
# terminal exception (mcause=3). The test passes only if the channel path
# survives all NUM_TRANSACTIONS.
```

## Expected vs actual

- **Expected:** `channel_transact` / `channel_read` / `channel_respond` sustain
  arbitrarily many back-to-back transactions; the client reaches
  `debug_shutdown(Ok)` and the test passes.
- **Actual:** after ~3300 transactions the active process's context is corrupted
  and it jumps to `0`; the kernel then takes a terminal exception.

## Impact

Any `pw_kernel` application on veer that drives sustained channel IPC crashes.
In our case it makes a PLDM firmware-update-over-I3C flow (a long download over
channel-backed transports) fail intermittently; the failure rate tracks the
amount of channel traffic, and the download never completes reliably.

## Notes

- The ~3300 bound scales with workload shape, which (together with the
  `SyscallBuffer` locus and the trashed pointer registers) points at a
  resource/bookkeeping issue in the channel-transaction buffer path rather than
  a single bad message.
- Not fixed by moving pigweed forward (`294d431a` → `f9b83b60` → `578e9b00`).
