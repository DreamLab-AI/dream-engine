---
id: NIT-01
title: The nightly loop and the run-state machine
area: nightly
governing: [docs/concepts.md]
adrs: [ADR-DE-007]
sources:
  - crates/dream-core/src/runstate.rs
  - crates/dream-core/src/lib.rs
  - crates/dream-core/src/persist.rs
  - crates/dream-gate/src/manifest.rs
  - docs/concepts.md
verified_commit: 4f5ac5324637b5e92eeceb00eab4bee2b94ae7eb
---
## For developers

One night is one experiment against one repository. This topic is the
restart-safety half of that: the `Phase` journal that `dream-core` writes beside
the frozen manifest, and the rules that decide, on every start, whether a night
is fresh, resumed, already finished or abandoned. After reading it you can
predict what a crash at any point costs.

The phases are an ordered enum so `>=` comparisons are meaningful
(`crates/dream-core/src/runstate.rs:33`). The ladder the concepts document
describes (`docs/concepts.md:20`) is `ManifestFrozen`, `BaselineEvaluated`,
`ModelCalled`, `CandidateEvaluated`, `Gated`; the code adds `Initialised`
before it and `Persisted`, `Complete` and `Abandoned` after it
(`crates/dream-core/src/runstate.rs:36`). The run id is a pure function of the
manifest fields (`crates/dream-gate/src/manifest.rs:202`), so a restart finds
its own journal. But the run id folds in the baseline HEAD, which the engine's
own ledger commit moves, so the terminal rule is night-level, not run-level
(`crates/dream-core/src/runstate.rs:158`, ADR-DE-007).

This crate ships the journal, not the driver that sequences the phases: the
crate docs list `runstate` among the pieces a host drives
(`crates/dream-core/src/lib.rs:21`).

## For the business

A nightly job that can be killed by a reboot has two bad outcomes: it silently
runs the same night twice, or it silently never finishes. The journal rules out
both. A finished night is never repeated by accident, an interrupted one gets
exactly one more try, and a night that keeps dying is parked loudly rather than
retried forever. The one deliberate way to rerun a finished night is an
operator action that archives, and never deletes, the old record.

## NIT-01.1 Phases of a night

```mermaid
stateDiagram-v2
    [*] --> Initialised
    Initialised --> ManifestFrozen
    ManifestFrozen --> BaselineEvaluated
    BaselineEvaluated --> ModelCalled
    ModelCalled --> CandidateEvaluated
    CandidateEvaluated --> Gated
    Gated --> Persisted
    Persisted --> Complete : normal close or finished_night on restart
    Initialised --> Abandoned : attempts exhausted
    ManifestFrozen --> Abandoned : attempts exhausted
    BaselineEvaluated --> Abandoned : attempts exhausted
    ModelCalled --> Abandoned : attempts exhausted
    CandidateEvaluated --> Abandoned : attempts exhausted
    Gated --> Abandoned : attempts exhausted
    Complete --> [*]
    Abandoned --> [*]
```

**What it shows.** The nine `Phase` values in progress order. `Persisted` is
entered before the draft pull request and ledger row are written, and
`Abandoned` is reachable from any non-terminal phase when `begin` finds the
attempt budget spent.
**Why it is this way.** `advance` only moves forward, so a resumed run
re-walking earlier steps cannot rewrite history
(`crates/dream-core/src/runstate.rs:268`). `Persisted` is deliberately entered
first, so a crash can lose a publish but never repeat one
(`crates/dream-core/src/runstate.rs:49`).
**Invariant:** `advance` never lowers the phase (`crates/dream-core/src/runstate.rs:269`).
**Tension:** the at-most-once ordering is a documented caller contract on
`persist` (`crates/dream-core/src/runstate.rs:276`); no code in this repository
sequences it, so nothing here enforces it, and a crash between `Persisted` and
the forge push loses that night's publish with no retry.

## NIT-01.2 What begin decides on every start

```mermaid
flowchart TB
    S["begin<br/>runstate.rs:212"] --> F{"finished_night<br/>runstate.rs:221"}
    F -->|"Complete or Persisted<br/>same repo, date, night id"| AC["AlreadyComplete<br/>do not re-run<br/>runstate.rs:222"]
    F -->|no| L{"prior journal with the same run id?<br/>runstate.rs:225"}
    L -->|no record or different run id| FR["Fresh<br/>phase Initialised, attempts 1<br/>runstate.rs:244"]
    L -->|yes| B{"Abandoned, or attempts at the budget?<br/>runstate.rs:226"}
    B -->|yes| AB["Abandoned<br/>record and move on<br/>runstate.rs:231"]
    B -->|no| RS["Resumed<br/>attempts plus 1, resumed_from set<br/>runstate.rs:234"]
```

**What it shows.** The four-way answer `begin` returns. Only `Fresh` and
`Resumed` say proceed (`crates/dream-core/src/runstate.rs:134`).
**Why it is this way.** The updated record is saved before `begin` returns, so
a crash straight after still counts the attempt and an interrupted run can
never pass as fresh (`crates/dream-core/src/runstate.rs:209`). The default
budget is two: one clean try plus one recovery (`crates/dream-core/src/runstate.rs:31`).
A different run id means the baseline or config moved, which is a new
experiment, and the manifest freeze has already archived the old document
(`crates/dream-core/src/runstate.rs:241`).
**Invariant:** a night already `Persisted` or `Complete` is `AlreadyComplete`
whatever the run id says (`crates/dream-core/src/runstate.rs:206`).

## NIT-01.3 The night-level terminal rule

```mermaid
sequenceDiagram
    participant D as Driver
    participant J as runstate
    participant G as ledger commit
    D->>J: persist before any external effect
    J->>J: phase Persisted, verdict recorded
    D->>G: ledger commit moves baseline HEAD
    Note over J,G: run id folds in baseline HEAD so it no longer matches
    D->>J: restart calls begin with the new run id
    J->>J: finished_night ignores run id and checks the night
    J-->>D: AlreadyComplete and Persisted becomes Complete
    D->>J: operator calls reset_night to rerun
    J->>J: rename run-state and manifest to superseded
```

**What it shows.** Why the rule is keyed on the night, not the run id: the
engine's own commit changes the id a restart computes.
**Why it is this way.** ADR-DE-007 records that a night is identified by its
directory (`crates/dream-core/src/runstate.rs:7`). `reset_night` renames the
two files and deletes nothing (`crates/dream-core/src/runstate.rs:188`).
Finalising `Persisted` to `Complete` is best effort and the answer does not
depend on the write landing (`crates/dream-core/src/runstate.rs:180`).
**Open:** the manifest archive on a moved baseline is named by run id
(`crates/dream-gate/src/manifest.rs:249`), the reset archive by timestamp
(`crates/dream-core/src/runstate.rs:194`); two naming schemes for superseded
documents.
