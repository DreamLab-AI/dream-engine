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
- Re-running a finished night is an explicit act: `runstate::reset_night` archives the run-state and manifest as `*-superseded-<nanosecond stamp>-<pid>-<counter>` by hard link (it can never replace an earlier archive; both files are archived or neither; nothing is deleted).
- `run_id` guarantees a stable identity only while HEAD is unmoved; its rustdoc says so.

## Consequences
- The engine must check `finished_night` before freezing a manifest, so a skipped night neither rewrites the manifest nor spends anything.
- A night whose publish was lost to a crash is not retried automatically; the operator resets it.

## Hardening (independent audit, 2026-10)
- **One engine per night.** `runstate::NightLock` takes an exclusive advisory lock (`File::try_lock`, stable from Rust 1.89, hence the workspace MSRV) on `<night_dir>/.lock` before the first journal read and holds it through persist. A second engine gets `LockError::Held` and skips the night; `reset_night` takes the same lock. The lock dies with the process, so a crash never leaves it stuck.
- **Absent is not damaged.** `load_checked` returns `Ok(None)` only for a missing file; unreadable, truncated or unknown-schema journals are `JournalError`s, and `begin`/`finished_night` return them rather than starting fresh. The engine reports `BLOCKED-ENV` for that night and points at `reset-night`; the damaged file is left as evidence. `load` remains as a lossy wrapper. Journals the previous binary wrote (schema 1, camelCase fields) are covered by fixtures.
- **The budget is the night's.** A journal for the same repo/date/night id with a different run id starts a fresh experiment at `Initialised` but keeps counting attempts; `Abandoned` stays abandoned until `reset-night`. A journal recording another repo, date or night id is `JournalError::WrongNight`.

## Known limit: publish outcomes are not reconciled (at-most-once)
`Persisted` is journalled **before** the forge request and ledger commit, and nothing records their outcomes afterwards. Consequences, stated exactly:
- A crash after `Persisted` and before the pull request exists loses that publish permanently: the next start finalises the night to `Complete`.
- A crash after the pull request is created but before its URL or ledger row is recorded likewise leaves no record; the restart still finalises to `Complete` without reconciling against the forge.
- Duplicate publication is impossible by construction (the cost of the first two points). Recovery is manual: `reset-night`, which archives the journal and manifest so the next run is fresh.
- The idempotency key for any future reconciliation is the night key: `(repo, date, night id)`; within one repository the pull-request branch `dream/<deep>-<date>` (`persist::branch_name`) identifies the same night. A forge-side lookup by either is the natural basis if exactly-once is ever wanted; it is deliberately not built.
