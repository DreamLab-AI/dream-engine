---
id: ADR-DE-007
title: A night is identified by its directory, not by the run id
date: 2026-10-15
decision_status: accepted
implementation_status: implemented
activation_status: staged
supersedes: []
owner: jjohare
review_trigger: a second caller that commits to the repository during a night, or a change to what `run_id` hashes
---

# ADR-DE-007 — A night is identified by its directory, not by the run id

## Context
`run_id` hashes repo, date, deep slot, baseline HEAD and config digest. The engine's own ledger commit moves HEAD, so after a finished night the next start computed a different id, `runstate::begin` saw a mismatch and started a fresh run: a second model call, ledger row and publish request. A crash between the ledger commit and `complete` opened the same hole.

## Decision
- The terminal rule is night-level. `runstate::finished_night` (and `begin`) treat a prior journal for the same repo, date and night id in `Persisted` or `Complete` as finished, whatever the run id. A `Persisted` journal is finalised to `Complete` with its recorded verdict.
- Phases below `Persisted` keep the old rule: a different run id means the baseline moved, so the run is fresh.
- `Persisted` is entered *before* the externally visible effects (forge request, ledger commit) by `runstate::persist`, which records the verdict. A crash afterwards can lose a publish, never repeat one (at-most-once). A failed ledger commit after that point does not retry on its own.
- Re-running a finished night is an explicit act: `runstate::reset_night` renames the run-state and manifest to `*-superseded-<timestamp>` (nothing is deleted).
- `run_id` guarantees a stable identity only while HEAD is unmoved; its rustdoc says so.

## Consequences
- The engine must check `finished_night` before freezing a manifest, so a skipped night neither rewrites the manifest nor spends anything.
- A night whose publish was lost to a crash is not retried automatically; the operator resets it.
