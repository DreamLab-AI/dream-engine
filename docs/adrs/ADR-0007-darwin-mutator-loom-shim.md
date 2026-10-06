# ADR-0007: darwin's ruvllm mutator reaches the model through a loopback Loom shim

- **Status**: Accepted — corrects the diagnosis in ADR-0006 §1; ADR-0006's check stands
- **Date**: 2026-10-06
- **Related**: ADR-0006 (score uniformity fails the evaluator), ADR-0005 (darwin entrypoint), ADR-0002 (entrypoint liveness)
- **Deciders**: operator, 2026-10-06
- **Tags**: dream-cycle, evaluation-adapters, evaluator-trust

## 1. Context

ADR-0006 read the uniform 0.765 leaderboards as a scorer returning a default.
That diagnosis was wrong. Under `--sandbox mock`, darwin@0.10.2 scores each
variant deterministically from its surface files (`dist/mock-sandbox.js`: the
`maxAttempts` budget in `retry_policy.ts` and the `.slice(0, N)` width in
`context_builder.ts` decide how many of five scripted tasks pass). No model
scores anything. Every mutant matched the baseline because every mutant *was*
the baseline: no mutation was ever applied.

Reproduced 2026-10-06 by putting a logging proxy in front of the Loom door and
running the configured command:

- darwin's `RuvllmMutator` posts a bare OpenAI chat request with no
  `loom_options`. The Loom handles it as an ontology lookup and answers in
  about 15 ms with `"_Served verbatim from the Ontology Loom (generation:
  visionGraph@…); no model generation was performed._"` followed by a concept
  page. 20 of 20 calls got this answer.
- darwin's validator rejects the page (`mutation rejected by validator; surface
  unchanged (secret handling)`). Every variant stays byte-identical to the
  baseline, and the mock sandbox scores each one 0.765 (3 of 5 tasks pass).
  The whole run takes about 1.5 s, which is why the nightly receipts show
  `duration=1541ms`.
- The same request with `loom_options.scaffold=false` returns a real
  replacement file, but only after about 60 s (a reasoning model generating
  about 1,100 tokens). darwin aborts its fetch after a hard-coded 30 s
  (`timeoutMs ?? 30_000`; nothing reads `RUVLLM_TIMEOUT_MS`, so the
  `RUVLLM_TIMEOUT_MS=180000` in `dream.config.json` had no effect), and an
  abort is another no-op.

darwin is a pinned third-party package (ADR-0005), so neither defect can be
fixed at its source here.

## 2. Decision

When the darwin command uses `--mutator ruvllm`, `scripts/darwin-entrypoint.sh`
starts a loopback shim (`packages/cli/src/loomShim.ts`, run as
`dist/loomShimBin.js`) and points darwin's `--ruvllm-url` at it. The shim:

- forwards each request to the configured Loom door with
  `loom_options.scaffold=false` and `max_tokens` raised to at least 8192
  (`LOOM_SHIM_MIN_MAX_TOKENS`), so reasoning cannot push the file past the cap;
- sends response headers at once. darwin's 30 s abort only covers the wait for
  headers, because it clears the timer once `fetch` resolves. The shim then
  enforces `RUVLLM_TIMEOUT_MS` (capped at 280 s, below undici's 300 s body
  timeout) on the upstream call itself;
- never invents a mutation: a verbatim ontology page, a completion with
  `finish_reason: "length"`, an empty answer, an upstream error or a timeout
  all become `{"choices":[]}`, which darwin records as a no-op;
- logs one line per call and a per-verdict summary
  (`loom-shim: summary ok=… scaffold=… truncated=… empty=… upstream-error=…`)
  to stderr, so the evaluator receipt shows how many mutations were real.

The script also refuses (exit 64):

- `--mutator ruvllm` with no `--ruvllm-url`. darwin's `localhost:8080` default is
  never the Loom.
- The `--flag=value` spellings of `--sandbox`, `--mutator`, `--ruvllm-url` and
  `--ruvllm-model`. darwin reads only `--flag value`, so `--sandbox=mock`
  previously passed the script's check while darwin ran the default real
  sandbox.

## 3. Consequences

- darwin nights make real model calls: about 60 s each, 20 per run at the
  default 3×4 shape, inside the annexe's 1800 s evaluator budget.
- ADR-0006's uniformity check stays as the backstop. A night can still fail it
  honestly, because only `retryPolicy` and `contextBuilder` mutations can move
  a mock score. The receipt's shim summary now separates "mutations happened
  but changed nothing the mock sandbox measures" from "no mutation happened".
- Bumping the darwin pin must re-check both defects. If a later darwin sends
  `loom_options` itself or reads `RUVLLM_TIMEOUT_MS`, the shim is harmless but
  redundant.

## 4. Alternatives considered

- **Point darwin at the model port (:8085) directly.** Rejected: estate policy
  is that consumers hold the Loom door, never the model behind it.
- **Disable reasoning so calls fit in 30 s.** Rejected: it depends on a
  model-specific template flag, still races a fixed timeout on larger surface
  files, and trades away mutation quality.
- **Drive darwin's library API from a wrapper script.** Rejected: it hides the
  pinned `npx @metaharness/darwin@X evolve` command from the dream-engine
  admission check (ADR-0005).

## 5. Test contract

- `packages/cli/src/loomShim.test.ts`: request rewrite, response
  classification, headers sent before a slow upstream answers, timeout, and
  scaffold handling against a fake Loom.
- `packages/cli/src/darwinEntrypoint.test.ts`: the real script routes a
  darwin-shaped mutator call through the shim (scaffold off, an 800 ms answer
  outliving a 300 ms header abort), leaves non-ruvllm runs untouched, and
  refuses a missing `--ruvllm-url` and the `=` spellings.
