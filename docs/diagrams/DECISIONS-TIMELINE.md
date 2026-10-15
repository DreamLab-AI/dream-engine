# Decisions timeline

The register's sibling view: the decision records in the order they were
written, and the code changes that moved a decision after it was recorded.
Hand-written; the generator does not produce it. Dates are commit dates from
`git log` up to the stamped revision.

## 2026-10-15

| When | Commit | Decision | Later |
|---|---|---|---|
| 12:15 | `1a16ad2` | Workspace skeleton with the two leaf crates, `pairwise-gate` and `failure-fingerprint`. | — |
| 12:18 | `1d5394c` | ADR-DE-001 to ADR-DE-006 recorded: standalone permissive engine, licence boundary, protected-path guard, pairwise selection and fingerprints, five seams, automerge off by default. | README still calls `dream-gate` Planned at the stamped revision (CRT-01.1). |
| 12:30 | `79df0ba` | `dream-gate` built: verdict, manifest, receipts, guard, per-test evidence and the gate (GAT-01). | — |
| 12:55 | `d2e8f48` | `candidate::run` fixes the order refuse, guard, worktree, build, evaluate (GAT-01.3). | — |
| 13:04 | `a89bfc0` | Replay capture and verify with a stable on-disk schema (NIT-02). | `ce4811e` (13:56) bumps it to schema 2 so the live policy is recorded; schema 1 is then refused. |
| 13:16 | `2f8e496` | Automerge policy, default off (NIT-03). | `cd40f83` (13:23) reads protection of the PR's real base branch and un-drafts before `--auto`. |
| 13:19 | `2662980` | `dream` command-line binary over the pure checks. | — |
| 20:00 | `4f5ac53` | ADR-DE-007: a night is identified by its directory, not by the run id; persist before external effects; `reset_night` (NIT-01). | The stamped revision of this corpus. |
