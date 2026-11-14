# Diagrams as code — moved to the VisionFlow estate master record

The hand-authored, citation-verified corpus that lived here (8 topics in four areas, the
register and the decisions timeline) moved on 2026-11-05 into the estate's single diagram
corpus, where it joins the estate's own dream-engine topics (DE-01 to DE-06) in the
`dream-engine` area:

- tree: <https://github.com/DreamLab-AI/VisionFlow/tree/main/docs/diagrams/dream-engine>
  (local checkout: `../VisionFlow/docs/diagrams/dream-engine`)
- gate: `node scripts/diagram-index-gen.cjs docs/diagrams --check --strict-citations`, run from
  the VisionFlow root. `sources:` there reach this repository as `../dream-engine/…`, and every
  topic keeps the `verified_commit` it was stamped at here.
- the narratives and register paragraphs moved intact, and the master record cite-checks
  their prose as it does the diagrams.

`review-ledger.jsonl` stays here as the history of the nightly diagram reviews of this
corpus (agentbox ADR-2131); new reviews of these topics land in the master record's ledger.
The vendored generator and Pele build went with the corpus. History up to the move is in
this repository's git log.

## Id map

Topic ids were renumbered into the master record's one-prefix-per-area scheme. Diagram ids
follow their topic (`GAT-01.2` is now `DE-10.2`). Review files under `docs/review/` keep the
ids they were written with.

| Old | New | Was | Now (VisionFlow) |
|---|---|---|---|
| NIT-01 | DE-07 | `docs/diagrams/nightly/01-the-nightly-loop-and-run-state.md` | `docs/diagrams/dream-engine/07-the-nightly-loop-and-run-state.md` |
| NIT-02 | DE-08 | `docs/diagrams/nightly/02-replay-capture-and-verify.md` | `docs/diagrams/dream-engine/08-replay-capture-and-verify.md` |
| NIT-03 | DE-09 | `docs/diagrams/nightly/03-automerge-policy-decision-tree.md` | `docs/diagrams/dream-engine/09-automerge-policy-decision-tree.md` |
| GAT-01 | DE-10 | `docs/diagrams/gate/01-verdict-classes-and-the-decide-flow.md` | `docs/diagrams/dream-engine/10-verdict-classes-and-the-decide-flow.md` |
| CRT-01 | DE-11 | `docs/diagrams/crates/01-crate-dependency-dag-and-the-licence-boundary.md` | `docs/diagrams/dream-engine/11-crate-dependency-dag-and-the-licence-boundary.md` |
| CRT-02 | DE-12 | `docs/diagrams/crates/02-seams-and-what-implements-them.md` | `docs/diagrams/dream-engine/12-seams-and-what-implements-them.md` |
| SEL-01 | DE-13 | `docs/diagrams/selection/01-pairwise-policies-and-the-fault-mapping.md` | `docs/diagrams/dream-engine/13-pairwise-policies-and-the-fault-mapping.md` |
| SEL-02 | DE-14 | `docs/diagrams/selection/02-failure-fingerprint-lifecycle.md` | `docs/diagrams/dream-engine/14-failure-fingerprint-lifecycle.md` |
