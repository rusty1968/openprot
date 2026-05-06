# Copilot Prompts For Architecture Decisions

Use these prompts directly in Copilot Chat from the repo root.

## 1) Extract Evidence From Recent Changes

"Scan recent changes affecting <component>. Create evidence cards in docs/architecture-knowledge/evidence using the template. For each card, include claim, source links, confidence, and counter-evidence."

## 2) Build Option Set

"For decision <DEC-ID>, propose 3 options with explicit tradeoffs in security, reliability, performance, complexity, and reversibility. Do not choose yet."

## 3) Fill Scorecard

"Using linked evidence cards for <DEC-ID>, fill docs/architecture-knowledge/templates/scorecard.md for options A/B/C. Flag any score not backed by evidence."

## 4) Adversarial Review

"Act as a skeptical reviewer. For the current top option in <DEC-ID>, list failure modes, attack paths, and operational risks with severity and likely detection signals."

## 5) Design Fastest Validation Experiment

"Identify the highest-impact low-confidence assumption in <DEC-ID> and produce an experiment plan from templates/experiment.md that can validate or falsify it within one sprint."

## 6) Decision Record Completion

"Complete the decision record for <DEC-ID> with chosen option, rationale, risks, mitigation, validation plan, and revisit trigger. Keep unsupported claims out."

## 7) Outcome Review

"Compare predicted vs actual outcomes for <DEC-ID> using commits, tests, and incidents since acceptance. Create an outcome review and list process improvements."

## 8) Drift Check

"Audit accepted decisions in docs/architecture-knowledge/decisions for stale evidence, missed revisit dates, and contradictory outcomes. Produce a remediation list."
