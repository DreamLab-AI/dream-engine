# dream-core

The nightly loop around [`dream-gate`](../dream-gate): everything that touches
a filesystem, a subprocess or a model, so that `dream-gate` can stay pure.
A host (a scheduler, an agent platform) plugs in five small seams and gets an
evidence-gated, restart-safe, replayable evolution loop. Install with
`cargo add dream-core`; the offline `dream` command is the separate
[`dream-cli`](../dream-cli) crate.

| Module | Role |
|---|---|
| `runner` | `EvaluatorRunner` (sync, dyn-safe), `LocalRunner` (`bash -o pipefail` under `timeout`), `ScriptedRunner`. |
| `candidate`, `persist` | Candidate worktree, patch application strategies, required-evaluator rerun. **`candidate::run` guards before it applies** — refuse, guard, worktree, build, evaluate. |
| `testfmt` | Per-test outcomes from `libtest`, JUnit XML and Vitest JSON output. |
| `ledger` | Append-only markdown ledger; TS-parity reader (`parse_ledger`, `verify_ledger`, `validate_ledger`, `learning_signals`, `verdict_stats`). |
| `streak` | Harness-stall detection across nights (`HarnessStuck`) and the dry streak. |
| `replay` | Capture a night's gate inputs; `verify` re-runs the pure gate and diffs it against the recorded verdict. |
| `source`, `context` | Tree-read: the source a hypothesis touches, and context governance for the evidence pack — both behind the `Llm` trait. |
| `compile` | Deterministic prompt compilation with a neutral built-in template. |
| `readiness`, `runstate`, `roster`, `sweep` | Admission, restart-safe run identity and night idempotency, fair rotation, branch ageing. |
| `automerge` | The pure automerge decision (default off). |
| `seams` | `Llm`, `MemorySink` (`NullSink`, `JsonlSink`), `Notifier` (infallible; `StderrNotifier`), `Forge` (`GhForge`). |

No async runtime is a dependency: the async traits return a plain-`std`
`BoxFut`. Nothing in the crate reads the process environment; every knob is a
parameter. `#![forbid(unsafe_code)]`.

## Nights and automerge

A night directory whose journal reached `Persisted` or `Complete` is terminal
whatever the run id (`runstate::finished_night`). `runstate::persist` records the
verdict before the ledger commit and the forge request, so publishing is
at-most-once: a crash can lose a publish, never repeat one.
`runstate::reset_night` archives a finished night; it never deletes
([ADR-DE-007](../../docs/adr/ADR-DE-007-a-night-is-identified-by-its-directory.md)).

Automerge is the config key `autoMerge` (alias `automerge`), default off. It is
refused for self-referential targets and unless the pull request's real base
branch has required status checks; then `gh pr ready` and
`gh pr merge --auto --squash` run. That forge step has not yet been exercised
against real GitHub.

The dry streak (`streak::next_dry_streak`) advances on `REJECT` and
`INCONCLUSIVE`, resets on `ACCEPT`, and ignores `BLOCKED-ENV`, `HANDOFF` and
`OPERATOR`. A repeated identical `BLOCKED-ENV` raises `Notice::HarnessStuck`.

An evaluator with no declared `testFormat` counts as one case, so it gets no sign
or Wilson statistics.

## Order of operations for a candidate

```text
deletes a binary?  -> Refused         (nothing run)
guard::check       -> Protected / Refused   (no git command, build or evaluator ran)
git worktree + apply (guarding the normalised text too)
build step -> required evaluators -> CandidateState::Applied { patch }
dream_gate::gate::decide(...)       re-checks the same patch
```

## Reading per-test output

Declare `"testFormat": "libtest" | "junit" | "vitest-json"` on an evaluator in
the config; `testfmt::outcomes` turns the baseline and candidate executions
into the gate's `TestOutcomes`, or `None` (receipt-only judging) when either
side cannot be read. Evaluators piped through `| tail -N` lose most test lines:
declare a format only where the full output reaches the parser.

## Replay

`replay::capture(run_dir, &night)` judges a night and persists manifest,
receipts, report, config, patch, per-test evidence, the policy additions and the
decision in a stable schema (schema 2); `replay::verify(run_dir)` recomputes the
verdict from those files and reports any difference, including a receipt whose
typed outcome contradicts its own exit status.

The protected set is rebuilt the way the live night built it: build the
policy with `replay::night_policy(manifest, config_bytes, fallback_ledger,
&PolicyExtras { self_referential, extra_globs })`, give that value to
`candidate::run`, and put the same `PolicyExtras` in `NightInputs::policy`.
`verify` then judges under the same protected set, including the
self-referential flag. A schema 1 record does not say which policy the night
used and is refused with an error saying so; recapture it.

It shows that the stored inputs and decision agree, not that they are
untampered: only the report and the config are digest-bound. There is no
historical corpus: only nights captured after a host adopts this module are
replayable.

`candidate::run` returns an `Attempt` whose `error` is a typed `AttemptError`
(`PatchDidNotApply { stderr }`, `Provisioning`, `Prepare`), and
`seams::MemoryRecord` has optional `deep`, `night_date`, `witness` and `source`
fields. Both additions are source-breaking for code that builds those structs
with a literal (use `..Default::default()` for the record).

## Licence

MIT OR Apache-2.0.
