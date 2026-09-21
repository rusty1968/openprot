# Is the HSM (hierarchical state machine) design overkill? — discussion to revisit

This captures a design discussion prompted by the `PreSupervision` corruption
gap (see [`presupervision-corruption-gap.md`](./presupervision-corruption-gap.md)).
The gap itself raised a broader question worth coming back to: was choosing an
HSM (`statig`, superstates) for this state machine the right call, or is it
more machinery than the problem currently needs?

---

## The question

`orchestrator-machine.md`'s "Centralizing the Supervision Contract" section
justifies the HSM design (four leaf states — `Ready`, `Updating`, `Recovering`,
`AwaitingReady` — sharing one superstate, `SupervisingPlatform`) on the grounds
that it centralizes two shared rules (`AttestationChallenge` always answered,
`CorruptionDetected` always triggers recovery) in one place instead of
duplicating them across every state.

While investigating the `PreSupervision` gap, the natural follow-up question
came up: **for a payoff this small (2 events, 4 states), is the HSM pattern
overkill?**

---

## Case that it's overkill (right now)

- The shared surface is small: exactly 2 events, across 4 states. That's a
  thin payoff for the added indirection.
- The doc's own text admits the cost directly:

  > "The cost is that reading one state no longer tells the whole story... This
  > is a trade, not a free win... And the payoff today is small in raw terms:
  > the superstate shares just two events across four states."

- You cannot know a leaf state's *full* behavior by reading its own match arms
  — you must also check whether `superstate()` returns `Some(...)` or `None`
  for it, and then go read the superstate's handler too. That extra
  "is this state a member?" check is exactly the kind of implicit,
  easy-to-miss detail that let the `PreSupervision` gap go unnoticed and
  untested.
- The doc's justification for taking on this cost rests on **speculative
  future growth**, not present necessity: *"transit tamper detection and
  telemetry queries are both platform-wide operational events on the OpenPRoT
  roadmap."* Neither has shipped. Today, the abstraction is being paid for by
  a payoff that doesn't exist yet.

## Case that it isn't

- The `PreSupervision` gap is not actually evidence against the HSM
  specifically. A flat design with a shared helper function (called explicitly
  from each state's default arm) has an identical failure mode: someone still
  has to decide whether `PreSupervision`'s default arm calls the helper, and
  getting that decision wrong is just as invisible either way. The gap is a
  **membership/scoping decision** (which states opt in), not a defect in
  `statig`'s superstate dispatch mechanism.
- Because of the HSM, *fixing* the gap (if it's deemed a bug) is a one-line
  change — add `State::PreSupervision` to the `superstate()` match arm that
  returns `Some(Superstate::SupervisingPlatform(...))` — rather than manually
  threading duplicate logic into `PreSupervision`'s own table. That is a real,
  tangible benefit independent of whether the roadmap events ever land.
- The "single copy to verify" argument for INV5/INV6 (see
  `orchestrator-machine.md`'s "Invariant Verification" section) is still true
  for the four states that *are* linked — nothing about the gap undermines
  that specific benefit for those four.

## Working verdict (not final — revisit later)

Not clearly overkill, but also not strongly justified at the current 2-event,
4-state scale on its own — the design is explicitly betting on a roadmap, not
present necessity. The more interesting question isn't really "HSM vs. flat
state machine" — it's **why `PreSupervision` sits outside the `SupervisingPlatform`
boundary at all**, given the docs claim continuous supervision "from the first
result onward." That specific scoping decision is what's actually worth
re-litigating, independent of which architectural pattern wraps it.

---

## Open questions to revisit

1. If `PreSupervision` is added to `SupervisingPlatform`'s membership (the
   `corruption_during_presupervision_selfloop_triggers_recovery` test in
   `lib.rs` currently fails, asserting this desired behavior), does that
   change break any other invariant or existing test? Needs a full pass, not
   just the one new test.
2. Should `AttestationChallenge` also be answerable during `PreSupervision`
   (i.e., mid-walk, before the chain is fully verified)? The CSA quote about
   measurements being collectible "during this sequence" suggests yes, but
   this hasn't been decided.
3. If the roadmapped tamper-detection/telemetry events never materialize,
   should the HSM pattern be reconsidered in a future refactor, or is the
   `PreSupervision`-membership question independent of that and worth fixing
   regardless?
4. Is there a shell-side guarantee (outside this crate) that `CorruptionDetected`
   is never emitted while the machine is still in `PreSupervision`? If so, the
   gap may be dead code protection rather than a live bug — this needs
   confirmation from whoever owns the shell implementation.
