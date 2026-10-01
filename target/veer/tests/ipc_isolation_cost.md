# The i3c ↔ mctp isolation boundary: throughput cost

## Question

OpenPRoT runs the I3C driver and the MCTP stack as **separate pw_kernel
userspace processes**, connected by an IPC channel. Every I3C frame the MCTP
stack sends or receives crosses that boundary as a `channel_transact` — a
kernel-mediated cross-process copy (`SyscallBuffer`) plus a syscall and context
switches. caliptra-mcu-sw does **not** have this boundary: its I3C driver and
MCTP are in-kernel Tock capsules that call each other directly.

**How much throughput does that isolation cost us?**

## TL;DR

Removing the i3c↔mctp boundary (merging the two processes so the MCTP router
reaches the I3C target in-process via `dispatch` instead of a channel) is worth:

- **~35%** on raw transport, measured in emulated time (`i3c_throughput_colo`).
- **~26%** on a full PLDM download, measured UA-side wall-clock
  (`pldm_throughput_colo`).

That is the price of the driver/stack fault isolation. Whether to pay it is a
**security decision, not a pure performance one** — so both colocated builds are
kept as experiments on the `i3c-mctp-colocated` branch, not merged.

## Why this boundary, and why it dominates per-byte cost

Three processes, two boundaries:

```
pldm_fd ──"mctp"── mctp_server ──"i3c"── i3c_server ──(IRQ/MMIO)── HW
                 boundary 1            boundary 2
```

- `pldm_fd ↔ mctp_server` is crossed **once per MCTP message**.
- `mctp_server ↔ i3c_server` is crossed **once per i3c fragment** (every
  `I3cOp::Send`/`Recv`). At the 241 B MTU that is ~1 crossing per 240 bytes — the
  highest-frequency crossing in the path, so it is the one to target.

The colocated experiments remove **only** boundary 2: the I3C `Server` is shared
in-process (a `RefCell`) and an in-process `Transport` (`ColoTransport`) calls
`i3c_server::dispatch` directly. The app (sink or PLDM FD) stays a separate,
isolated process over `mctp`, so only the i3c↔mctp copy is eliminated.

## Method

Two A/B experiments, each identical to its 3-process baseline except for the
merged boundary (same sink/FD, same host driver, same message/chunk sizes):

| experiment | vs baseline | metric |
|---|---|---|
| `i3c_throughput_colo` | `i3c_throughput` | emulated time (firmware `mtime`) |
| `pldm_throughput_colo` | `pldm_throughput` | UA-side wall-clock |

Emulated time (`mtime`) is the cleaner metric — it counts the firmware's own
cycles and is independent of the emulator's host-execution speed. Wall-clock is
what caliptra-mcu-sw reports, but it folds in the host round-trip and the
emulator's speed, which dilutes the measured gain.

## Results

### Raw transport — emulated time (`i3c_throughput_colo`)

| message | fragments | 3 processes | colocated | gain |
|---|---|---:|---:|---:|
| 120 B | 1 | 835 B/s | 1100 B/s | +32% |
| 240 B | 2 | 1237 B/s | 1734 B/s | +40% |
| 480 B | 3 | 1718 B/s | 2387 B/s | +39% |
| 720 B | 4 | 2043 B/s | 2777 B/s | +36% |

Consistent **~35%** across message sizes.

### Full PLDM download — UA wall-clock (`pldm_throughput_colo`)

| | 3 processes | colocated |
|---|---:|---:|
| download (5-run mean, 4096 B) | ~1323 B/s | ~1670 B/s |

**+26%**, 5/5 reliable (1665–1687 B/s, dev 0.1 s).

## Interpretation

- The two figures are consistent: the boundary has a **fixed per-fragment cost**
  (the `channel_transact` copy + syscall + context switches). In emulated time
  that is ~35% of the per-byte transport cost. In the PLDM wall-clock number the
  same fixed saving is diluted by the host round-trip and the emulator's
  host-execution speed, so it shows as ~26% end-to-end.
- Treat **~35% (emulated) as the boundary's true per-byte cost**, and ~26% as its
  end-to-end PLDM share on this emulator. Neither is a real-silicon figure — the
  absolute B/s is emulator-bound (see `pldm_throughput/PERF_ANALYSIS.md`); the
  **relative** gain is the hardware-independent signal.
- Cross-comparison to upstream's 1100–1300 B/s is loose (different emulator
  config and stack). The clean signal is the internal colo-vs-3-process A/B.

## The tradeoff

The boundary is **fault and privilege isolation** between the I3C driver and the
MCTP stack. Removing it means a bug or compromise in the I3C driver can corrupt
MCTP state directly — the kind of blast-radius containment a Root-of-Trust is
built to keep. caliptra-mcu-sw accepts this (no boundary); OpenPRoT's architecture
deliberately does not.

So the decision is: **is ~26–35% download throughput worth collapsing that
isolation?** For a RoT the default answer is no. If a specific product needs the
throughput, a middle path keeps most of the modularity: link the i3c and mctp
crates into one process (direct calls, no channel) while keeping them as separate,
host-testable libraries — losing fault isolation only between those two layers,
not against the PLDM FD or the rest of the system.

## Reproduce

```bash
# raw transport, emulated-time sweep
for n in 120 240 480 720; do
  bazel test //target/veer/tests/i3c_throughput_colo:i3c_throughput_colo_test \
    --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
    --test_env=TPUT_MSG_BYTES=$n 2>&1 | grep -E 'THROUGHPUT:'
done

# full PLDM download, UA wall-clock
bazel test //target/veer/tests/pldm_throughput_colo:pldm_throughput_colo_test \
  --runs_per_test=5 --flaky_test_attempts=1 \
  --test_output=all --test_arg=--nocapture --nocache_test_results --jobs=2 \
  2>&1 | grep -E 'PLDM_THROUGHPUT|Stats over'
```

Compare against the 3-process `i3c_throughput` / `pldm_throughput`.

## Implementation notes (for anyone extending the colocated builds)

- **RefCell double-borrow.** `match srv.borrow_mut()...on_interrupt()` with
  another `srv.borrow_mut()` inside an arm panics at runtime. Read the event into
  a local first (`let evt = srv.borrow_mut()...; match evt { ... }`).
- **Trace logs skew `mtime`.** Hot-path `pw_log::info!` calls cost emulated
  cycles — ~6 per message dropped the 240 B figure from 1734 to 1058 B/s. Strip
  all tracing before taking a measurement.

See `i3c_throughput_colo/README.md` and `pldm_throughput_colo/README.md` for the
per-experiment detail.
