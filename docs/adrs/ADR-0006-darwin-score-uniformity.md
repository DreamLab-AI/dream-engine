# ADR-0006: A darwin leaderboard where every mutant scores the same fails the darwin evaluator

- **Status**: Accepted — extends ADR-0005
- **Date**: 2026-10-06
- **Related**: ADR-0005 (darwin entrypoint fails on a bound breach), ADR-0003 (darwin bound guard), ADR-0002 (entrypoint liveness)
- **Deciders**: operator, 2026-10-06 ("wire it in properly")
- **Tags**: dream-cycle, evaluation-adapters, evaluator-trust

## 1. Context

Every darwin leaderboard observed from the pinned `@metaharness/darwin@0.10.2`
against the :8084 ruvllm endpoint — the 2026-09-07 run (ab4ced4e48b76e83) and
every receipt from 2026-09-27 through 2026-10-03 — scores all mutants
identically (0.765, `Delta over baseline: +0.000`). A scorer that returns the
same value for every candidate is returning a default, a fallback answer or a
parse-error value; it is not judging. The leaderboard's winner is then an
artefact, yet the run passed the ADR-0005 checks and exited 0.

The 2026-10-03 security-adversarial candidate added a detector for this shape
as an unwired diagnostic with its own row parser.

## 2. Decision

Score uniformity is checked in the same place, and fails through the same
channel, as the ADR-0005 step-10 bounds:

- `parseLeaderboardRows` (`packages/cli/src/darwinBounds.ts`) reads each row's
  score; there is one leaderboard parser.
- `checkDarwinBounds` calls `checkDarwinScoreUniformity`. When **two or more**
  scored mutants (non-baseline rows) all carry the identical score, it adds the
  violation `score uniformity: all N mutants score S[, equal to baseline |
  , baseline B] — scorer not discriminating`.
- Fewer than two scored mutants is too little evidence and is not a violation.
  The baseline row is reported against, never counted as a mutant.
- `verify-entrypoint darwin` (and so `scripts/darwin-entrypoint.sh`) prints the
  violation under `darwin bounds VIOLATED` and exits **3**, so the annexe
  records the REQUIRED darwin evaluator as FAILED, as for any bound breach.

## 3. Consequences

- Until the scorer or the mutator changes, darwin nights fail with exit 3 and
  the uniformity line in the receipt instead of passing on a meaningless
  leaderboard. This is intended: the receipt now names the defect.
- The real 2026-09-07 leaderboard fixture is now the uniformity breach case;
  a discriminating fixture (`darwin-leaderboard-varied.txt`) is the compliant
  ceiling.

## 4. Test Contract

`packages/cli/src/darwinBounds.test.ts` (unit), `index.test.ts`
(`verify-entrypoint darwin` exits 3 on the uniform leaderboard, 0 on a varied
one) and `darwinEntrypoint.test.ts` (the real script, same two cases).
