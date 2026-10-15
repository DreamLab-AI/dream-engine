# Comparison with REEF

[REEF](https://github.com/Human-Agent-Society/reef) (Apache-2.0) is the nearest
system we know of. This page is our reading of its public description, and it
may be out of date; check REEF's own documentation before relying on it.

## What REEF does

REEF evolves a harness as a tree. Candidate harnesses are evaluated on paired
episodes, and a change is published or reverted according to the result.

## Where dream-engine is similar

Both compare a changed system with the one it came from on the same tasks, and
both decide from the evidence whether to keep the change.

## What we do differently

| Area | dream-engine |
|---|---|
| Experiment identity | The manifest is frozen before any model call and the run id is a pure function of it, so a restart resumes the same experiment and results are not compared across a silent change |
| Environment faults | `BLOCKED-ENV` is a separate verdict from `REJECT`, and faults are excluded from the tally rather than counted as losses |
| Statistics | Exact one-sided binomial and Wilson lower bounds, used only with at least 20 per-test cases, plus `NoRegression` for any pass-to-fail pairing |
| Who merges | The default outcome is a draft pull request that a human merges; automerge is opt-in and bounded ([ADR-DE-006](adr/ADR-DE-006-automerge-is-an-option.md)) |
| Scope | One patch a night against one repository, not a population of harnesses |

## What REEF does that we plan

Post-publish recheck and rollback: re-evaluating a change after it has landed
and reverting it if it no longer holds. Periodic re-screening of past accepted
changes. Neither is in dream-engine yet.

## Choosing

If you want a harness population evolved over many episodes, look at REEF. If
you want one conservative proposal a night against your own repository, with
the verdict separated from the health of the testbed, dream-engine is built for
that.
