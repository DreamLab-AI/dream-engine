---
id: NIT-03
title: The automerge policy decision tree
area: nightly
governing: [docs/adr/ADR-DE-006-automerge-is-an-option.md]
adrs: [ADR-DE-006]
sources:
  - crates/dream-core/src/automerge.rs
  - crates/dream-core/src/persist.rs
  - crates/dream-core/src/seams.rs
  - docs/adr/ADR-DE-006-automerge-is-an-option.md
verified_commit: 4f5ac5324637b5e92eeceb00eab4bee2b94ae7eb
---
## For developers

An ACCEPT says the repository's evaluators did not object, not that the
change is wanted, so the default is a draft pull request a human merges
(`crates/dream-core/src/automerge.rs:3`). `automerge::decide` is a pure
function that turns facts the caller gathered into `Disabled`, `Refused` or
`Allowed` (`crates/dream-core/src/automerge.rs:110`). `persist::publish`
gathers the facts, calls it, and only then asks the forge to enable
automerge (`crates/dream-core/src/persist.rs:298`).

## For the business

Out of the box, nothing merges itself: each morning there is a draft change
waiting for a person. A repository can opt in to unattended merges, but only
if it already has branch protection with required checks, only for ordinary
changes (never to the engine itself) and only if a clean draft was opened. If
the engine cannot read the protection rules for any reason, it treats that as
no, not as yes.

## NIT-03.1 The decision tree

```mermaid
flowchart TB
    S["decide<br/>automerge.rs:110"] --> A{"cfg.auto_merge on?<br/>automerge.rs:111"}
    A -->|no, the default| DIS["Disabled"]
    A -->|yes| B{"engine repo, protects gate crates, or contains them?<br/>automerge.rs:114"}
    B -->|yes| R1["Refused SelfReferential"]
    B -->|no| C{"verdict is ACCEPT?<br/>automerge.rs:120"}
    C -->|no| R2["Refused NotAccepted"]
    C -->|yes| D{"clean draft: pushed and PR url?<br/>automerge.rs:123"}
    D -->|no| R3["Refused PrNotClean"]
    D -->|yes| E{"required status checks<br/>automerge.rs:126"}
    E -->|"error, 404 or unsupported"| R4["Refused ChecksUnknown"]
    E -->|"list empty or blank"| R5["Refused NoRequiredChecks"]
    E -->|"at least one name"| OK["Allowed"]
```

**What it shows.** Five refusal reasons in a fixed order, ahead of the one
`Allowed` leaf. The order matters for what is reported, not for the outcome.
**Why it is this way.** `gh pr merge --auto` merges at once when nothing is
required, so an empty list, a 404 and any API error are refusals
(`crates/dream-core/src/automerge.rs:13`). The self-referential test has
three parts so the engine never merges changes to itself: the slug, a config
whose protected set covers the gate crates, and a checkout that contains them
(`crates/dream-core/src/automerge.rs:136`, `crates/dream-core/src/automerge.rs:144`).
The flag lives in the protected config, so a candidate patch cannot switch it
on (`docs/adr/ADR-DE-006-automerge-is-an-option.md:30`).
**Invariant:** `Disabled` is returned before any other fact is consulted
(`crates/dream-core/src/automerge.rs:111`).

## NIT-03.2 Gathering the facts in publish

```mermaid
sequenceDiagram
    participant P as publish
    participant F as Forge
    participant D as automerge decide
    P->>F: publish_draft, persist.rs line 305
    F-->>P: pushed and PR url
    P->>P: draft clean means pushed with a url, persist.rs line 306
    alt auto_merge on and draft clean
        P->>F: pr_base_branch of the PR, persist.rs line 314
        F-->>P: actual base branch
        P->>F: required_status_checks of that branch only if it matches, persist.rs line 316
    else flag off
        P->>P: required checks left unsupported, branch protection never queried
    end
    P->>D: decide with the gathered facts, persist.rs line 329
    D-->>P: Allowed
    P->>F: enable_automerge, persist.rs line 332
    F-->>P: ok or error kept in automerge_error
```

**What it shows.** With the flag off no protection query is made at all
(`crates/dream-core/src/persist.rs:293`). With it on, protection is read for
the branch the PR really targets, and a mismatch with the expected base is a
refusal (`crates/dream-core/src/persist.rs:318`).
**Why it is this way.** A forge that cannot say what protects a branch never
gets automerge: the trait defaults return `Unsupported`, which the policy
treats as a refusal (`crates/dream-core/src/seams.rs:377`).
**Tension:** the forge step is untested against live GitHub. The only test
touching `GhForge` is a dyn-safety check (`crates/dream-core/src/seams.rs:647`),
so the protection parsing, the draft-to-ready transition and the squash
request have only been exercised against fakes.
**Open:** a forge failure after `Allowed` is recorded in `automerge_error` and
the pull request stays a draft (`crates/dream-core/src/persist.rs:284`); there
is no retry.
