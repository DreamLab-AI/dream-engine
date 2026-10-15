---
id: ADR-DE-003
title: Protected-path guard and config-digest binding
date: 2026-10-15
decision_status: accepted
implementation_status: implemented
activation_status: staged
supersedes: []
owner: jjohare
review_trigger: a candidate patch that changed an evaluator's behaviour without touching a protected path, or a new class of file that decides pass or fail
---

# ADR-DE-003 — Protected-path guard and config-digest binding

## Context

The loop asks a model for a patch and then trusts the repository's own
evaluators to judge it. A patch that edits the evaluators, the config, or the
gate itself can make any change pass. Before this repository the Rust engine
had no protected-path guard, so the property rested on the model not trying.
The guard is new here.

## Decision

A protected set of paths is declared in the config and always includes the
config file itself, the evaluator scripts, the held-out dev set (see
[ADR-DE-004](ADR-DE-004-pairwise-selection-and-fingerprints.md)) and, for a
repository that contains the engine, the gate crates (self-referential
protection).

The guard runs twice: before a patch is applied, and again inside the gate
against the applied candidate tree. A violation is a `REJECT` through the
existing Evidence veto class, with the subject `protected-path`. No new enum
variant is added; the verdict space stays as it is.

The manifest records a digest of the config. If the digest at gate time differs
from the manifest, and the patch did not touch the config, something else
changed the config during the run. That is a harness veto and the verdict is
`BLOCKED-ENV`, not `REJECT`, because the candidate is not at fault.

## Consequences

A model cannot move the goalposts through the patch. It can still try, and the
attempt is recorded as a rejection with a named reason. The guard does not make
build scripts safe: they run on the evaluation host, which must be sandboxed
([SECURITY.md](../../SECURITY.md)). The guard and digest binding are implemented in `dream-gate`
(`guard::check`, `gate::decide` with its `GateContext`, the manifest digest) and
called by `dream_core::candidate::run`.
