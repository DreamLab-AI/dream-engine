---
id: ADR-DE-004
title: Pairwise selection and failure fingerprints
date: 2026-10-15
decision_status: accepted
implementation_status: in-progress
activation_status: staged
supersedes: []
owner: jjohare
review_trigger: a policy that selects a candidate later shown to regress, or a fingerprint collision seen in practice
---

# ADR-DE-004 — Pairwise selection and failure fingerprints

## Context

Comparing two result sets by mean score hides regressions and mistakes
infrastructure faults for evidence. A flaky service or a timeout should not
count as a loss for the candidate or a win for the baseline.

## Decision

`pairwise-gate` compares a candidate with a baseline on the same ordered list
of tasks. Its policies are `Margin`, `Floor`, `SignTest`, `WilsonLower`,
`NoRegression` and `TrainDev`. Each returns `Select`, `Reject` or
`Inconclusive` with the numbers behind it.

- A fault (service down, timeout, patch would not apply) is never a loss and
  never a win. Faulted pairings are excluded from the tally and reported.
- The sign test and the Wilson bound are used only with at least 20 per-test
  cases. An evaluator with no declared per-test format counts as one case, so it
  never reaches them. Below that, the per-test evidence is too thin for either bound to
  mean much.
- `NoRegression` rejects any pairing that passed on the baseline and fails on
  the candidate, whatever happens elsewhere.
- `TrainDev` selects when the training set selects and the held-out dev set
  does not reject. The held-out dev set is made of evaluators left out of the
  prompt shown to the model. They are read-denied to the model and protected
  under [ADR-DE-003](ADR-DE-003-protected-path-guard.md).

`failure-fingerprint` gives each failure a stable identity: SHA-256 (the
`sha2` crate) over the task, the stage and the normalised cause, truncated to
16 hex characters. Across steps it classifies a failure as `New`,
`Persisting` or `Fixed`.

The exact binomial and Wilson bounds were checked against reference values
computed independently in exact arithmetic: 9 wins of 10 gives p = 11/1024,
and Wilson for 8 of 10 at z = 1.96 gives a lower bound of 0.4902.

## Implementation status

`pairwise-gate` and `failure-fingerprint` are implemented and tested, the gate
applies `NoRegression`, `Floor`, `Margin`, `SignTest` and `WilsonLower` to
per-test evidence, and `dream-core::streak` uses fingerprints to raise `HarnessStuck`. Status stays **in-progress**: `TrainDev` and held-out dev evaluators
are not wired into the gate's per-test path, and no consumer supplies them yet.

## Consequences

A broken testbed shows up as faults and `Inconclusive`, not as a verdict on the
candidate. The 20-case floor means small test suites should lean on `NoRegression`
and `Margin` instead. Sixteen hex characters is 64 bits, enough to tell failure modes
apart within one repository, not a collision-resistant identifier across
repositories.
