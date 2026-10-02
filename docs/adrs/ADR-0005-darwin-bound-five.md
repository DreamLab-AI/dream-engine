# ADR-0005: Darwin's per-generation bound is five, and the darwin entrypoint fails on a bound breach

- **Status**: Accepted — amends ADR-0003
- **Date**: 2026-10-02 (decision taken 2026-09-13)
- **Related**: ADR-0003 (darwin bound guard; this record amends its candidate bound and its "detection, not enforcement" consequence), ADR-0002 (entrypoint liveness classification), ADR-0001 §2.3
- **Deciders**: operator, 2026-09-13 ("lever E+P"); operator integration, 2026-10-02
- **Tags**: dream-cycle, evaluation-adapters, evaluator-trust, pinned-evaluator-entrypoints

## 1. Context

ADR-0003 made the nightly step-10 bound machine-checkable:
**≤ 3 generations × ≤ 4 candidates per generation × ≤ 1 promoted lineage**.
It also stated plainly that the check was detection only. `verify-entrypoint`
was an operator diagnostic, and the annexe gate never called it.

The nights that followed showed the four-candidate bound to be wrong for the
evaluator actually pinned (`@metaharness/darwin@0.10.2`, the
pinned-evaluator-entrypoints discipline in `dream.config.json`):

| Night | Ledger finding |
|-------|----------------|
| 2026-09-07 | darwin gen2 ran 5 candidates; evaluator still PASSED (ADR-0003's motivating run) |
| 2026-09-08 | g2=5 breach, second night; the guard was in-tree, but the gate stayed fail-open |
| 2026-09-12 | "g2=5 is structural: pinned darwin enumerates all 5 config surfaces" |
| 2026-09-13 | "Gate veto keys on darwin exit code only; g2=5 breach 4th obs, 0 ACCEPTs" |
| 2026-09-27, 2026-10-02 | the same finding re-raised; both nights VETOED as BLOCKED-ENV |

The receipts tag every g2 candidate with a distinct mutator surface (planner,
toolPolicy, reviewer, contextBuilder, scorePolicy). These are the five surfaces
of darwin@0.10.2's map, one candidate each, not a carried elite inflating the
count (ADR-0003 §1). The rule as written did not describe the tool it governed,
so every darwin night was a guaranteed "breach" that carried no information.

Two defects therefore compounded. The bound fired on every run, so it said
nothing. The check could not veto a night anyway, because the gate vetoes on
the darwin evaluator's exit code and the bound check never touched that code.
Five nights tried to ship a fix as a dream patch; none of the patches applied.

## 2. Decision

The operator decided on 2026-09-13, verbatim:

> Decision: lever E+P. P: amend ADR-0003's bound to ≤5 candidates/gen to match
> darwin@0.10.2's 5-surface map, recorded as ADR-0005 (0004 is taken). E: the
> darwin entrypoint becomes a checked-in script that runs darwin, then
> verify-entrypoint, and exits non-zero on breach. Ship both in one dream-patch.
> V (darwin bump) is deferred. No issue filed.

### 2.1 Lever P: the bound is five

`DARWIN_BOUNDS.maxCandidatesPerGeneration` (`packages/cli/src/darwinBounds.ts`)
becomes **5**. The prose bound compiled into the nightly prompt's step 10
(`packages/compile/src/index.ts`) moves with it. The generation bound (≤ 3) and
the promoted-lineage bound (≤ 1) are unchanged.

### 2.2 Lever E: the darwin entrypoint is a checked-in script

`scripts/darwin-entrypoint.sh` is now the darwin evaluator. Its argument is the
pinned darwin command line, so the full command stays in `dream.config.json`:

```
RUVLLM_TIMEOUT_MS=180000 ./scripts/darwin-entrypoint.sh npx @metaharness/darwin@0.10.2 evolve . --sandbox mock --mutator ruvllm ...
```

The script:

1. Refuses (exit 64) any command that does not carry an exact semver pin of
   `@metaharness/darwin`, carries two different pins, or lacks
   `--sandbox mock|agent`. Nothing runs in that case.
2. Executes the command through `dream-machine verify-entrypoint darwin
   --passthrough`, so the CLI runs darwin itself and parses the leaderboard
   once, from that run. `--passthrough`, a new flag, echoes darwin's stdout and
   stderr ahead of the verdict, so the evaluator receipt still carries the
   leaderboard.
3. Exits with the CLI's status: 0 live and in bounds, 1 darwin failed,
   2 suspicious-silent, 3 bound breached or leaderboard unparsable, 69 CLI not
   built.

The darwin command was kept in config, not moved inside the script, for two
reasons. First, the dream-engine annexe's admission
(`services/dream-engine/src/readiness.rs` and `config.rs` in agentbox)
recognises darwin by the package name in the command and refuses it without
`--sandbox mock|agent`. A bare `./scripts/darwin-entrypoint.sh` would have
switched that check off without anyone noticing. Second, the compiled-prompt
test from PR #18 requires every `@metaharness/darwin` mention to be one exact
pin, so the evaluator line still has to show it. The script path itself passes
admission's missing-script check because the annexe receives `git archive HEAD`
of a tree that contains it, with the executable bit set.

### 2.3 Lever V is deferred

Bumping darwin past 0.10.2 is out of scope. The pin and its discipline text are
unchanged. Because the bound is now tied to 0.10.2's surface map, any future
bump must re-derive the bound from the new version's behaviour in the same
change.

## 3. Consequences

- **The 2026-09-07 leaderboard is now compliant.** Five candidates in g2 are
  the ceiling, not a breach. The regression fixtures move up by one: six in a
  generation is the pinned breach (`g2 candidates=6 > 5`).
- **Detection becomes enforcement on the darwin path.** ADR-0003 §2 said that
  nothing in it could veto a night. That no longer holds for the darwin
  evaluator as the annexe runs it. A breach now exits 3, the annexe records the
  REQUIRED evaluator as `FAILED`, and the gate's existing exit-code veto, the
  very mechanism the 2026-09-13 night identified, does the rest. No annexe
  change was needed. `verify-entrypoint` run by hand is still only a diagnostic.
- **Fail-closed on leaderboard drift.** An unparsable leaderboard already
  reported `ok: false` (ADR-0003 §2.1). On this path it now fails the night. If
  a future darwin changes its output format, nights fail visibly instead of
  passing silently. That is the intended trade, and it is one more reason a
  darwin bump (V) must be deliberate.
- **The promoted-lineage bound is still unchecked** on this path (ADR-0003 §3).
  Two of the three bounds are enforced, not three.
- **The script needs a built CLI.** It runs `packages/cli/dist/bin.js`, which
  the annexe's `buildStep` (`npm ci && npm run build`) produces before
  evaluators run. A missing build exits 69, not 0.
- `verify-entrypoint` gains `--passthrough`. Its existing exit codes and output
  are unchanged when the flag is absent.

## 4. Alternatives Considered

- **Keep ≤ 4 and treat g2=5 as a darwin bug.** Rejected: the behaviour is
  structural to the pinned version (one candidate per surface). A bound that
  fails every run carries no signal, and the nights that hit it could not ACCEPT.
- **Bump darwin (lever V) to a version with configurable fan-out.** Deferred by
  the operator. It is a supply-chain change with its own evidence burden, and
  the pin discipline requires it to be deliberate and recorded.
- **Run darwin in the script, then pipe its captured output into
  `verify-entrypoint --cmd "cat <file>"`.** Rejected: the CLI would then
  classify `cat`'s exit status instead of darwin's, so a failed darwin run with
  partial output could read as live. Having the CLI run darwin keeps one parse,
  of one run, with darwin's own exit status.
- **Hard-code the darwin command inside the script, with config calling only
  the script.** Rejected: it bypasses the annexe's darwin sandbox admission and
  hides the pin from the compiled-prompt pin test (§2.2).
- **Enforce in the annexe gate.** Not needed. The gate already vetoes on the
  required evaluator's exit code, and lever E makes that code truthful.

## 5. Test Contract

1. `DARWIN_BOUNDS.maxCandidatesPerGeneration === 5`. The other two bounds are
   unchanged.
2. The pinned 2026-09-07 leaderboard (`{1: 4, 2: 5}`) reports `ok: true`. The
   same leaderboard with a sixth g2 row reports exactly `g2 candidates=6 > 5`.
3. `verify-entrypoint darwin` exits 0 on the five-candidate leaderboard and 3 on
   the six-candidate one. With `--passthrough`, the command's stdout precedes
   the verdict and its stderr is echoed. Without the flag, neither is echoed.
4. `scripts/darwin-entrypoint.sh` is executable. Run against a stub darwin
   (`packages/cli/test-fixtures/fake-darwin.mjs`) and the captured leaderboard
   fixtures, it exits 0 / 3 / 1 / 2 for in-bounds, breach, darwin failure and
   silent success. It passes arguments verbatim and the environment through,
   and it refuses (64) with nothing run for: no command, an unpinned pin, a
   ranged pin, conflicting pins, a non-darwin command, and a missing or
   `real` sandbox.
5. `dream.config.json`'s darwin evaluator runs through the script, and its
   command line keeps `@metaharness/darwin@0.10.2 evolve .` and
   `--sandbox mock` visible. The compiled-prompt test from PR #18 (one exact
   darwin pin) stays green.

## 6. References

- `packages/cli/src/darwinBounds.ts`, `packages/cli/src/darwinBounds.test.ts`
- `packages/cli/src/index.ts` (`verify-entrypoint`, `--passthrough`),
  `packages/cli/src/index.test.ts`
- `scripts/darwin-entrypoint.sh`, `packages/cli/src/darwinEntrypoint.test.ts`,
  `packages/cli/test-fixtures/`
- `packages/compile/src/index.ts` (step-10 prose bound)
- `docs/dream-cycle/LEDGER.md` rows 2026-09-07 to 2026-10-02
- agentbox `services/dream-engine/src/readiness.rs` (`assess`, `script_paths`,
  `DarwinSandboxMissing`) and `runner.rs` (`bash -o pipefail`, evaluator exit
  status)
