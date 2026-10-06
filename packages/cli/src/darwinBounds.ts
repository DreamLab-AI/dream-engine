/**
 * Darwin bound policy — nightly pipeline step 10:
 *   generations ≤ 3, candidates/generation ≤ 5, promoted lineages ≤ 1.
 *
 * Motivation (2026-09-07, run ab4ced4e48b76e83): the REQUIRED darwin
 * evaluator returned outcome=PASSED while its leaderboard listed five
 * candidates in generation 2 (g2_v0..g2_v4) — a silent breach of the then
 * ≤ 4 bound. ADR-0005 later raised the bound to 5: darwin@0.10.2 mutates one
 * candidate per surface of its five-surface map, so five is its behaviour.
 * This module makes the bound machine-checkable: additive-only and
 * dependency-free (pattern proven on 2026-09-06).
 *
 * It also fails a leaderboard where two or more mutants all score identically
 * (ADR-0006): such a scorer is not discriminating.
 *
 * This module only DETECTS. It becomes enforcement through
 * scripts/darwin-entrypoint.sh (ADR-0005), which runs darwin through
 * `verify-entrypoint` so a breach fails the REQUIRED evaluator; the gate that
 * honours that failure lives in the external annexe runner. See
 * docs/adrs/ADR-0003-darwin-bound-guard.md, ADR-0005-darwin-bound-five.md and
 * ADR-0006-darwin-score-uniformity.md.
 */

export interface DarwinLeaderboardRow {
  id: string;
  generation: number;
  /**
   * The row's leaderboard score. Always set by `parseLeaderboardRows`; a row
   * built by hand without one is left out of the score-uniformity check.
   */
  score?: number;
}

export interface DarwinBoundsReport {
  ok: boolean;
  parseStatus: 'ok' | 'unparsable';
  /**
   * Deepest generation index observed — the bound-relevant number. A run
   * producing g1, g3, g5 evolved five generations deep even though it emitted
   * only three distinct labels, so depth (not cardinality) is what `maxGenerations`
   * constrains.
   */
  generations: number;
  /** Count of distinct generation labels seen. Reporting only, never a bound. */
  generationsObserved: number;
  candidatesPerGeneration: Record<number, number>;
  maxCandidatesPerGeneration: number;
  promotedLineages: number;
  /** Non-baseline rows that carry a score. */
  mutantsScored: number;
  /** Distinct scores among those mutants; 1 with ≥ 2 mutants is a violation. */
  distinctMutantScores: number;
  violations: string[];
}

export const DARWIN_BOUNDS = {
  maxGenerations: 3,
  // Aligned with darwin@0.10.2 behaviour (5-surface map) on 2026-10-02 per ADR-0005.
  maxCandidatesPerGeneration: 5,
  maxPromotedLineages: 1,
} as const;

const ROW_RE = /^\s*([\d.]+)\s+(baseline|g(\d+)_v(\d+))\s/;

export function parseLeaderboardRows(stdout: string): DarwinLeaderboardRow[] {
  const rows: DarwinLeaderboardRow[] = [];
  for (const line of stdout.split('\n')) {
    const m = ROW_RE.exec(line);
    if (!m) continue;
    const score = Number(m[1]);
    rows.push(
      m[2] === 'baseline'
        ? { id: 'baseline', generation: 0, score }
        : { id: m[2], generation: Number(m[3]), score },
    );
  }
  return rows;
}

/**
 * Check a parsed leaderboard against DARWIN_BOUNDS, and that its scores
 * discriminate (see `checkDarwinScoreUniformity`).
 *
 * Fail-visible by construction: an empty row set reports
 * `parseStatus: 'unparsable'` AND `ok: false`. The two can never disagree, so a
 * caller reaching for this pure function cannot get a silent pass on input the
 * parser did not understand — precisely the failure mode the module exists to
 * eliminate.
 */
export function checkDarwinBounds(
  rows: DarwinLeaderboardRow[],
  promotedLineages: number,
): DarwinBoundsReport {
  const perGen = new Map<number, number>();
  for (const row of rows) {
    if (row.id === 'baseline') continue;
    perGen.set(row.generation, (perGen.get(row.generation) ?? 0) + 1);
  }

  const violations: string[] = [];
  if (rows.length === 0) {
    violations.push('leaderboard unparsable: 0 rows read');
  }

  // Bound on DEPTH, not cardinality: g1+g3+g5 is five generations deep even
  // though perGen.size is 3.
  const deepestGeneration = perGen.size === 0 ? 0 : Math.max(...perGen.keys());
  if (deepestGeneration > DARWIN_BOUNDS.maxGenerations) {
    violations.push(`generations=${deepestGeneration} > ${DARWIN_BOUNDS.maxGenerations}`);
  }

  let maxCandidates = 0;
  for (const [gen, count] of perGen) {
    maxCandidates = Math.max(maxCandidates, count);
    if (count > DARWIN_BOUNDS.maxCandidatesPerGeneration) {
      violations.push(
        `g${gen} candidates=${count} > ${DARWIN_BOUNDS.maxCandidatesPerGeneration}`,
      );
    }
  }
  const uniformity = checkDarwinScoreUniformity(rows);
  if (uniformity.violation) violations.push(uniformity.violation);

  if (promotedLineages > DARWIN_BOUNDS.maxPromotedLineages) {
    violations.push(
      `promotedLineages=${promotedLineages} > ${DARWIN_BOUNDS.maxPromotedLineages}`,
    );
  }

  const candidatesPerGeneration: Record<number, number> = {};
  for (const [gen, count] of perGen) {
    candidatesPerGeneration[gen] = count;
  }

  return {
    ok: violations.length === 0,
    parseStatus: rows.length === 0 ? 'unparsable' : 'ok',
    generations: deepestGeneration,
    generationsObserved: perGen.size,
    candidatesPerGeneration,
    maxCandidatesPerGeneration: maxCandidates,
    promotedLineages,
    mutantsScored: uniformity.mutantsScored,
    distinctMutantScores: uniformity.distinctMutantScores,
    violations,
  };
}

/**
 * Convenience wrapper: parse evaluator stdout, then check it. Kept for callers
 * holding raw output; the fail-visible guarantee now lives in
 * `checkDarwinBounds` itself, so this adds parsing and nothing else.
 */
export function checkDarwinBoundsFromStdout(
  stdout: string,
  promotedLineages: number,
): DarwinBoundsReport {
  return checkDarwinBounds(parseLeaderboardRows(stdout), promotedLineages);
}

export interface DarwinUniformityReport {
  /** Non-baseline rows that carry a score. */
  mutantsScored: number;
  /** Distinct scores among those mutants. */
  distinctMutantScores: number;
  /** The violation line for `DarwinBoundsReport.violations`, or '' when none. */
  violation: string;
}

/**
 * Score-uniformity check, part of `checkDarwinBounds`.
 *
 * A leaderboard where two or more scored mutants all carry the identical score
 * has no selection signal: the scorer returned a default, a fallback answer, or
 * a parse-error value rather than judging the candidates. The 2026-09-07 run
 * (run ab4ced4e48b76e83) and every pinned @metaharness/darwin@0.10.2 receipt
 * since have this shape: every mutant at 0.765, "Delta over baseline: +0.000".
 * Its winner is then an artefact, not a result, so the run fails the bound check
 * the same way an over-wide generation does.
 *
 * Fewer than two scored mutants is too little evidence and is not a violation;
 * the baseline row is reported against but never counted as a mutant.
 */
export function checkDarwinScoreUniformity(rows: DarwinLeaderboardRow[]): DarwinUniformityReport {
  const scores: number[] = [];
  for (const row of rows) {
    if (row.id !== 'baseline' && row.score !== undefined) scores.push(row.score);
  }
  const distinct = new Set(scores);
  let violation = '';
  if (scores.length >= 2 && distinct.size === 1) {
    const baseline = rows.find((r) => r.id === 'baseline')?.score;
    const relation =
      baseline === undefined ? '' : baseline === scores[0] ? ', equal to baseline' : `, baseline ${baseline}`;
    violation = `score uniformity: all ${scores.length} mutants score ${scores[0]}${relation} — scorer not discriminating`;
  }
  return { mutantsScored: scores.length, distinctMutantScores: distinct.size, violation };
}
