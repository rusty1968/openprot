# Architecture Knowledge Base

This folder is a lightweight system for making architecture decisions with evidence.
Use it with Copilot to keep decisions consistent, auditable, and revisitable.

## Goals

- Capture decision context and constraints once.
- Compare options using the same scorecard each time.
- Link claims to evidence instead of opinion.
- Record outcomes to improve future decisions.

## Structure

- `decisions/`: one decision record per decision (ADR-style).
- `evidence/`: evidence cards from code, tests, incidents, benchmarks, and specs.
- `experiments/`: quick validation plans for uncertain assumptions.
- `outcomes/`: post-implementation result reviews.
- `templates/`: canonical templates used by all records.
- `prompts/`: reusable Copilot prompts for this workflow.
- `index.md`: dashboard and status tracker.

## Recommended Workflow

1. Create a decision record from `templates/decision-record.md`.
2. Ask Copilot to extract evidence cards into `evidence/`.
3. Fill a scorecard from `templates/scorecard.md` for all options.
4. If confidence is low, create an experiment from `templates/experiment.md`.
5. Choose an option and record rationale, risks, and revisit trigger.
6. After implementation, create an outcome review from `templates/outcome-review.md`.

## Naming Convention

- Decisions: `DEC-YYYYMMDD-short-title.md`
- Evidence: `EVD-YYYYMMDD-short-claim.md`
- Experiments: `EXP-YYYYMMDD-short-hypothesis.md`
- Outcomes: `OUT-YYYYMMDD-decision-id.md`

## Confidence Levels

- `high`: directly supported by fresh measurements/tests in this repo.
- `medium`: inferred from related components or older data.
- `low`: assumption not yet validated.

## First Run Checklist

- Copy all templates at least once into active records.
- Create `index.md` entries for open decisions.
- Add one revisit date to each accepted decision.
- Link each major claim to at least one evidence card.
