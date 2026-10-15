# Concepts: the nightly loop

dream-engine runs one experiment a night against one repository. An experiment
asks a single question: is this patched tree better than the tree we started
with, according to the repository's own evaluators?

## Manifest

Before any model call the engine freezes an experiment manifest and writes it
atomically. It records the baseline revision, the tree hash, the sha256 of each
evaluator command, the model identity and a digest of the config. The run id is
a pure function of those fields. Two consequences follow. A restart resumes the
same experiment rather than starting a new one, and changing an evaluator
command or the model produces a different run, so results are never compared
across a silent change. If the baseline has diverged from what an earlier run
recorded, the old record is archived. It is never overwritten.

## Run states

A run moves through these states in order. The last two are bookkeeping around
the external effects.

| State | What has happened |
|---|---|
| `ManifestFrozen` | The manifest is on disk; nothing has been evaluated |
| `BaselineEvaluated` | The unmodified tree has been run through every evaluator |
| `ModelCalled` | The model has returned a patch |
| `CandidateEvaluated` | The patched tree has been run through the same evaluators on the same testbed |
| `Gated` | The gate has returned a verdict |
| `Persisted` | The verdict is recorded on disk; written before the ledger commit and the forge request |
| `Complete` | The night is finished |

Baseline first, then the model, then the candidate, on the same testbed. The
baseline is measured in the same conditions as the candidate, so a slow or
broken testbed affects both.

## Gate

The gate is a pure function of the two evaluator result sets and the manifest.
It calls no model and reads no clock. It is built from the selection policies
in [`pairwise-gate`](../crates/pairwise-gate) and the vetoes of
[`dream-gate`](../crates/dream-gate).

An evaluator that is missing, silent, blocked, timed out, explicitly failing or
exiting non-zero vetoes. Vetoes come in three classes, and the class decides
the verdict:

| Veto class | Meaning | Verdict |
|---|---|---|
| Harness | The testbed failed, not the candidate | `BLOCKED-ENV` |
| Evidence | The evidence counts against the candidate | `REJECT` |
| Unproven | The evidence is too thin to decide | `INCONCLUSIVE` |

If nothing vetoes and the selection policy selects, the verdict is `ACCEPT`.

Keeping `BLOCKED-ENV` apart from `REJECT` is the point of the class split. A
run where the test database was unreachable says nothing about the patch, and
it should not teach anyone that the patch was bad. For the same reason
[`pairwise-gate`](../crates/pairwise-gate) excludes infrastructure faults from
the win and loss tally.

## What the gate guards against

A candidate may not change the files that decide whether it passes. The
protected-path guard runs before the patch is applied and again inside the gate;
a violation is a `REJECT` with subject `protected-path`. If the config digest at
gate time differs from the manifest and the patch did not touch the config, the
run is `BLOCKED-ENV`. See [ADR-DE-003](adr/ADR-DE-003-protected-path-guard.md).
The guard is implemented in `dream_gate::guard`, and `dream_core::candidate::run`
calls it before any git command, build or evaluator runs. The same check runs
inside `gate::decide` through its `GateContext`, as defence in depth. `dream guard
check` runs it from the command line.

## Per-test evidence

An evaluator may declare a `testFormat` (`libtest`, `junit` or `vitest-json`).
`dream_core::testfmt` then reads per-test outcomes from the baseline and
candidate output, and the gate can apply the sign test and Wilson bound. An
evaluator without a format counts as one case, so it gets no sign or Wilson
statistics, only its pass or fail.

## Nights are idempotent

A night directory whose journal reached `Persisted` or `Complete` is terminal,
whatever the run id, so the engine's own ledger commit (which moves HEAD) cannot
cause a second model call or a second pull request. `Persisted` is written
before the ledger commit and the forge request, so publishing is at-most-once: a
crash can lose a publish but never repeat one. `reset_night` archives a finished
night; it never deletes ([ADR-DE-007](adr/ADR-DE-007-a-night-is-identified-by-its-directory.md)).

## Across nights

The dry streak counts trailing nights that ended `REJECT` or `INCONCLUSIVE`.
`ACCEPT` resets it. `BLOCKED-ENV`, `HANDOFF` and `OPERATOR` neither advance nor
reset it. Separately, a `BLOCKED-ENV` night whose harness failures are exactly
last night's raises a `HarnessStuck` notice, because more nights will not fix a
testbed.

## Replay

`dream_core::replay` captures the gate inputs of every night that reaches the
gate (schema 2, including the protected set the night used). `dream replay
verify` recomputes the verdict from those files with the same protected set. It
shows that stored inputs and decision agree. It is not tamper evidence.

## After the gate

An `ACCEPT` opens a draft pull request and a human merges it. Automerge exists
as an option (`autoMerge`, alias `automerge`), off by default. It is refused for
self-referential targets and unless the pull request's real base branch has
required status checks; then the engine runs `gh pr ready` followed by
`gh pr merge --auto --squash`
([ADR-DE-006](adr/ADR-DE-006-automerge-is-an-option.md)). That forge step has not
yet been exercised against real GitHub. The five seams
(`Llm`, `MemorySink`, `EvaluatorRunner`, `Notifier`, `Forge`) are how a product
supplies its own model, memory, test runner, notifications and forge
([ADR-DE-005](adr/ADR-DE-005-seams.md)).

## Failure identity

[`failure-fingerprint`](../crates/failure-fingerprint) gives each failure a
16-hex-character SHA-256 identity over its task, stage and normalised cause, and
classifies it as `New`, `Persisting` or `Fixed` across steps. A failure that
persists from night to night is a different signal from one that has just
appeared. Sixteen hex characters is 64 bits: enough to tell failure modes apart
within one repository, not collision-resistant across repositories.
