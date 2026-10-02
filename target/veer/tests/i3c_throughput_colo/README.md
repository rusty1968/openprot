# i3c_throughput_colo — colocated i3c+MCTP throughput experiment

An experiment that answers one question: **how much does the `mctp <-> i3c` IPC
boundary cost?** It is `i3c_throughput` with the `i3c_server` and `mctp_server`
processes **merged into one** (`mctp_i3c_colo`), so the MCTP router reaches the
I3C target in-process via `i3c_server::dispatch` instead of a pw_kernel channel.
Everything else — the sink, the host driver, the message sizes — is identical,
so the difference is purely that removed boundary.

Lives on the `i3c-mctp-colocated` branch, separate from the main throughput work.

## How it differs from `i3c_throughput`

- **2 processes, not 3.** `mctp_i3c_colo` owns the I3C peripheral + IRQ *and*
  runs the MCTP router; it handles the i3c interrupt inline (latch into the ring)
  and services the MCTP app channel in the same loop.
- The i3c `Server` is shared via a `RefCell`; an in-process `Transport`
  (`ColoTransport`) calls `dispatch` directly — no `channel_transact`, no
  cross-process `SyscallBuffer` copy, no context switch per fragment.
- Only the i3c↔mctp boundary is removed. The sink stays a separate, isolated
  process over the `mctp` channel.
- Outbound back-pressure is dropped (the sink's acks are single-fragment, read
  before the next — no multi-TX overrun to guard).

## Result (emulated-time `mtime`, sink-measured)

| message | fragments | `i3c_throughput` (3 proc) | this (2 proc) | gain |
|---|---|---:|---:|---:|
| 120 B | 1 | 835 B/s | 1100 B/s | +32% |
| 240 B | 2 | 1237 B/s | 1734 B/s | +40% |
| 480 B | 3 | 1718 B/s | 2387 B/s | +39% |
| 720 B | 4 | 2043 B/s | 2777 B/s | +36% |

**Removing the i3c↔mctp boundary is worth ~35%**, consistently. Measured in
*emulated* time (the sink reads `mtime`), so this is a code-level signal — the
per-byte cost of the per-fragment `channel_transact`, independent of the
emulator's host-execution speed.

## The tradeoff — why this is an experiment, not a merge

The boundary exists on purpose: it is fault/privilege isolation between the I3C
driver and the MCTP stack, which a Root-of-Trust wants. Removing it means a bug
or compromise in the i3c driver can corrupt MCTP state directly. caliptra-mcu-sw
accepts this — its i3c driver and MCTP are in-kernel Tock capsules with no
boundary — which is part of why its per-byte path is cheaper.

So ~35% is the price of that isolation. Whether to pay it is a security decision,
not a pure perf one; this test exists to make the number explicit.

## Running

```bash
for n in 120 240 480 720; do
  bazel test //target/veer/tests/i3c_throughput_colo:i3c_throughput_colo_test \
    --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
    --test_env=TPUT_MSG_BYTES=$n 2>&1 | grep -E 'THROUGHPUT:'
done
```

Same flags/caveats as [`../i3c_throughput`](../i3c_throughput) (`emulator`,
`exclusive`, `--nocapture`, `--jobs=2`).
