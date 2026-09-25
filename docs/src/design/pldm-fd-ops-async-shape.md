# `FdOps`: the "asynchronous" trait that isn't, and how to actually use it that way

## The observation

`pldm-interface`'s `FdOps` trait (`pldm_interface::firmware_device::fd_ops::FdOps`,
vendored via `@rust_crates//:pldm-interface`) documents itself as asynchronous:

> "This trait defines **asynchronous** methods for performing various firmware
> device operations..."

Every method's doc comment repeats it — "**Asynchronously** retrieves device
identifiers," etc. But every method is a plain synchronous `fn` returning
`Result<T, FdOpsError>` directly. No `async fn`, no `Future`, no `Poll`, no
callback/waker registration. `download_fw_data`, `verify`, and `apply` all
have to fully finish and hand back a definitive result within one call.

That reads like residue from a Tock-upcall or Rust-async origin that got
flattened into synchronous calls during a port, without the docs (or,
charitably, the intended *contract*) being updated to match.

This matters because OpenPRoT's own idiom, everywhere else, is the opposite
of "one call does everything": `orchestrator_capabilities`'s `Updatable::
poll_stage`, `BootWatch::poll`, and `IncrementalVerifier::poll` are all
explicit, bounded, single-step, never-blocking — progress comes back as a
value (`StageProgress::Transferring { written, total }`,
`WalkVerdict::Waiting { deadline_millis }`, `PollOutcome::Processing { done,
total }`), and the caller polls again. A trait that instead demands "finish
the whole operation in one call" doesn't fit that world, and every operation
it backs is a candidate for blocking a whole (cooperative, single-threaded)
kernel thread for however long the real work takes.

## What the wire protocol and `CmdInterface` actually do

Reading the vendored crate directly (not just the trait declaration)
changes the diagnosis. `pldm-interface/src/firmware_device/fd_context.rs`,
`pldm_fd_progress_verify`/`pldm_fd_progress_apply` (~L929-994):

```rust
fn pldm_fd_progress_verify(&mut self, _payload: &mut [u8]) -> Result<usize, MsgHandlerError> {
    ...
    res = self.ops.verify(&self.internal.get_component(), &mut progress_percent)?;
    self.internal.set_fd_verify_progress(progress_percent.value());
    if res == VerifyResult::VerifySuccess && progress_percent.value() < 100 {
        // doing nothing and wait for the next call
        return Ok(0);
    }
    // ...only past this point does it send VerifyComplete
```

This is already the bounded-step poll shape: one call does one bounded unit
of work, reports `progress_percent`, and "success but under 100%" means
*call me again*. `GetStatus` (`get_status_rsp`, ~L537) reads that cached
progress from `get_fd_verify_progress`/`get_fd_apply_progress` while a
verify/apply is in flight, so a UA polling `GetStatus` mid-operation sees
real progress. The "asynchronous" in the doc comments describes this
call-again-until-done convention — not `async`/`await`, but a real,
already-functioning analog of `IncrementalVerifier::poll`.

`download_fw_data` doesn't have this internal retry shape, but it doesn't
need to: it's driven by the wire protocol's own chunking
(`RequestFirmwareData` is MTU-bounded), so one call already only ever
covers one bounded chunk.

## Where it's actually broken

Not the trait. The implementations.

`DemoFdOps::verify()` (`target/ast10x0/tests/pldm/firmware_update/fd_main.rs`)
and `MockFdOps::verify()` (`services/pldm/tests/*.rs`) both ignore all of
the above: they read back or "verify" the *entire* image in one call and
leave `progress_percent` at its default (`NOT_SUPPORTED`, which the check
above treats as "done regardless" — the demo's own comment says as much:
"FD treats as done"). For a 1 KiB test image this is free. For a real
image, `DemoFdOps::verify`'s `for base in (0..IMAGE_SIZE).step_by
(READBACK_CHUNK)` loop is exactly the kind of single blocking call the rest
of the codebase's poll-based capabilities exist to avoid.

`download_fw_data` is lower-risk than it looks, but only because
`DemoFdOps::init_flash()` erases the whole destination sector *up front*,
before the transfer starts, rather than inside `download_fw_data` itself.
Each `download_fw_data` call is therefore already just one bounded page
program — fast, bounded, no surprise sector-erase latency hiding inside a
per-chunk call. A production implementation needs to preserve that
discipline (erase ahead of time) rather than erasing on demand inside
`download_fw_data`, or this risk reappears there too.

## The fix: adapt `verify`/`apply` onto capabilities that already exist

A production `FdOps::verify`/`apply` should be a thin adapter over the
orchestrator's own `IncrementalVerifier`/`VerifySession::poll` (or an
equivalent bounded "apply" step) — one `FdOps` call, one `poll()` step:

```rust
fn verify(
    &self,
    _component: &FirmwareComponent,
    progress_percent: &mut ProgressPercent,
) -> Result<VerifyResult, FdOpsError> {
    match self.session.borrow_mut().poll(&self.payload) {
        PollOutcome::Processing { done, total } => {
            *progress_percent = ProgressPercent::new((done * 100 / total) as u8)
                .map_err(|_| FdOpsError::VerifyError)?;
            Ok(VerifyResult::VerifySuccess) // success + <100% == "call me again"
        }
        PollOutcome::Authenticated(_) => {
            *progress_percent = ProgressPercent::new(100)
                .map_err(|_| FdOpsError::VerifyError)?;
            Ok(VerifyResult::VerifySuccess)
        }
        PollOutcome::Rejected(_) | PollOutcome::Fault(..) => Ok(VerifyResult::VerifyGenericError),
    }
}
```

One `FdOps::verify()` call == one `VerifySession::poll()` step == one
bounded chunk of hashing, which lines up with how `FirmwareDevice::
run_once_inner` already treats one PLDM exchange as one bounded loop
iteration (see `run_until`, `services/pldm/src/firmware_device.rs`). No
trait change, no patch — just an implementation that uses the incrementality
the trait and `CmdInterface` already support instead of doing everything in
one call.

`apply()` should get the same treatment against whatever bounded "commit"
capability backs activation.

## What's left, and why it's lower priority

There's no clean type-level way to express "still working" distinct from a
real percentage — the only signal for "call me again" is `VerifySuccess`
paired with a progress value under 100, conflating an in-progress state
with a made-up interim percentage. A cleaner contract (an explicit
`Pending`/`InProgress` variant) would require patching `pldm-interface`
itself — precedented in this codebase (see the Pigweed SysTick patch), so
not unheard of, but a real, separate piece of work. Lower priority than the
implementation fix above, since the existing convention already works
correctly once something actually uses it; it's an ergonomics gap, not a
correctness one.

## Summary

- The trait's "asynchronous" framing is inaccurate as async/await, but the
  underlying call-again-until-done convention it's describing already
  exists and already fits OpenPRoT's poll-based idiom.
- `verify`/`apply`'s blocking risk is a property of today's demo/mock
  implementations, not of the trait or `CmdInterface`.
- The actionable fix is to rewrite `FdOps::verify`/`apply` as adapters over
  `IncrementalVerifier::poll` (or equivalent), one bounded step per call.
- `download_fw_data`'s risk is already mitigated by up-front erasing;
  preserve that discipline in any production implementation.
- A cleaner `Pending`-vs-percentage distinction would need a
  `pldm-interface` patch — real, but optional and lower priority.
