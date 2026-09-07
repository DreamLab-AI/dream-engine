# ADR-0003: Darwin generation/candidate bounds are machine-checked as a fail-visible diagnostic, not a gate veto

- **Status**: Accepted
- **Date**: 2026-09-07
- **Related**: ADR-0001 §2.3 ("Evaluation is delegated, never reimplemented"), ADR-0002 (evaluator entrypoints classified live/blocked/suspicious-silent before EVALUATED is recorded)
- **Deciders**: dream-cycle nightly session (evaluation-adapters, SCAN=flywheel/darwin), 2026-09-07; operator integration, 2026-09-07
- **Tags**: dream-cycle, evaluation-adapters, evaluator-trust, evaluation-is-not-promotion, witness-every-quantitative-claim

## 1. Context

The compiled nightly prompt bounds every Darwin run at step 10:
**≤ 3 generations × ≤ 4 candidates per generation × ≤ 1 promoted lineage,
frozen fitness function** (`packages/compile/src/index.ts:269`). That bound
exists only as prose inside the prompt. Nothing on the evaluator path reads it.

On 2026-09-07 (run `ab4ced4e48b76e83`, commit `9f4d8a9a`) the REQUIRED darwin
evaluator returned `outcome=PASSED exit=0` while its own leaderboard listed
**five** candidates in generation 2 (`g2_v0..g2_v4`). The gate recorded
`accepted: true, vetoes: []`. The breach was independently re-confirmed from
the 2026-09-02 and 2026-09-05 receipts, where all five g2 rows carry distinct
mutator tags (reviewer / planner / contextBuilder / toolPolicy / scorePolicy),
so none is a carried elite inflating the count.

This is the same class of defect ADR-0002 exists to prevent — an evaluator
reporting success while the thing it was supposed to establish did not hold —
one level up: not a silent *no-op*, but a silent *out-of-policy run*. ADR-0002
made liveness machine-checkable; the run's declared bounds stayed unchecked.

A second observation from the same nights frames the value honestly: under
`--sandbox mock` the Darwin reward channel carries zero ranking signal (all
mutants score identically to the parent, Δ +0.000, winner = baseline, every
night). A PASSED darwin currently proves the harness ran, nothing more — and,
as 09-07 showed, it passes even while breaching its own bounds.

## 2. Decision

Add a pure, dependency-free bound checker at
`packages/cli/src/darwinBounds.ts`: `DARWIN_BOUNDS` policy constants,
`parseLeaderboardRows`, `checkDarwinBounds`, and a
`checkDarwinBoundsFromStdout` parsing wrapper.

Three properties are load-bearing:

1. **Fail-visible by construction.** Zero parsed rows yields
   `parseStatus: 'unparsable'` **and** `ok: false`, inside `checkDarwinBounds`
   itself rather than in the wrapper. The two fields can never disagree, so a
   caller reaching for the exported pure function cannot get a silent pass on
   input the parser did not understand. Guarding against leaderboard format
   drift is the whole point; a checker that returns `ok` on text it failed to
   read reproduces the defect it was written to catch.
2. **The generation bound constrains depth, not cardinality.** A run emitting
   g1, g3, g5 evolved five generations deep while showing only three distinct
   labels. The check uses the maximum generation index; the distinct-label
   count is retained as `generationsObserved` for reporting only.
3. **Baseline rows are never counted as generation candidates**, so a carried
   champion cannot inflate a generation.

The checker is wired into the `verify-entrypoint` CLI command
(`packages/cli/src/index.ts`), immediately after `classifyEntrypointResult`:
when the label is `darwin` and the verdict is `live`, the leaderboard is
checked and violations are printed on the error channel, with a new exit
code **3** joining the existing 0/1/2 vocabulary.

### This is detection, not enforcement

**Stated plainly: nothing in this ADR can veto a night.** The nightly gate —
the code that writes `gate.json`, honours `requiredOutcomes`, and records
`vetoes` — is **not in this repository**. It lives in the external dream-engine
annexe runner (agentbox `services/dream-engine`). This repo's packages compile
the nightly *prompt*; they do not execute the gate.

`verify-entrypoint` is an operator diagnostic. The annexe does not call it to
decide ACCEPT. Wiring the guard there makes a bound breach **loudly visible to
an operator who runs the command**, and it can neither fail a night nor block a
merge nor alter an evaluator score. A true auto-veto — making a bound breach
flip `accepted: false` — requires a change in the annexe runner and must be
filed separately against that component, citing
`packages/compile/src/index.ts:269` as the spec. Anyone reading a green
`verify-entrypoint` as evidence that bounds were *enforced* has misread it.

## 3. Consequences

- Silent bound breaches become machine-detectable, and the real 2026-09-07
  leaderboard is pinned as a regression fixture, so a future parser change that
  stops seeing the breach fails a test.
- Policy lives in one `DARWIN_BOUNDS` constant, adjustable in one line, instead
  of in prose inside a compiled prompt.
- Fail-visible parsing means leaderboard format drift surfaces as a violation
  rather than as a false pass.
- The `verify-entrypoint` exit vocabulary grows a fourth value (3). Existing
  0/1/2 semantics are unchanged; any caller treating "non-zero" as failure is
  unaffected.
- **The gap this does not close**: the nightly gate still cannot veto a bound
  breach. Detection without enforcement is an improvement over neither, but it
  is not the guard-rail, and the ledger should not be read as though it were.
- The promoted-lineage bound is **unchecked on the `verify-entrypoint` path**:
  the leaderboard exposes `Winner:` and `Lineage:` lines but not a promotion
  count, so the call site passes `0` explicitly and says so in a comment.
  Two of the three bounds are checked there, not three.

## 4. Alternatives Considered

- **Wire the checker straight into the gate as an auto-veto.** Rejected as
  impossible from this repository: the gate is in the annexe runner, outside
  this checkout. Merging any PR here cannot produce a veto, and claiming
  otherwise would be the same overstatement ADR-0002 was written against.
- **Parse bounds out of the compiled prompt at runtime** instead of duplicating
  them as constants. Rejected: it couples the checker to prompt prose formatting
  — a far more fragile contract than three integers — and the prompt is an
  input to the night, not a machine-readable policy file.
- **Fail closed on an unparsable leaderboard by throwing.** Rejected: a
  diagnostic that crashes is less useful than one that reports
  `parseStatus: 'unparsable'` with the rest of the report intact. `ok: false`
  already denies the silent pass, which is the property that matters.
- **Count distinct generation labels** (the original patch's behaviour).
  Rejected: it passes a sparse g1/g3/g5 run that is five generations deep. See
  §2.2.
- **Do nothing until the mock sandbox is discriminative.** Rejected: the two
  defects are independent. A zero-gradient reward channel makes a PASS weak
  evidence; an unchecked bound makes it *misleading* evidence. Fixing the second
  does not wait on the first.

## 5. Test Contract

1. `parseLeaderboardRows` parses the pinned real 2026-09-07 leaderboard as
   1 baseline + 9 mutants, and returns `[]` for text containing no rows.
2. The pinned 09-07 leaderboard reports `candidatesPerGeneration {1: 4, 2: 5}`,
   `ok: false`, and a violation containing `g2 candidates=5 > 4`.
3. The same leaderboard with `g2_v4` removed reports `ok: true`, no violations.
4. Baseline rows never appear in `candidatesPerGeneration`.
5. A run four generations deep reports a `generations=4` violation.
6. `promotedLineages = 2` reports `promotedLineages=2 > 1`.
7. **Fail-visible through the pure function**: `checkDarwinBounds([], 0)`
   reports `parseStatus: 'unparsable'` and `ok: false` — not only through the
   `checkDarwinBoundsFromStdout` wrapper.
8. **Depth, not cardinality**: rows at g1/g3/g5 report
   `generationsObserved: 3`, `generations: 5`, `ok: false`, and a violation
   `generations=5 > 3`.
9. `verify-entrypoint darwin` on a live leaderboard breaching a bound exits 3
   and prints each violation on the error channel; a compliant leaderboard
   still exits 0; a non-`darwin` label is unaffected.

## 6. References

- `packages/cli/src/darwinBounds.ts`, `packages/cli/src/darwinBounds.test.ts`
- `packages/cli/src/index.ts` — `verify-entrypoint` dispatch
- `packages/compile/src/index.ts:269` — the prose bound this ADR makes checkable
- 2026-09-07 receipts: `eval-darwin.txt` (`outcome=PASSED exit=0 duration=1316ms
  required=true`), `gate.json` (`accepted: true`, `vetoes: []`)
- ADR-0002 — evaluator entrypoint liveness classification
- Superseded draft: PR #11 shipped this as `docs/adr/ADR-0057-…`; renumbered to
  0003 on integration to match this repo's `docs/adrs/` series (the 0056 the
  night assumed came from a different corpus).
