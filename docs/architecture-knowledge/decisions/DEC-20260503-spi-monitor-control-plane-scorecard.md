# Decision Scorecard: DEC-20260503-spi-monitor-control-plane

- Decision ID: DEC-20260503-spi-monitor-control-plane
- Date: 2026-05-03
- Weights owner: platform architecture

## Criteria Weights

Total equals 100.

| Criterion | Weight |
|---|---|
| Security | 25 |
| Reliability | 20 |
| Performance | 10 |
| Complexity | 10 |
| Operability | 15 |
| Time to deliver | 10 |
| Reversibility | 10 |

## Option Scoring

Rate each criterion from 1 (worst) to 5 (best).

| Option | Security | Reliability | Performance | Complexity | Operability | Time to deliver | Reversibility | Weighted Total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| A: flash drivers only | 2 | 2 | 5 | 4 | 2 | 4 | 3 | 285 |
| B: dedicated monitor service | 5 | 4 | 4 | 3 | 4 | 3 | 4 | 410 |
| C: orchestrator owns MMIO | 3 | 3 | 4 | 3 | 3 | 3 | 3 | 320 |

## Evidence Coverage Check

| Option | # High confidence evidence cards | # Medium | # Low | Gaps |
|---|---:|---:|---:|---|
| A | 2 | 1 | 0 | no explicit runtime lock/attestation owner |
| B | 4 | 0 | 1 | update-window token model details |
| C | 2 | 1 | 1 | least-privilege boundary proof |

## Notes

- Weighted total formula: sum(score * weight).
- Option B leads by more than 5 percent; no tie-break experiment required for choice.
