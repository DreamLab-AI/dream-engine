| Date | Deep | Finding | Issue | PR | Evaluated? | Verdict | Effect | Witness | Prior-night fates | Reviewer | Review-minutes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-17 | evaluator-adapters | Environment fault, hypothesis untested: fmt: required evaluator exited 0 with no | NONE | NONE | blocked | BLOCKED-ENV |  | BLOCKED |  |  |  |
| 2026-10-18 | source-and-context | Baseline gate broken: cargo-deny absent, doc+fmt die on harness quoting | NONE | NONE | yes | INCONCLUSIVE |  | d95d2a32c43d |  |  |  |
| 2026-10-19 | developer-experience | Gate still broken: cargo-deny absent; doc/fmt die on harness quoting (night 2) | NONE | NONE | yes | INCONCLUSIVE |  | 16710160fac4 |  |  |  |
| 2026-10-20 | ledger-signals | Added passing test witnessing record_night starts afresh on a corrupt manifest | NONE | https://github.com/DreamLab-AI/dream-engine/pull/8 | yes | ACCEPT |  | cb8acb6a96c6 |  |  |  |
| 2026-10-21 | evaluator-adapters | VETOED: Witness test: tail-truncated libtest candidate parses under bare names | NONE | VETOED | yes | REJECT |  | 66ab75240ea3 |  |  |  |
| 2026-10-22 | source-and-context | VETOED: Witnessed clip_middle keeps head/tail verbatim and reports exact elided… | NONE | VETOED | yes | REJECT |  | 2996ef1af620 |  |  |  |
