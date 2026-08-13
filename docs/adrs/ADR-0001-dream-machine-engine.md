# ADR-0001: The Dream Machine engine — a config-driven, evidence-gated nightly evolution loop composed from the ruvnet stack

- **Status**: Proposed — founding design; no engine code shipped yet. Two production routines (`ruvnet/ruflo`, `ruvnet/metaharness`) are the working reference implementations this engine will subsume.
- **Date**: 2026-08-13
- **Deciders**: ruv
- **Tags**: dream-machine, founding, config-compiler, flywheel, darwin, redblue, promotion-gate, witness, autonomy
- **Composes**: `@metaharness/flywheel` 0.1.10, `@metaharness/darwin` 0.9.1, `@metaharness/redblue` 0.1.4, `metaharness` CLI 0.4.5, ruvector 0.2.41, agentdb 3.0.0-alpha (all optional/peer — see §4)
- **Prior instances**: `ruvnet/metaharness` ADR-251 (MetaHarness Nightly Dream Cycle); the Ruflo Nightly Dream Cycle v3 (gist `ruvnet/889ffa92dab49d508e70b123c940e1b9`)

---

## 1. Context

Two nightly routines run today in Anthropic's cloud scheduler, each a fully
isolated cloud session with a fresh checkout, driven by one ~800-line prompt:

- **Ruflo Nightly Dream Cycle** — `ruvnet/ruflo`, cron `0 6 * * *` UTC.
- **MetaHarness Nightly Dream Cycle** — `ruvnet/metaharness`, cron `0 8 * * *` UTC (ADR-251 in that repo).

Both run the identical 26-step pipeline:

```
ledger → research → hypothesis → candidate → baseline → evaluation
  → adversarial critique → bounded Darwin evolution → flywheel evidence
  → witness → issue → draft PR → durable ledger row
```

A full read of both prompts, the tutorial, and metaharness ADR-251 shows they
are **near-identical** except for a small, well-defined set of per-repo
details. Today those two prompts are hand-maintained twins, and each is itself
duplicated between the scheduler and an in-repo mirror. ADR-251 §2.4 names the
resulting hazard explicitly: the mirror "can drift; the header rule makes
drift a reviewable defect but cannot mechanically prevent it."

Three forces make this the right moment to extract an engine:

1. **Two instances is enough to see the seam.** With one routine the shared
   spine is invisible; with two, the generic-vs-config split is empirical, not
   speculative (see §3).
2. **The evaluation backends now exist as published packages.**
   `@metaharness/flywheel` (promotion gate/receipts/replay),
   `@metaharness/darwin` (bounded evolution), and `@metaharness/redblue`
   (adversarial red/blue) are shipped npm packages, so the pipeline's heaviest
   stages can be *composed* rather than reimplemented per routine.
3. **The duplication is already a maintenance defect**, not a hypothetical
   one — four hand-synced copies (two repos × scheduler+mirror) of an
   800-line prompt.

## 2. Decision

Build **the Dream Machine**: a config-driven engine that compiles a
`dream.config` for a target repository into the routine prompt, backed by a
small toolkit for the pipeline's durable-state and provenance mechanics, and
delegating the evaluation-heavy stages to the ruvnet packages that already
implement them.

### 2.1 The compiler is the core

`@dream-machine/compile` takes a `dream.config` and emits the full routine
prompt. Both the scheduler copy and the in-repo mirror become build
**outputs** of the same source — mechanically eliminating the drift ADR-251
could only flag by convention.

A `dream.config` supplies exactly the per-repo delta set identified in §3:

```jsonc
{
  "repo": "ruvnet/metaharness",
  "cron": "0 8 * * *",                    // staggered per repo
  "slots": [                              // rotation vocabulary, per repo
    { "deep": "generator-genome",   "scan": ["router", "turn-credit"] },
    { "deep": "flywheel-promotion", "scan": ["evals-verticals", "bench"] }
    // ...
  ],
  "bonusModuli": { "25": "vertical-packs", "75": "meta-proxy" },
  "controlPlaneProbes": ["node packages/create-agent-harness/dist/bin.js --help"],
  "buildStep": { "cmd": "npm ci && npm run build", "degradeOnWasmFailure": true },
  "evaluatorEntrypoints": { "bench": "experiments/*/run.mjs", "flywheel": "..." },
  "adrTemplate": "metaharness-7-section",  // vs ruflo's 14-section
  "competitorList": ["LangGraph", "AutoGen", "DSPy/GEPA", "SWE-bench derivatives"],
  "extraDiscipline": ["adr-250-proof-ladder"]
}
```

### 2.2 The generic spine ships as a toolkit

The repo-agnostic mechanics become tested library code instead of behavior the
LLM re-derives every night:

- **`@dream-machine/ledger`** — parse / append / verify the 10-column
  `LEDGER.md` schema, and compute the STEP 1.1 learning signals
  (duplicate-finding suppression, zero-merge-in-14-nights bias, low-score-streak
  throttling) deterministically.
- **`@dream-machine/witness`** — `stamp` and `verify` the
  `WITNESS = sha256(sha256(gist) + SESSION_COMMIT)` scheme as a reusable CLI,
  matching the 5-step verifier both routines already publish.
- **`@dream-machine/schedule`** — push a compiled prompt to the cloud
  `/schedule` routine and keep the mirror in lockstep. **v1 ships as
  "compile → paste into `/schedule`"**; automation waits on a confirmed
  Anthropic-side scheduling API (not verified as a callable API as of this ADR).

### 2.3 Evaluation is delegated, never reimplemented

The pipeline's expensive stages wire through `evaluatorEntrypoints` to the
packages that own them:

| Stage | Backend | Package |
|---|---|---|
| STEP 10 adversarial critic, STEP 11 reward-hack scan | red/blue harness | `@metaharness/redblue` |
| STEP 12 bounded Darwin evolution | mutation + scoring engine | `@metaharness/darwin` |
| STEP 13 evidence retention, STEP 14 promotion gate | flywheel gate/receipts/replay | `@metaharness/flywheel` |
| STEP 15 security review | scan surfaces | `metaharness` CLI (`mcp-scan`, `threat-model`, `secrets`) |

This follows both source prompts' own principle: "use the best available
implementation; do not introduce dependencies merely to satisfy this prompt."

### 2.4 Non-negotiable invariants (carried verbatim from both routines)

- Every run ends `ACCEPT | REJECT | INCONCLUSIVE`; `INCONCLUSIVE`-with-reason
  (including missing-credentials) is a success, not a failure.
- **Evaluation is not promotion.** The session never merges, never
  self-promotes flywheel state, never weakens a test/benchmark/threshold,
  never edits gold answers, never force-pushes, never publishes packages.
- The ledger row is written on every run, even a budget-forced halt
  (`HALT: budget`) — it is the only durable cross-night memory.
- All output lands as gists, issues, and always-**draft** PRs. Promotion is a
  human act.

## 3. Consequences

- **Drift becomes structurally impossible** for the parts the compiler owns:
  the scheduler prompt and the in-repo mirror are the same build output. This
  is the concrete resolution of ADR-251's stated unsolved problem.
- **Onboarding a third repo is writing a `dream.config`**, not forking and
  hand-editing an 800-line prompt. The rotation vocabulary, evaluator
  entrypoints, and ADR shape are declared, not prose.
- **The evaluation backends stay optional.** A target with none of
  flywheel/darwin/redblue built or credentialed runs a legitimate
  no-model-call night (`LLM_EVAL=blocked`), exactly as ADR-251 already
  specifies for metaharness. The engine must never hard-depend on them.
- **New surface to maintain**: a compiler, three toolkits, and adapter shims.
  The bet is that a tested engine is cheaper to maintain than four hand-synced
  800-line prompts, and gets more correct with each repo added rather than
  more fragile.
- **The `/schedule` automation is deliberately deferred** to a paste-in step
  until a callable scheduling API is confirmed — shipping an unverified API
  wrapper would be exactly the overclaim the pipeline's own discipline forbids.
- **This repo is itself a candidate target.** Once the engine runs, a
  `dream.config` for `ruvnet/dream-machine` would let the Dream Machine dream
  about itself — deferred until the engine exists, and noted here as the
  natural closing of the loop, not a v1 commitment.

## 4. Alternatives Considered

- **Leave the two prompts hand-maintained.** Rejected: four hand-synced copies
  of an 800-line prompt is already a maintenance defect, and ADR-251 admits it
  cannot mechanically prevent mirror drift.
- **A shared prompt *include* / partial system** (templated fragments, no typed
  config). Rejected: it factors the text but not the *decisions* — the
  per-repo deltas (rotation slots, evaluator entrypoints, ADR shape) want a
  validated schema, not string interpolation.
- **Reimplement flywheel/darwin/redblue inside the engine** for a
  zero-dependency build. Rejected: it duplicates thousands of tested lines,
  contradicts the "compose, don't reimplement" principle, and the packages are
  already the maintained homes of that logic. Optional/peer wiring keeps
  degraded nights working without them.
- **A GitHub Actions cron workflow per repo instead of a cloud routine.**
  Rejected (same reasoning as ADR-251): the loop needs an agentic session with
  parallel research and judgment, not a CI step, and keeping evolution
  authority out of the repo's own CI credentials is a security feature.
- **Fold the engine into `ruvnet/metaharness`.** Rejected: the Dream Machine is
  repo-agnostic by construction and drives *other* repos (Ruflo included);
  binding it to one target's release cadence and ADR series would re-entangle
  exactly what this extraction separates. A standalone repo keeps the
  one-engine-many-targets boundary legible.

## 5. Test Contract

This ADR is satisfied when:

1. **Compiler parity**: `@dream-machine/compile` compiles the
   `ruvnet/metaharness` `dream.config` to a prompt byte-identical to the
   hand-written `docs/dream-cycle/PROMPT.md` shipped in metaharness ADR-251
   (the existence proof that the config captures the full delta), and likewise
   for the Ruflo prompt.
2. **Ledger toolkit**: `@dream-machine/ledger` round-trips the 10-column schema
   (parse → append a row → re-parse) and reproduces the STEP 1.1 learning
   signals on a fixture ledger, with property tests for the append invariant
   (exactly one row added, schema unchanged).
3. **Witness toolkit**: `@dream-machine/witness verify` accepts a genuine
   `stamp` output and rejects any single-byte mutation of the gist or the
   session commit — the 5-step scheme, executable.
4. **Optional-backend degradation**: the compiled prompt and toolkits run a
   full pipeline to a verdict with `@metaharness/{flywheel,darwin,redblue}`
   absent, producing an `INCONCLUSIVE`/`LLM_EVAL=blocked` night with a ledger
   row — proving the backends are optional, not required.
5. **No-merge invariant**: no engine code path merges, self-promotes, or writes
   to a protected branch; the only publication surfaces are gist, issue, and
   draft PR.

## 6. References

- `ruvnet/metaharness` ADR-251 — the MetaHarness Nightly Dream Cycle (the
  direct predecessor and first config target); `docs/dream-cycle/PROMPT.md`
  there is the reference prompt.
- Ruflo Dream Cycle v3 — gist `ruvnet/889ffa92dab49d508e70b123c940e1b9`
  (`dream-machine-tutorial.md`, `x-claude-prompt.md`).
- `@metaharness/flywheel`, `@metaharness/darwin`, `@metaharness/redblue`,
  `metaharness` CLI — the composed evaluation backends.
- **Negative precedent**: Sakana AI, "The AI Scientist" — the reward-hacking
  incident (an autonomous loop rewriting its own timeout) that motivates the
  reward-hack check (STEP 11) and the hard authority boundaries.
- **Positive precedent**: DSPy / GEPA-style prompt-evolution frameworks (the
  Darwin stage's conceptual neighbors); classical champion/challenger +
  shadow-deployment MLOps (the promotion-gate design applied to LLM-harness
  evolution).
