# Kickoff Prompt — HACE Exclusivity Refactor (AST10x0)

Purpose: pay down the technical debt in
[goal-tech-debt.md](goal-tech-debt.md) — make HACE single-engine exclusivity
**structural**, not caller discipline. Pattern reference: `design-patterns`
skill, catalog entry `borrow-arbitrated-engine-exclusivity` (its Checklist is
the acceptance gate).

> **Scope note:** behavior-preserving structural/soundness refactor only. HACE
> has a pinned behavioral-parity goal ([goal.md](goal.md)) + a KAT/parity
> harness — **no digest/HMAC byte output may change**; the harness must stay
> green. This is an ownership/state-placement change, not an algorithm change.

---

## Facts to front-load

1. **The debt, cited:** `HaceDevice` is `#[derive(Copy, Clone)]`
   (`../device.rs:9`) and operation state is a process-global `HashContext`
   via `unsafe { &mut *shared_ctx_ptr() }` (`../digest.rs:128`,
   `../context.rs:142`). Together these make the `from_device(&mut …)`
   borrow-split cosmetic — exclusivity is the Zephyr reference's
   "caller-serializes" discipline, not structural. Full analysis:
   [goal-tech-debt.md](goal-tech-debt.md).
2. **Acceptance gate:** the `borrow-arbitrated-engine-exclusivity` Checklist.
   The two failing boxes: device `!Copy/!Clone`; no global mutable op-state
   aliased outside the device.
3. **Reference instance to mirror:** the SBC port in the `openprot-ecdsa`
   worktree — `target/ast10x0/peripherals/sbc/{device.rs,op.rs}`
   (`SbcDevice`: no `Copy/Clone`; `SbcOp`: operands by value, no global ctx).
   Copy its shape.
4. **DMA hazard (do not skip):** `HashContext` is `.ram_nc`,
   `#[repr(C, align(64))]`, holds SG/buffer/digest DMA targets
   (`context.rs`, `goal.md` §1.3/§5.1). Ownership must move into the device
   **without** changing placement, alignment, or the single-in-flight
   invariant. `goal.md` §2.2 is an open memory-layout-sensitive HMAC-SHA512
   fault — this refactor can perturb its victim; re-run the **full** KAT
   harness and treat any regression per the `stop-and-instrument` skill.
5. **Stop condition:** `!Copy` device owning (or threading) the context,
   borrow-arbitrated ops, Checklist satisfied, KAT/parity harness green. No
   parity/algorithm changes.

---

## Recommended prompt (copy-paste)

> Pay down the HACE exclusivity technical debt in
> `target/ast10x0/peripherals/hace/plans/goal-tech-debt.md`, following the
> `design-patterns` catalog entry `borrow-arbitrated-engine-exclusivity`.
> Mirror the conforming SBC instance
> (`openprot-ecdsa:target/ast10x0/peripherals/sbc/{device.rs,op.rs}`).
>
> Make `HaceDevice` **not** `Copy`/`Clone`, and move the `HashContext` so it
> is owned by / threaded through the device instead of the process-global
> `shared_ctx_ptr()` — **preserving** its `.ram_nc` placement,
> `#[repr(C, align(64))]`, and the single-in-flight invariant. Every
> `HaceDigest`/`HaceHmac` must be obtainable **only** via an exclusive `&mut`
> borrow of the one device.
>
> Hard constraint: behavior-preserving. Do not change any digest/HMAC output
> or the parity behavior in `goal.md`. Re-run the full KAT/parity harness; it
> must stay green. `goal.md` §2.2 records an open memory-layout-sensitive
> HMAC-SHA512 fault — if a previously-green case regresses, that is this
> refactor's doing: stop and instrument (per the `stop-and-instrument`
> skill), do not paper over.
>
> Acceptance: every box in the `borrow-arbitrated-engine-exclusivity`
> Checklist; harness green; no parity delta. Stop there.

## Minimal version

> Apply `borrow-arbitrated-engine-exclusivity` to HACE: `HaceDevice` `!Copy`,
> `HashContext` owned by the device (not `shared_ctx_ptr()`), ops only via
> `&mut` borrow; preserve `.ram_nc`/align/placement; KAT harness stays green;
> zero parity change. Mirror SBC. Stop there.

---

## Why the prompt is shaped this way

- Naming the catalog entry loads its Checklist as the conformance oracle.
- "Mirror SBC" — there is a conforming sibling instance; shape-matching it is
  the cheapest path to conformance and keeps the family consistent.
- "Behavior-preserving / harness green / §2.2 caveat" — the only real risk
  here is perturbing the known layout-sensitive fault; the prompt forces
  re-running the harness and escalating instead of guessing.

## Known caveat

HACE being `Copy` is *correct* for the `cooperative-yield-bounded-poll-device`
entry (whose Sample Code HACE is — that pattern uses the borrow-split for
type-erasure, where `Copy` is fine). The same code conforms there and fails
here: the two Checklists differ deliberately. Removing `Copy` must not break
the cooperative-yield wiring (the type-erased `yield_fn` reborrow path) —
verify both patterns still hold after the change.
