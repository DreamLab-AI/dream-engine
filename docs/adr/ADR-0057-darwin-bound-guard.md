# ADR-0057: Fail-visible Darwin bound guard in the evaluation-adapter layer

- Date: 2026-09-07
- Status: Proposed (draft PR `dream/evaluation-adapters-20260907`)
- Companion to: ADR-2024 (deterministic evaluator veto)

## Context

Nightly pipeline step 10 bounds every Darwin run: generations <= 3,
candidates per generation <= 4, promoted lineages <= 1. On 2026-09-07
(run `ab4ced4e48b76e83`, commit `9f4d8a9a`) the REQUIRED darwin evaluator
returned `outcome=PASSED` while its leaderboard listed five candidates
in generation 2 (`g2_v0..g2_v4`). The bound existed only as prose;
nothing on the evaluator path detected the breach.

## Decision

The adapter layer gains a pure, dependency-free bound checker
(`packages/cli/src/darwinBounds.ts`) that parses leaderboard output,
counts candidates per generation label, and reports violations.
Parsing is fail-visible: zero rows parsed => `unparsable`, never `ok`.
Policy constants live in `DARWIN_BOUNDS` for one-line adjustment.

## Consequences

+ Silent bound breaches become machine-detectable; tonight's real
  leaderboard is pinned as a regression fixture in tests.
+ Fail-visible parsing guards against leaderboard format drift.
- Hard-wiring the checker into the live evaluator gate (auto-veto) is
  deliberately deferred to a human-reviewed follow-up.

Numbering: 0057 follows known-highest ADR-056; renumber on merge if the
sequence has advanced.
