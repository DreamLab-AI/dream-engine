# dream-core

The nightly loop around [`dream-gate`](../dream-gate): everything that touches
a filesystem, a subprocess or a model, so that `dream-gate` can stay pure.
A host (a scheduler, an agent platform) plugs in four small seams and gets an
evidence-gated, restart-safe, replayable evolution loop.

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
| `readiness`, `runstate`, `roster`, `sweep` | Admission, restart-safe run identity, fair rotation, branch ageing. |
| `seams` | `Llm`, `MemorySink` (`NullSink`, `JsonlSink`), `Notifier` (infallible; `StderrNotifier`), `Forge` (`GhForge`). |

No async runtime is a dependency: the async traits return a plain-`std`
`BoxFut`. Nothing in the crate reads the process environment; every knob is a
parameter. `#![forbid(unsafe_code)]`.

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
receipts, report, config, patch, per-test evidence and decision in a stable
schema; `replay::verify(run_dir)` recomputes the verdict from those files and
reports any difference, including a receipt whose typed outcome contradicts its
own exit status. There is no historical corpus: only nights captured after the
host adopts this module are replayable.

## Licence

MIT OR Apache-2.0.
