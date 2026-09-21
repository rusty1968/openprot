# Orchestrator docs cleanup plan

Scope: `docs/src/design/orchestrator/{orchestrator-overview,orchestrator-machine,orchestrator-model}.md`.
Goal: remove redundant and stale content (~200 of 857 lines) without losing
information. Markdown only — edit but **do not stage** (per convention).

Order chosen so mechanical/safe cuts land first and the risky prose compression
last.

---

## C1 — De-duplicate the state diagram (safe, mechanical)

**Problem:** two Mermaid diagrams of the same topology.
- `orchestrator-overview.md` L20–L52 (effects omitted)
- `orchestrator-machine.md` L7–L40 (with effects)

The overview's is a strict subset of the machine's; every transition edit must
touch both and they can drift.

**Change:** keep the machine.md diagram as the single canonical one. In
overview.md, replace the fenced diagram with a one-line pointer:

> See [State Machine](./orchestrator-machine.md) for the full state diagram
> (states, guards, effects, and entry actions).

Keep the short intro sentence above the old diagram so the "State Topology"
section still reads. Retarget any in-page anchors if needed.

**Est. savings:** ~33 lines. **Risk:** low.

---

## C2 — Collapse the triplicated "effects/reads as data" statement (safe)

**Problem:** the same core idea appears three times:
- `orchestrator-overview.md` L62 "Design Principles" (Effects not actions; Reads
  as events)
- `orchestrator-machine.md` L66 "Context — `Sink`" prose
- `orchestrator-model.md` L270 §5 "The Platform Boundary" (authoritative table)

**Change:** treat model §5 as the source of truth.
- In overview.md, cut the "Effects, not actions" and "Reads as events" bullets
  down to a single line pointing at model §5; keep "Feedback as data" and
  "Board-supplied policy" (those are not restated elsewhere).
- In machine.md "Context — `Sink`", trim the prose to what is specific to `Sink`
  (append-only, fresh-per-dispatch, sized to `E`); drop the general
  "core never does I/O" restatement, linking to model §5 instead.

**Est. savings:** ~15 lines. **Risk:** low.

---

## C3 — Compress the "supervision contract" prose (needs care)

**Problem:** machine.md argues the same point across three sections:
- L277 "Superstate — `SupervisingPlatform`" (the table — **keep**)
- L291 "Centralizing the Supervision Contract" (~80-line HSM-vs-flat essay)
- L372 "Invariant Verification" (restates the same "one copy + four links" case)

The essay and the invariant section overlap heavily (both argue copies-drift /
one-authoritative-place).

**Change:** keep the L277 table. Replace L291 + L372 with one tighter section:
- 1 short paragraph: why the shared rules live in a superstate (single
  authoritative copy of the corruption + attestation response; the safe thing is
  the default fall-through).
- Keep the INV5/INV6 "where it lives / to verify" table verbatim — it is the
  concrete, testable payload.
- Drop the extended flat-vs-HSM debate, the "honest caveat" paragraph, and the
  duplicated framing.

**Est. savings:** ~90 lines. **Risk:** medium — this is opinionated design
rationale; confirm nothing referenced elsewhere links into the removed prose.

---

## C4 — Fix stale `statig` content (correctness, not just redundancy)

**Problem:** machine.md still describes the removed `statig` design and fields
that no longer exist after the ComponentStatus migration (`3f1a17e`).
- L43 "Shared storage — `Rot<N>`" table lists `held`, `failed`, `retry_count`,
  `awaiting` as fields. Current code: `chain`, `cursor`, `statuses`
  (`ComponentStatus { lifecycle, retry }`), `max_retry`, `_effect_cap`; the
  `failed`/`awaiting` data now lives in `State` payloads
  (`Recovering(ComponentId)`, `AwaitingReady(Option<ComponentId>)`).
- L409 "`statig` integration" section — obsolete; the crate no longer depends on
  `statig` (BUILD deps are heapless only). Delete the section.
- Stale references to `statig` context / `handle_with_context` / `superstate()`
  in surrounding prose — reword to the plain reducer (`Sink`, `Outcome::Super`,
  `is_supervised`).
- Stale test name: `corruption_during_presupervision_selfloop_is_dropped` →
  actual is `corruption_during_presupervision_selfloop_triggers_recovery`.

**Change:** rewrite the shared-storage table to the real fields; delete the
`statig` integration section; reword statig references to the reducer model; fix
the test name.

**Est. savings:** ~20 lines (plus correctness). **Risk:** medium — verify field
descriptions against `services/orchestrator/sm/src/lib.rs` before writing.

---

## Sequencing & validation

1. C1, then C2 (mechanical, independent).
2. C4 (correctness pass against current `lib.rs`).
3. C3 last (largest prose change).
4. After each: re-check intra-doc links/anchors resolve; skim rendered mdBook if
   convenient.
5. Do **not** `git add` any of these files.

**Total est. reduction:** ~160–200 lines of 857.
