# Concepts: the nightly loop

dream-engine runs one experiment a night against one repository. An experiment
asks a single question: is this patched tree better than the tree we started
with, according to the repository's own evaluators?

## The manifest

Before any model call the engine freezes an experiment manifest and writes it
atomically. It records the baseline revision, the tree hash, the sha256 of each
evaluator command, the model identity and a digest of the config. The run id is
a pure function of those fields. Two consequences follow. A restart resumes the
same experiment rather than starting a new one, and changing an evaluator
command or the model produces a different run, so results are never compared
across a silent change. If the baseline has diverged from what an earlier run
recorded, the old record is archived. It is never overwritten.

## The run

A run moves through five states in order.

| State | What has happened |
|---|---|
| `ManifestFrozen` | The manifest is on disk; nothing has been evaluated |
| `BaselineEvaluated` | The unmodified tree has been run through every evaluator |
| `ModelCalled` | The model has returned a patch |
| `CandidateEvaluated` | The patched tree has been run through the same evaluators on the same testbed |
| `Gated` | The gate has returned a verdict |

Baseline first, then the model, then the candidate, on the same testbed. The
baseline is measured in the same conditions as the candidate, so a slow or
broken testbed affects both.

## The gate

The gate is a pure function of the two evaluator result sets and the manifest.
It calls no model and reads no clock. It is built from the selection policies
in [`pairwise-gate`](../crates/pairwise-gate) and the vetoes of the planned
`dream-gate`.

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
The guard is planned.

## After the gate

An `ACCEPT` opens a draft pull request and a human merges it. Automerge exists
as an option, off by default, with conditions
([ADR-DE-006](adr/ADR-DE-006-automerge-is-an-option.md)). The five seams
(`Llm`, `MemorySink`, `EvaluatorRunner`, `Notifier`, `Forge`) are how a product
supplies its own model, memory, test runner, notifications and forge
([ADR-DE-005](adr/ADR-DE-005-seams.md)).

## Failure identity

[`failure-fingerprint`](../crates/failure-fingerprint) gives each failure a
16-hex-character SHA-256 identity over its task, stage and normalised cause, and
classifies it as `New`, `Persisting` or `Fixed` across steps. A failure that
persists from night to night is a different signal from one that has just
appeared.
