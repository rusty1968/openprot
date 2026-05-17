# HACE Technical Debt — exclusivity is discipline, not structure

*Status: **RESOLVED** 2026-05-16. Behavior-preserving structural/soundness
refactor; HACE digest/HMAC parity (see `goal.md`) left exactly as-is — the
KAT-harness pass/fail boundary is byte-for-byte unchanged (all-green through
`hmac-sha512 rfc4231-4`, same known §2.2 failure at `hmac-sha512 rfc4231-6`
with identical wrong bytes `actual[0..8]=5a6e818a5a40625b
actual[last4]=693158be`), so the layout-sensitive §2.2 fault is provably
unperturbed.*

> **Correction to Evidence #1 (found during the fix):** `device.rs:9`'s
> `#[derive(Debug, Copy, Clone, PartialEq, Eq)]` is on the unused `HashAlgo`
> enum stub, **not** on `HaceDevice` (struct is the next item, no derive — true
> since the introducing commit `f30a614`). `HaceDevice` was already
> `!Copy`/`!Clone`, so Checklist **box 1 always passed**. The single
> genuinely-failing box was **box 2** (process-global `HashContext` via the
> free `shared_ctx_ptr()` accessor, independently `&mut`-aliased per
> `from_device` call). That is what was fixed.
>
> **Fix:** deleted the crate-wide `shared_ctx_ptr()` free accessor; the
> `.ram_nc` / `#[repr(C, align(64))]` context static is now reached only via
> `context::acquire_shared_ctx()`, called **once** at the `unsafe fn new*`
> single-instance gate and stored as the device's sole private
> `ctx: *mut HashContext`. `HaceDigest::from_device` reborrows it as a
> transient `&mut` *through* the exclusive `&mut HaceDevice` borrow (the
> `yield_fn` reborrow pins `&'a mut device`, arbitrating it). A raw-pointer
> field (not `&'static mut`) was deliberate: the KAT harness holds multiple
> shadowed-but-live `HaceDevice` values in one scope (e.g. `target.rs:198`–
> `240`); a `&'static mut` field would put two aliasing `&'static mut` to the
> same static in scope at once — UB the borrow checker cannot see and exactly
> the §2.2-class hazard. The raw-pointer field preserves today's
> transient-`&mut`-per-op profile byte-for-byte. `.ram_nc`, `align(64)`,
> single-in-flight, and the cooperative-yield wiring are all unchanged.
>
> *Original analysis below retained for the record.*

## The debt (one sentence)

`HaceDevice` *looks* like it structurally enforces single-engine exclusivity
(`HaceDigest::from_device(&mut HaceDevice)`), but it does **not** — real mutual
exclusion rests entirely on the *documented* `unsafe` non-reentrancy contract,
i.e. the Zephyr reference's "caller must serialize" discipline written in Rust.

## Evidence (cited)

1. **`HaceDevice` is `#[derive(Debug, Copy, Clone, PartialEq, Eq)]`**
   ([../device.rs:9](../device.rs)). A `Copy` device defeats `&mut`
   arbitration: copy the device → two independent values → two `from_device`
   borrows → two concurrent operations, borrow checker none the wiser.
2. **Operation state is a process-global `HashContext`** reached via
   `unsafe { &mut *super::context::shared_ctx_ptr() }`
   ([../digest.rs:128](../digest.rs); `shared_ctx_ptr` defined
   [../context.rs:142](../context.rs)). It is aliased *outside* the device;
   two `HaceDigest`/`HaceHmac` (trivially obtainable, per (1)) take `&mut` to
   the **same global** — unsound aliasing, not merely unguarded.

Net: the `from_device(&mut …)` borrow-split is *cosmetic for exclusivity*. The
SHA/HMAC ops of the one HACE engine are mutually exclusive only by caller
discipline — exactly the defect the structural pattern exists to remove.

## Acceptance gate

Conform to the `design-patterns` catalog entry
**`borrow-arbitrated-engine-exclusivity`** (its Checklist is the oracle). The
two boxes HACE fails today:

- device is **`!Copy`/`!Clone`**;
- **no `static`/global mutable operation state aliased via raw pointer outside
  the device** (the `HashContext` must be owned by / threaded through the
  device, not `shared_ctx_ptr()`).

## Worked reference to mirror

The sibling **SBC** port (ECDSA op #1, RSA op #2 — same engine) does it
correctly: `SbcDevice` has **no** `derive(Copy/Clone)`, and `SbcOp` carries no
global op-state (operands passed by value). See the `openprot-ecdsa` worktree
`target/ast10x0/peripherals/sbc/{device.rs,op.rs}` and that catalog entry's
Sample Code. Mirror that shape for HACE.

## Hard constraints / known hazards (read before touching)

- **Behavior-preserving only.** HACE has a pinned behavioral-parity goal
  (`goal.md`) and a KAT/parity harness. This refactor must not change any
  digest/HMAC byte output; the harness must stay green. It is an
  ownership/state-placement change, **not** an algorithm change.
- **DMA / memory-layout sensitivity.** `HashContext` holds the SG list,
  `buffer`, and `digest` — DMA targets in `.ram_nc`, `#[repr(C, align(64))]`
  (see `context.rs`, `goal.md` §1.3/§5.1). Moving its ownership into the
  device **must preserve** the non-cached placement, 64-byte alignment, and
  the single-in-flight invariant. `goal.md` §2.2 records an *open*
  memory-layout-sensitive HMAC-SHA512 long-key fault whose victim moved when
  buffers were relocated — **this refactor can perturb that**. Re-run the full
  KAT harness; if a previously-green case regresses, that is this refactor's
  fault, not a new bug. Treat per the `stop-and-instrument` skill, do not
  paper over.
- The vendor constraint (HACE cannot do concurrent streaming;
  `goal.md` §5.1) is unchanged — this makes the single-owned-device model the
  *correct* one, which is the point.

## Out of scope

Algorithm/parity changes; HMAC-SHA512 §2.2 debugging (separate); any HAL-trait
redesign. Just make exclusivity structural while preserving behavior.
