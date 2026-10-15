# dream-gate

A **pure** acceptance gate for model-written patches in an evidence-gated
self-improvement loop. No network, no subprocess, no async runtime. The judging
functions (`guard::check`, `gate::decide`, receipt classification, verdict
parsing, witnesses) are deterministic, so a verdict can be replayed and
audited. The only impurities are wall-clock timestamps in a few constructors
and the manifest/receipt file helpers (`freeze`, `load`, `persist`).

| Module | Role |
|---|---|
| `manifest` | Freeze what an experiment *is* (baseline, evaluators, model, config digest) before the model is called; restart-safe run identity. |
| `receipts` | Typed raw results of every evaluator run (passed / failed / blocked / timed out / silent / missing). |
| `guard` | Refuse patches that touch protected paths — fail closed, before the patch is applied. |
| `gate` | Combine strict verdict, receipts, guard and optional per-test evidence into one `GateDecision`. |
| `pertest` | Neutral per-test input for `pairwise-gate` and `failure-fingerprint` evidence. |
| `verdict`, `witness`, `patch`, `outcome`, `config` | Supporting vocabulary: strict verdict parsing, evidence-chain witnesses, pure diff repair, execution outcome, config schema. |

## Guard

`guard::check(patch, &policy)` parses a unified git diff and refuses it unless
every touched path is clear. It collects `diff --git` (both sides), `---`/`+++`,
`rename`/`copy` `from`/`to`; refuses symlinks, gitlinks, odd modes and binary
patches; resolves `.`/`..`; rejects absolute and escaping paths; handles CRLF
and git-quoted names; and treats **any** parse doubt as a refusal.

```rust
use dream_gate::guard::{check, ProtectedPolicy};

let policy = ProtectedPolicy::default().with_self_referential(true);
let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n";
assert!(check(patch, &policy).is_ok());
```

Call it **before applying** a patch. `gate::decide` re-derives the touched paths
from the patch again through its `GateContext` as defence in depth, and a refused patch is a `REJECT`
(evidence), never diluted into `BLOCKED-ENV`.

Known limit: an evaluator that reads files it never names on its command line
(a `Makefile` behind `make test`, a `conftest.py`) cannot be protected by
command-path extraction; list such files in the config's `protected` globs.

## Verdict mapping

| Cause | Class | Verdict |
|---|---|---|
| evaluator missing / silent / blocked / timed out, patch did not apply, config digest drift, per-test fault | harness | `BLOCKED-ENV` |
| non-zero exit, explicit FAIL, protected path touched, new failure fingerprint, pairwise-policy rejection | evidence | `REJECT` |
| no patch, unreadable verdict line | unproven | `INCONCLUSIVE` |

## Config vocabulary

`DreamConfig` validates what the reference TypeScript `validateConfig`
validates: `repo` is `owner/name` (`[\w.-]` parts), a present `cron` has five
fields and a single fixed minute (so the minimum interval is one hour), every
`bonusModuli` key is an integer, `adrConvention` is `"3-digit"`, `"4-digit"`,
`{ "pad": n >= 1, "dir": "..." }` or a legacy free-form template such as
`"ADR-XX-NNN"` (`AdrConvention`). `autoMerge` (alias `automerge`) defaults to off.
A config-digest mismatch where the patch did not touch the config is
`BLOCKED-ENV`. An absent `cron` is
accepted: this crate has no opinion about scheduling. An evaluator may declare
`"testFormat": "libtest" | "junit" | "vitest-json"` (`TestFormat`), frozen into
the manifest, so per-test output is read the same way on both trees. An evaluator without
one counts as a single case.

## Licence

MIT OR Apache-2.0.
