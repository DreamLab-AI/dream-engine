# ADR-0004: Ledger rows that are not nights carry their own verdict tokens

- **Status**: Accepted
- **Date**: 2026-09-07
- **Related**: ADR-0001 §120, §188 (the three-state invariant — qualified, not overturned, by this record), ADR-0003
- **Deciders**: operator integration, 2026-09-07
- **Tags**: dream-cycle, ledger-signals, evaluation-is-not-promotion

## 1. Context

ADR-0001 fixes the night's outcome vocabulary at `ACCEPT | REJECT |
INCONCLUSIVE` — "never a fourth, never silence" — and
`packages/ledger/src/rowContract.ts` enforced exactly that set on every row.

But not every ledger row is a night. The ledger already carries rows for work
that was never an experiment: the 2026-08-28 fate-reconciliation row and the
2026-09-07 PR #11 integration row are both operator actions, with `operator` in
the witness column and no hypothesis behind them. Lacking a token for that, both
were filed `INCONCLUSIVE`.

That is not a cosmetic mislabel. The engine derives a **dry streak** from the
ledger and counts `INCONCLUSIVE` rows toward it, ignoring any other token; a
long enough streak parks the repo. Filing operator handoffs as `INCONCLUSIVE`
therefore pushes a repo toward being parked on the strength of nights that were
never attempted — the ledger's own signal channel, poisoned by rows that carry
no experimental content at all. The engine already emits `BLOCKED-ENV` and
`HANDOFF` elsewhere (see the website ledger's 09-07 row), so the vocabulary had
already drifted apart from the validator in practice.

A second, smaller defect sat underneath: `verifyLedger` gated on its own private
copy of the vocabulary (`packages/ledger/src/index.ts`), so the row contract and
the verifier could disagree about whether a row was legal.

## 2. Decision

The row vocabulary splits in two, both defined once in
`packages/ledger/src/rowContract.ts` and imported everywhere else:

- `NIGHT_VERDICTS = ACCEPT | REJECT | INCONCLUSIVE` — **unchanged**. A night that
  ran still ends in exactly one of these. ADR-0001's invariant stands, and the
  compiled nightly prompt still states it verbatim; this ADR does not touch it.
- `NON_NIGHT_VERDICTS = BLOCKED-ENV | HANDOFF | OPERATOR` — for rows that record
  something other than a completed experiment: an environment that stopped a
  night starting, a handoff to another actor, an operator acting on the repo
  outside the loop.

The Evaluated column gains `n/a` for the same reason: an operator handoff did
not evaluate anything, and `no` would imply a night chose not to.

`verifyLedger` and the row contract now read the same two constants. A validator
that disagrees with the verifier is worse than either alone, because a row can
pass one gate and fail the other with no single place to look.

The ACCEPT-only rules (`accept-without-pr`, `accept-without-witness`) are
deliberately **not** widened: a non-night row legitimately has no PR and no
computed witness.

## 3. Consequences

- Operator and environment rows stop feeding the dry streak, so the parking
  signal reflects nights that actually ran.
- ADR-0001's three-state invariant is qualified in scope, not overturned: it
  governs night outcomes, which remain three. Per `docs/adrs/INDEX.md`, an
  Accepted ADR is amended by a follow-on record rather than edited in place,
  which is why this is ADR-0004 and ADR-0001 is untouched.
- The vocabulary has one definition. Adding a token is a one-line change in one
  file, and the verifier cannot fall out of step with the contract.
- `verdictStats` (`packages/ledger/src/index.ts`) still buckets only the three
  night outcomes and sends the rest to `other`, and the TUI's "nights" total
  still counts every row. Both were already true of the pre-existing 08-28
  operator row; neither is corrected here, and a TUI that distinguishes nights
  from operator rows is left as follow-on work.
- The engine emits `BLOCKED-ENV`/`HANDOFF` today but this repo's CLI does not
  yet mint them; they are accepted on read before they are produced on write,
  which is the safe ordering.

## 4. Alternatives Considered

- **Keep filing operator rows as `INCONCLUSIVE`.** Rejected: it is the status
  quo, and it is what corrupts the dry streak. The token is load-bearing input
  to an automated decision, not a label.
- **Drop operator rows from the ledger entirely.** Rejected: the ledger is the
  only durable cross-night memory, and the 08-28 row exists precisely because
  fate reconciliation needed to be remembered. Losing that is worse than a
  vocabulary change.
- **Add a separate "row kind" column.** Rejected: the table is a fixed 10-column
  contract that parsers, the TUI and the dashboard all assume. Widening the
  schema to express what one existing column can express is a far larger blast
  radius than three tokens.
- **Let `verifyLedger` keep its own vocabulary copy** and change only the row
  contract, as originally scoped. Rejected on discovery: the row this ADR exists
  to permit would have passed the contract and then been rejected by
  `dream-machine ledger verify`.

## 5. Test Contract

1. Every token in `NIGHT_VERDICTS` validates clean on an otherwise compliant row.
2. Every token in `NON_NIGHT_VERDICTS` validates clean on a row with `n/a` in the
   Evaluated column.
3. A verdict outside the vocabulary still raises `verdict-vocab`.
4. An `OPERATOR` row with no PR and no witness raises neither
   `accept-without-pr` nor `accept-without-witness` — the ACCEPT-only rules did
   not widen with the vocabulary.
5. `NIGHT_VERDICTS` and `NON_NIGHT_VERDICTS` are disjoint, and `LEDGER_VERDICTS`
   is exactly their union — a token in both would make the dry-streak
   distinction meaningless.
6. `verifyLedger` accepts rows carrying `OPERATOR`, `HANDOFF` and `BLOCKED-ENV`,
   and an `n/a` Evaluated column, with zero errors.
7. The real `docs/dream-cycle/LEDGER.md` complies from the enforcement cutoff,
   including the 2026-09-07 `OPERATOR` row.

## 6. References

- `packages/ledger/src/rowContract.ts` — `NIGHT_VERDICTS`, `NON_NIGHT_VERDICTS`,
  `LEDGER_VERDICTS`, `LEDGER_EVALUATED`
- `packages/ledger/src/index.ts` — `Verdict`, `Evaluated`, `verifyLedger`
- `docs/dream-cycle/LEDGER.md` — 2026-08-28 and 2026-09-07 operator rows
- ADR-0001 §120, §188 — the three-state night invariant this record scopes
- `README.md`, `packages/cli/README.md` — user-facing wording
