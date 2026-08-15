| Date | Deep | Finding | Issue | PR | Evaluated? | Verdict | Effect | Witness | Prior-night fates |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-08-13 | security-adversarial | redblue evaluator entrypoint silently no-ops (npx bin-symlink isMain footgun); added classifyEntrypointResult + verify-entrypoint | #6 | #7 | yes | ACCEPT | npm test 85->96, 0 regressions | ec2052aa | first real night (demo seed rows removed 2026-08-13; see #6) |
| 2026-08-14 | ledger-signals | Loom timeout (degraded night — model benchmarking) | NONE | NONE | yes | INCONCLUSIVE |  | ec02320f | first HP annexe e2e test |
| 2026-08-15 | golden-snapshots | Darwin fitness saturated: 9 mutants x 5 genotype surfaces all = baseline 0.985; evaluator sensitivity unproven | NONE | NONE | yes | INCONCLUSIVE | +0.000 | 46445823 | GLM-5.3 via Z.AI |
| 2026-08-15 | evaluation-adapters | Loom timeout (degraded night) | NONE | NONE | yes | INCONCLUSIVE |  | de2ce03a |  |
| 2026-08-15 | evaluation-adapters | INCONCLUSIVE — see report | NONE | NONE | yes | INCONCLUSIVE |  | 7b050758 |  |
| 2026-08-15 | evaluation-adapters | Given the committed bench corpus (96 tests) and the Darwin fitness harness, when | NONE | NONE | yes | REJECT |  | e6af8d3b |  |
| 2026-08-15 | compiler-parity | Given the `compile` package's 19 unit tests expose no golden-snapshot fixtures v | NONE | NONE | yes | REJECT |  | 4d4245aa1959 |  |
