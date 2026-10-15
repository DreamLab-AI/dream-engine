---
id: SEL-02
title: The failure-fingerprint lifecycle
area: selection
governing: [docs/adr/ADR-DE-004-pairwise-selection-and-fingerprints.md]
adrs: [ADR-DE-004]
sources:
  - crates/failure-fingerprint/src/lib.rs
  - crates/dream-gate/src/pertest.rs
  - crates/dream-core/src/streak.rs
  - docs/adr/ADR-DE-004-pairwise-selection-and-fingerprints.md
verified_commit: 0f6a66913c072603b6e29a80bdc67350737b9b21
---
## For developers

`failure-fingerprint` gives a failure a stable identity that survives volatile
text such as temp paths, line numbers and request ids, and classifies each step's
failures against the previous manifest as `New`, `Persisting` or `Fixed`
(`crates/failure-fingerprint/src/lib.rs:1`). The identity is the first 8
bytes of SHA-256 over task, stage and the normalised cause, joined by the unit
separator (`crates/failure-fingerprint/src/lib.rs:13`). In the gate, a baseline
manifest and a candidate step are classified, and any `New` record raises an
evidence veto (`crates/dream-gate/src/pertest.rs:253`).

## For the business

The question the nightly engine needs to answer is not "is anything red" but
"did this change cause a new kind of red". A suite may already be failing on
a known problem. Giving each failure a stable name lets the engine tell the
known failure, still there, from a brand new one that the change introduced.

## SEL-02.1 From text to identity

```mermaid
flowchart TB
    OBS["Observation: task, stage, cause text<br/>lib.rs:376"] --> NC["normalize_cause<br/>lib.rs:177"]
    NC --> P["paths become path placeholder"]
    NC --> I["long opaque ids become id placeholder"]
    NC --> D["digit runs become number placeholder"]
    NC --> WS["collapse whitespace, truncate to 200 chars<br/>lib.rs:190"]
    WS --> H["SHA-256 over task, 0x1F, stage, 0x1F, cause<br/>lib.rs:357"]
    H --> FP["first 8 bytes, 16 hex characters<br/>lib.rs:365"]
```

**What it shows.** Normalisation order is paths, ids, digits, whitespace, then
truncation (`crates/failure-fingerprint/src/lib.rs:190`).
**Why it is this way.** A separator byte between fields prevents two different
splits of the same characters from colliding
(`crates/failure-fingerprint/src/lib.rs:360`). As a side effect, long opaque
identifiers such as tokens never reach a persisted cause
(`crates/failure-fingerprint/src/lib.rs:15`).
**Tension:** the identity is 64 bits (`crates/failure-fingerprint/src/lib.rs:263`).
For a single suite the chance of an accidental collision is negligible, but a
collision between a new failure and a baseline failure would classify the new
one as `Persisting` and let it past the gate's new-failure veto, with no signal.
ADR-DE-004 lists a collision seen in practice as its review trigger.

## SEL-02.2 New, persisting, fixed

```mermaid
stateDiagram-v2
    [*] --> New : observed, absent from previous manifest, count 1
    New --> Persisting : observed again next step, count plus 1
    Persisting --> Persisting : observed again, count plus 1
    New --> Fixed : not observed next step
    Persisting --> Fixed : not observed next step
    Fixed --> [*] : dropped from the next manifest
    Fixed --> New : observed again later, count restarts at 1
```

**What it shows.** The lifecycle `advance` implements
(`crates/failure-fingerprint/src/lib.rs:550`). A persisting record keeps its
`first_seen_step` and increments its count
(`crates/failure-fingerprint/src/lib.rs:576`). A fixed record is a previous
record returned unchanged and absent from the next manifest
(`crates/failure-fingerprint/src/lib.rs:584`).
**Why it is this way.** Observations with the same fingerprint in one step are
de-duplicated, first wins (`crates/failure-fingerprint/src/lib.rs:562`). A
failure that disappears and returns is `New` again with its count reset
(`crates/failure-fingerprint/src/lib.rs:782`).
**Invariant:** the persisted cause is always the normalised text, never the raw
text (`crates/failure-fingerprint/src/lib.rs:410`).
**Debt:** `count` is the previous count plus one and not a strict consecutive
count; callers who need strictness check `prev.step + 1 == step`
(`crates/failure-fingerprint/src/lib.rs:539`).

## SEL-02.3 How the gate uses two steps

```mermaid
sequenceDiagram
    participant PT as pertest
    participant FF as advance
    PT->>FF: advance empty manifest with baseline failures, step 1, pertest.rs line 253
    FF-->>PT: baseline manifest
    PT->>FF: advance baseline manifest with candidate failures, step 2, pertest.rs line 254
    FF-->>PT: classification with new, persisting, fixed
    PT->>PT: any new record gives an evidence veto, pertest.rs line 255
    Note over PT,FF: persisting and fixed records never veto
```

**What it shows.** The gate runs the lifecycle for exactly two steps inside a
single evaluator, with the stage set to the evaluator's name
(`crates/dream-gate/src/pertest.rs:247`).
**Why it is this way.** The question is only whether the candidate introduces a
failure shape the baseline did not have, so a suite that was already red is
not penalised for its known failures (`crates/dream-gate/src/pertest.rs:6`).
## SEL-02.4 The cross-night use: harness stall detection

```mermaid
flowchart TB
    N["night ends BLOCKED-ENV with harness-class vetoes<br/>streak.rs:68"] --> A["advance_failures: only harness vetoes become observations<br/>streak.rs:74"]
    L["per-repo manifest file in the shared state directory<br/>streak.rs:94"] --> R["record_night loads it, corrupt or absent means an empty manifest<br/>streak.rs:124"]
    R --> A
    A --> C{"classification"}
    C -->|"BLOCKED-ENV, some persisting, none new, none fixed<br/>streak.rs:76"| ST["harness stuck notice<br/>streak.rs:136"]
    C -->|"anything else"| NS["not stuck"]
```

**What it shows.** The same crate used a second way: across nights, to notice
that the same environment fault has come back, so a broken testbed is
escalated to a person and does not just look like a quiet night.
**Why it is this way.** Only harness-class vetoes are fingerprinted, so ordinary
evidence against a candidate cannot look like a stuck harness
(`crates/dream-core/src/streak.rs:236`). A corrupt manifest fails open to an
empty one (`crates/dream-core/src/streak.rs:261`).
**Open:** the evidence-gate use in SEL-02.3 restarts from an empty manifest
every night, so it needs only 64 bits to separate two trees; the cross-night use
keeps records for as long as the failure persists, which is where a long-lived
collision would matter.
