---
id: ADR-DE-005
title: Seams — Llm, MemorySink, EvaluatorRunner, Notifier, Forge
date: 2026-10-15
decision_status: accepted
implementation_status: planned
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

The traits return boxed futures so they are dyn-compatible: a consumer can hold
`Box<dyn Llm>` and choose implementations at run time without the engine
depending on an async trait macro.

`Notifier` is infallible by signature. A failure to notify must never change a
verdict or stop a run, so it is fail-open: the implementation absorbs and logs
its own errors.

## Consequences

Product code stays in the consumer. The cost is that each seam is a public
contract that is versioned. The other four seams are fallible and their errors
are reported in the run state, not swallowed. The traits are planned.
