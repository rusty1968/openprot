# Evaluation: the PLDM `FdOps` callback shape on pigweed

Assesses the PLDM firmware-device callback trait (`FdOps`, from
`pldm-interface`/`pldm-lib`) as used by `services/pldm` on the pigweed
(`pw_kernel`) architecture. The shape was inherited from the original Tock OS +
async-Rust design in `../caliptra-mcu-sw`
(`runtime/userspace/api/pldm-lib/src/firmware_device/`).

## What the shape is

`FdOps` is the platform seam the PLDM FW-update state machine (`fd_context`)
calls back into: device identifiers, firmware parameters, transfer size,
component handling, **chunked download**, **verify**, **apply**, activate,
cancel, and `now()` timestamps.

The state machine is a **polled, incremental cooperative state machine**, not a
blocking one. Long operations are sliced and re-polled:

```rust
// pldm-interface fd_context.rs, pldm_fd_progress_verify (sync port)
res = self.ops.verify(&component, &mut progress_percent)?;   // one slice of work
self.internal.set_fd_verify_progress(progress_percent.value());
if res == VerifySuccess && progress_percent.value() < 100 {
    return Ok(0);   // "wait for the next call" — re-polled next fd_progress step
}
```

`download_fw_data` (per RequestFirmwareData chunk), `verify`, and `apply` all
work this way. `services/pldm`'s `FirmwareDevice::run_terminus`/`run_until`
re-enters `fd_progress` each loop iteration, interleaved with a responder poll.

## Key finding: the poll model predates async

The caliptra-mcu-sw async original's `pldm_fd_progress_verify` is **structurally
identical** to the sync port — same `if progress < 100 { return Ok(0) }`
re-poll. So the cooperative poll loop was always the real concurrency
mechanism; `async` only additionally let each *slice* yield during its own I/O
(e.g. a flash read inside one `verify` slice) and let the transport `.await` on
MCTP.

**This is why the port works and is the good news:** the incremental-poll shape
maps cleanly onto pigweed's blocking-reactor model. `run_terminus` *is* a
pigweed-style reactor; sync `MctpClient`/`channel_transact` replaces the
transport awaits; the slices replace the intra-op awaits. The core design is a
genuinely good fit for pigweed.

## Async vestiges that are now liabilities

1. **`&self` everywhere → forced interior mutability.** Every `FdOps` method
   takes `&self` (an async-trait-era choice: state shared across await points).
   In a sync single-threaded reactor this is unnecessary and worse: the mocks in
   `services/pldm/tests/*` wrap *all* state in `Cell`/`RefCell`
   (`component_accepted: Cell<bool>`, `download_bytes_received: Cell<usize>`, …).
   A real flash-backed impl inherits the same burden — losing compile-time
   exclusivity and gaining `RefCell` runtime-panic risk, where sync-native
   `&mut self` would be safe and natural.

2. **The load-bearing contract is undocumented.** In async, "don't hog the CPU"
   was enforced structurally by await points + the executor. In the sync port it
   is an *implicit* rule: **each `FdOps` slice must return promptly** so the loop
   can re-poll the responder (e.g. to service `CancelUpdate` mid-transfer).
   Nothing in the trait states this, and the doc comments still say
   "**Asynchronously** retrieves…" on synchronous `fn`s — actively misleading.
   **The sharp edge:** a naive `verify()` that hashes the whole image in one
   call (progress 0→100) blocks the entire pigweed thread for the full duration
   — starving the responder path *and* any other work that thread hosts. Async
   made that impossible to write by accident; the sync shape invites it.

3. **Redundant progress plumbing.** `ProgressPercent` out-params +
   `is_download_complete` + `query_download_progress` coexist with the slice
   return values — poll-support scaffolding from the async external-tracking
   model. Harmless but extra surface.

## Implications for running PLDM over I3C

Compatible: wire `FirmwareDevice` (sync `run_terminus`) to a `MctpClient` backed
by the i3c MCTP path (`mctp_server` over the i3c server). No async runtime is
needed — the reactor loop is the driver. See [[i3c-emulator-test-patterns]] for
the on-target harness conventions and [[veer-emulator-oom]] for build limits.

**But** the `FdOps` impl used for the test must honor the bounded-slice contract
— chunked download, incremental verify/apply — or it blocks the reactor and, on
hardware, stalls the responder so the UA's `CancelUpdate` cannot land. That
constraint is on the *implementation*, and is currently unenforced and
undocumented.

## Recommendations

- **Document the bounded-slice contract** on `FdOps` (each call does a small,
  prompt unit of work; long operations report incremental progress and are
  re-polled) and fix the "Asynchronously" doc comments. Cheapest,
  highest-value fix — it is the trap. (Upstream `pldm-lib` change.)
- **Consider a sync-native `&mut self`** trait to drop the forced
  `Cell`/`RefCell`. Breaking change to upstream `pldm-lib`, so an upstream
  conversation, not a local edit.
- For the test `FdOps`, deliberately slice `download`/`verify`/`apply` so the
  test exercises the responder-during-transfer path — enabling a later
  `CancelUpdate`-mid-transfer test, in the spirit of `tests/i3c_backpressure`.

## Bottom line

The callback shape survived the environment change better than expected —
because the real mechanism was always the poll loop, not async. The residue is
`&self`/interior-mutability, stale "Asynchronously" docs, and an unenforced
"return promptly" contract that is now the main correctness hazard on pigweed.
