# dream-engine diagram corpus

A hand-authored, citation-verified map of the engine: eight topic files, each
explained twice (for whoever inherits the code, and for whoever owns the
product), around Mermaid diagrams whose every mechanism claim cites
`path:line` at the revision stamped in the topic. It catalogues the state of
play and proposes no fixes; [REGISTER.md](REGISTER.md) is the input to later
work, not a plan.

**Stamped revision:** `815f963b1cbd6999e78144ce5475005ae1dd1d8e`, the source revision assessed on 1 November 2026. All eight topics and 24 panels were reviewed, including the unchanged selection sources. The carrying documentation commit is deliberately not the source pin. The previous pin was `0f6a66913c072603b6e29a80bdc67350737b9b21`.

This pass adds the formatter stage, corrects gate branching and automerge refusal, and qualifies execution, recovery and receipt guarantees. It also records fingerprint aliases, normalisation limits and repeated-call streak behaviour. The register contains implementation findings, not completed fixes. Local fixtures demonstrate the fingerprint and streak findings; prior candidate, recovery and forge probes inform their corresponding topics. The dated estate report and retained evidence live in VisionFlow's `docs/estate-review/2026-11-01-dream-diagram-corpus.md`.

Governing documents: [`docs/concepts.md`](../concepts.md) and [`docs/adr/`](../adr/README.md). Coverage is the eight-topic map, not every module or runtime integration in the engine.

## Doors

- **New to the code?** NIT-01 (run state) → GAT-01 (the gate) → CRT-01 (crates) →
  CRT-02 (seams) → NIT-02 (replay) → SEL-01 and SEL-02.
- **Deciding whether to run it unattended?** The "For the business" sections of
  NIT-01, NIT-03 and GAT-01, then [REGISTER.md](REGISTER.md).
- **Looking for a source file?** [COVERAGE.md](COVERAGE.md) is indexed by diagram,
  record, governing document and source path.
- [DECISIONS-TIMELINE.md](DECISIONS-TIMELINE.md): the records in time order.

## Topics

| Area | Topic | Subject |
|---|---|---|
| nightly | NIT-01 | the nightly loop, the run-state machine, the night-level terminal rule, journal errors, the night lock and the reset archive |
| nightly | NIT-02 | replay capture and verify |
| nightly | NIT-03 | the automerge decision tree |
| gate | GAT-01 | verdict classes, `decide`, receipts validated against the manifest, guard-before-apply, guard resource bounds and evaluator-command paths, config-digest binding |
| crates | CRT-01 | the crate DAG and the licence boundary |
| crates | CRT-02 | the five seams and what implements them |
| selection | SEL-01 | pairwise policies and the fault and Inconclusive mapping |
| selection | SEL-02 | the failure-fingerprint lifecycle |

## Checking and re-stamping

```bash
# structure, citations at the stamped revision, Pele grammar (vendored, no browser) and the 4500px limit
node docs/diagrams/tools/diagram-index-gen.cjs docs/diagrams --check --cite-check --render
# regenerate README index, COVERAGE.md and REGISTER.md
node docs/diagrams/tools/diagram-index-gen.cjs docs/diagrams
# JSON report for another consumer
node docs/diagrams/tools/diagram-index-gen.cjs docs/diagrams --check --cite-check --render --report docs/diagrams/diagrams.report.json
```

To re-stamp after the code moves, first re-read each topic against the new
revision (re-derive every line from its symbol, never from a diff offset), then
bump the stamp and re-run the gate. The bump is one command:

```bash
sed -i "s/^verified_commit: .*/verified_commit: $(git rev-parse HEAD)/" docs/diagrams/*/*.md \
  && node docs/diagrams/tools/diagram-index-gen.cjs docs/diagrams --check --cite-check --render
```

Do not run it blind: unchanged sources can still have incorrect diagrams or unsupported claims, so every panel needs semantic assessment before a new stamp. The
generator is the copy shipped with the `diagrams-as-code` skill, unmodified.
`rendered/` is gitignored and regenerable.



## Diagram index

<!-- BEGIN GENERATED DIAGRAM INDEX -->
_8 topic files, 24 diagrams. Regenerate with_ `node docs/diagrams/tools/diagram-index-gen.cjs docs/diagrams`.

### nightly

| ID | Topic | Diagrams | Kinds | Governing | ADRs |
|----|-------|----------|-------|-----------|------|
| NIT-01 | [The nightly loop and the run-state machine](nightly/01-the-nightly-loop-and-run-state.md) | 4 | stateDiagram-v2, flowchart, sequenceDiagram | [concepts.md](../../docs/concepts.md) | ADR-DE-007 |
| NIT-02 | [Replay capture and verify](nightly/02-replay-capture-and-verify.md) | 2 | flowchart, sequenceDiagram | [ADR-DE-003-protected-path-guard.md](../../docs/adr/ADR-DE-003-protected-path-guard.md) | ADR-DE-003 |
| NIT-03 | [The automerge policy decision tree](nightly/03-automerge-policy-decision-tree.md) | 2 | flowchart, sequenceDiagram | [ADR-DE-006-automerge-is-an-option.md](../../docs/adr/ADR-DE-006-automerge-is-an-option.md) | ADR-DE-006 |

### gate

| ID | Topic | Diagrams | Kinds | Governing | ADRs |
|----|-------|----------|-------|-----------|------|
| GAT-01 | [Verdict classes and the decide flow](gate/01-verdict-classes-and-the-decide-flow.md) | 5 | flowchart, sequenceDiagram | [concepts.md](../../docs/concepts.md), [ADR-DE-003-protected-path-guard.md](../../docs/adr/ADR-DE-003-protected-path-guard.md) | ADR-DE-003, ADR-DE-009 |

### crates

| ID | Topic | Diagrams | Kinds | Governing | ADRs |
|----|-------|----------|-------|-----------|------|
| CRT-01 | [The crate dependency DAG and the licence boundary](crates/01-crate-dependency-dag-and-the-licence-boundary.md) | 2 | flowchart | [ADR-DE-002-licence-and-provenance.md](../../docs/adr/ADR-DE-002-licence-and-provenance.md), [ADR-DE-001-standalone-rust-engine.md](../../docs/adr/ADR-DE-001-standalone-rust-engine.md) | ADR-DE-001, ADR-DE-002 |
| CRT-02 | [Seams and what implements them](crates/02-seams-and-what-implements-them.md) | 2 | classDiagram, sequenceDiagram | [ADR-DE-005-seams.md](../../docs/adr/ADR-DE-005-seams.md) | ADR-DE-005 |

### selection

| ID | Topic | Diagrams | Kinds | Governing | ADRs |
|----|-------|----------|-------|-----------|------|
| SEL-01 | [Pairwise selection policies and the fault mapping](selection/01-pairwise-policies-and-the-fault-mapping.md) | 3 | flowchart | [ADR-DE-004-pairwise-selection-and-fingerprints.md](../../docs/adr/ADR-DE-004-pairwise-selection-and-fingerprints.md) | ADR-DE-004 |
| SEL-02 | [The failure-fingerprint lifecycle](selection/02-failure-fingerprint-lifecycle.md) | 4 | flowchart, sequenceDiagram | [ADR-DE-004-pairwise-selection-and-fingerprints.md](../../docs/adr/ADR-DE-004-pairwise-selection-and-fingerprints.md) | ADR-DE-004 |
<!-- END GENERATED DIAGRAM INDEX -->
