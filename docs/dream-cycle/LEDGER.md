| Date | Deep | Finding | Issue | PR | Evaluated? | Verdict | Effect | Witness | Prior-night fates |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-08-13 | security-adversarial | redblue evaluator entrypoint silently no-ops (npx bin-symlink isMain footgun); added classifyEntrypointResult + verify-entrypoint | #6 | #7 | yes | ACCEPT | npm test 85->96, 0 regressions | ec2052aa | first real night (demo seed rows removed 2026-08-13; see #6) |
| 2026-08-14 | ledger-signals | Loom timeout (degraded night — model benchmarking) | NONE | NONE | yes | INCONCLUSIVE |  | ec02320f | first HP annexe e2e test |
| 2026-08-14 | developer-experience | ledger signals zeroMergeStreak is an unverified worst-case default (never wired mergedPrNumbers); added --merged flag | #8 | #9 | yes | ACCEPT | npm test 96->101, 0 regressions | e55f6413 | PR #7 merged by human 2026-08-13 (ironic: the exact merge the buggy signal missed) |
| 2026-08-15 | golden-snapshots | Darwin fitness saturated: 9 mutants x 5 genotype surfaces all = baseline 0.985; evaluator sensitivity unproven | NONE | NONE | yes | INCONCLUSIVE | +0.000 | 46445823 | GLM-5.3 via Z.AI |
| 2026-08-15 | evaluation-adapters | Loom timeout (degraded night) | NONE | NONE | yes | INCONCLUSIVE |  | de2ce03a |  |
| 2026-08-15 | evaluation-adapters | INCONCLUSIVE — see report | NONE | NONE | yes | INCONCLUSIVE |  | 7b050758 |  |
| 2026-08-15 | evaluation-adapters | Given the committed bench corpus (96 tests) and the Darwin fitness harness, when | NONE | NONE | yes | REJECT |  | e6af8d3b |  |
| 2026-08-15 | compiler-parity | Given the `compile` package's 19 unit tests expose no golden-snapshot fixtures v | NONE | NONE | yes | REJECT |  | 4d4245aa1959 |  |
| 2026-08-15 | compiler-parity | INCONCLUSIVE — see report | NONE | NONE | yes | INCONCLUSIVE |  | 9c55670e3550 |  |
| 2026-08-15 | compiler-parity | Given the Darwin evaluator configured with `--sandbox mock --mutator ruvllm --ru | NONE | NONE | yes | ACCEPT |  | 4d86e582cc13 |  |
| 2026-08-15 | compiler-parity | self-hosted dream.config.json had zero test coverage in @dream-machine/compile; added golden-snapshot + validation test reading the real config | #10 | #11 | yes | ACCEPT | npm test 96->100, 0 regressions | cf2f0711 | PR #7 merged 2026-08-13; PR #9 (2026-08-14) still open/draft, human review pending |
| 2026-08-16 | ledger-signals | Given the persisted ledger rows for 2026-08-15 carry empty `prior-night fates` ( | NONE | NONE | yes | ACCEPT |  | da74cb43142a |  |
| 2026-08-16 | ledger-signals | Given the Darwin evaluator at commit `8c945e3` runs with `--sandbox mock --mutat | NONE | NONE | yes | ACCEPT |  | 4e40f4930fa3 |  |
