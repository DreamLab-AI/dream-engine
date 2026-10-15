---
id: ADR-DE-005
title: Seams — Llm, MemorySink, EvaluatorRunner, Notifier, Forge
date: 2026-10-15
decision_status: accepted
implementation_status: implemented
activation_status: staged
supersedes: []
owner: jjohare
review_trigger: a consumer that needs a sixth seam, or a seam whose error type leaks a product detail
---

# ADR-DE-005 — Seams

## Context

Everything a product knows that the loop should not (which model, where memory
lives, how tests are run, where notices go, which forge hosts the pull
request) must be replaceable without forking the engine.

## Decision

`dream-core` is generic over five traits.

| Trait | Responsibility |
|---|---|
| `Llm` | Produce one patch (and report model identity) |
| `MemorySink` | Record what the run learned |
| `EvaluatorRunner` | Run an evaluator command against a tree and return its result |
| `Notifier` | Tell a person what happened |
| `Forge` | Open the draft pull request, and merge only if ADR-DE-006 allows |

`EvaluatorRunner` is synchronous and dyn-safe. The other traits return boxed futures so they are dyn-compatible: a consumer can hold
`Box<dyn Llm>` and choose implementations at run time without the engine
depending on an async trait macro.

`Notifier` is infallible by signature. A failure to notify must never change a
verdict or stop a run, so it is fail-open: the implementation absorbs and logs
its own errors.

## Consequences

Product code stays in the consumer. The cost is that each seam is a public
contract that is versioned. The other four seams are fallible and their errors
are reported in the run state, not swallowed. The traits live in `dream-core::seams` (`EvaluatorRunner` in `dream_core::runner`),
with built-in implementations `NullSink`, `JsonlSink`, `StderrNotifier`,
`GhForge`, `LocalRunner` and `ScriptedRunner`.

Two boundaries are deliberate, and a consumer should not fight them:

- `Notice` carries per-event notices only (`NightSummary`, `LostPr`,
  `HarnessStuck`, `Accepted`). Publishing a whole night's state (a relay post,
  a digest, a governance report) is a night-level driver in the host, not a
  `Notifier`; no `Notice` variant exists for them because no seam consumer
  needs one.
- `MemoryRecord` has typed optional `deep`, `night_date`, `witness` and
  `source` fields, omitted from the serialised form when absent, so a record
  written before they existed reads back and re-serialises unchanged.
- `candidate::Attempt` carries a typed `AttemptError`
  (`PatchDidNotApply { stderr }`, `Provisioning`, `Prepare`) next to the
  `CandidateState` text, so a host branches on data, not on a string prefix.
