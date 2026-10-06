# ADR-0007: darwin's ruvllm mutator reaches the model through a loopback Loom shim

- **Status**: Accepted — corrects the diagnosis in ADR-0006 §1 and amends ADR-0006's rule (§3b)
- **Date**: 2026-10-06
- **Related**: ADR-0006 (score uniformity fails the evaluator), ADR-0005 (darwin entrypoint), ADR-0002 (entrypoint liveness)
- **Deciders**: operator, 2026-10-06 (shim); owner, 2026-10-06 (§3b: fail only on zero real edits)
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
- ADR-0006's uniformity check stays as the backstop for an inert run, in the
  amended form of §3b. Only `retryPolicy` and `contextBuilder` mutations can
  move a mock score, so a uniform board after real edits is an expected,
  honest outcome; the shim count separates it from "no mutation happened".
- Bumping the darwin pin must re-check both defects. If a later darwin sends
  `loom_options` itself or reads `RUVLLM_TIMEOUT_MS`, the shim is harmless but
  redundant.

## 3a. Measured after the fix (2026-10-06)

A real run of the configured command through the shim:

- `loom-shim: summary ok=20 scaffold=0 truncated=0 empty=0 upstream-error=0`.
  All 20 mutations were real model edits (35–68 s each, about 18 minutes
  total).
- The leaderboard was still uniform at 0.765, and the script exited 3. The
  only score-relevant edits were `.slice(0, 30)` → `.slice(0, 40)` (g2_v2,
  g3_v4) and `maxAttempts` 3 → 4 (g3_v1). Both fall between rungs of the mock
  ladder, which needs a context width of 50 for 0.875 and 70 for 0.985
  (checked with darwin's own `extractSurfaceParams` and `scoreVariant`). The
  mutator sees only stderr (`task mock-4 unsolved after 3 attempts`), never
  the stdout trace that says the agent was `blind`, so it has no signal to
  widen the window far enough.
- The same entrypoint with darwin's deterministic mutator gives a varied
  leaderboard (winner g2_v5 at 0.875, delta +0.110) and exits 0.

The uniformity that remains is therefore real. Mutations happen, but none of
them reaches a rung the mock sandbox rewards. The owner's decision on that is
§3b.

## 3b. Amended uniformity rule (owner decision, 2026-10-06)

ADR-0006's check exists to catch an **inert pipeline**. That is what hid the
scaffold defect for a month: no mutation was ever applied, and a uniform
leaderboard was the only visible symptom. A run in which the model really did
edit the surfaces, but none of the edits moved the mock score, is an honest
"no score-moving edit" result, not a failure. The rule is therefore:

| Mutator | Shim count (`ok`) | Uniform leaderboard | Varied leaderboard |
|---|---|---|---|
| ruvllm | > 0 | exit 0, receipt note `no improvement found: N real mutations, all scored S (= baseline)` | exit 0 |
| ruvllm | 0 | exit 3: `… — 0 real mutations: the mutator made no edit` | exit 0 |
| ruvllm | summary missing or unreadable | treated as 0, so exit 3, with a line saying the summary was missing | exit 0 |
| deterministic (no shim) | none | exit 3, unchanged ADR-0006 rule | exit 0 |

- A "real mutation" is a shim `ok` verdict: the model returned non-empty,
  non-scaffold, untruncated content. darwin's own validator may still reject
  one, and the shim cannot see that; `ok` therefore bounds real edits from
  above. It always separates the inert case, because an inert run has
  `ok=0` by construction.
- A missing summary fails because otherwise a broken shim could pass
  silently, which is the very failure mode the check exists to catch.
- The deterministic mutator runs without a shim, so there is nothing to
  count. It keeps the strict rule; its mutations are byte-reproducible and
  reach the mock ladder, so a uniform board from it does signal a fault.
- Wiring: the shim rewrites `{ok, scaffold, truncated, empty,
  upstream-error}` to a stats file after every call. The entrypoint passes
  it as `verify-entrypoint --shim-stats <path>`, and
  `checkDarwinBounds(rows, 0, { realMutations })` applies the table above
  (`packages/cli/src/darwinBounds.ts`, `packages/cli/src/index.ts`).
- Applied to the §3a proof run (`ok=20`, all 0.765), the rule gives exit 0
  with the note `no improvement found: 20 real mutations, all scored 0.765
  (= baseline)`.

## 3c. Upstream asks (@metaharness/darwin)

The shim works around three darwin@0.10.2 defects. Each, fixed upstream,
removes a reason for it to exist:

1. `RuvllmMutator` should forward a configurable `loom_options` (or arbitrary
   extra request fields), so an OpenAI-compatible façade's scaffold can be
   turned off.
2. `RuvllmMutator` should read `RUVLLM_TIMEOUT_MS` (or take `--ruvllm-timeout`);
   today `timeoutMs` defaults to a hard-coded 30 s that the CLI never
   overrides, so reasoning models are aborted and every mutation silently
   becomes a no-op.
3. The mutator's reflection context should include the task's stdout trace,
   not only stderr. The mock sandbox's `blind` (context too narrow) signal is
   on stdout, so the model never learns which change would move the score.

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
  refuses a missing `--ruvllm-url` and the `=` spellings. For §3b, it drives
  (a) `ok=0` + uniform → 3, (b) `ok>0` + uniform → 0 with the note,
  (c) no summary + uniform → 3, (d) varied → 0.
- `packages/cli/src/index.test.ts` (`--shim-stats`) and
  `packages/cli/src/darwinBounds.test.ts` (`realMutations`): the same table at
  the CLI and pure-function levels, plus an unreadable summary and the
  generation bounds still holding alongside a real-mutation pass.
