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
 * This module only DETECTS. It becomes enforcement through
 * scripts/darwin-entrypoint.sh (ADR-0005), which runs darwin through
 * `verify-entrypoint` so a breach fails the REQUIRED evaluator; the gate that
 * honours that failure lives in the external annexe runner. See
 * docs/adrs/ADR-0003-darwin-bound-guard.md and ADR-0005-darwin-bound-five.md.
 */

export interface DarwinLeaderboardRow {
  id: string;
  generation: number;
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
  violations: string[];
}

export const DARWIN_BOUNDS = {
  maxGenerations: 3,
  // Aligned with darwin@0.10.2 behaviour (5-surface map) on 2026-10-02 per ADR-0005.
  maxCandidatesPerGeneration: 5,
  maxPromotedLineages: 1,
} as const;

const ROW_RE = /^\s*[\d.]+\s+(baseline|g(\d+)_v(\d+))\s/;

export function parseLeaderboardRows(stdout: string): DarwinLeaderboardRow[] {
  const rows: DarwinLeaderboardRow[] = [];
  for (const line of stdout.split('\n')) {
    const m = ROW_RE.exec(line);
    if (!m) continue;
    rows.push(
      m[1] === 'baseline'
        ? { id: 'baseline', generation: 0 }
        : { id: m[1], generation: Number(m[2]) },
    );
  }
  return rows;
}

/**
 * Check a parsed leaderboard against DARWIN_BOUNDS.
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

const SCORED_ROW_RE = /^\s*([\d.]+)\s+(baseline|g\d+_v\d+)\s/;

export interface DarwinUniformityReport {
  /** true when no uniformity signal was emitted — diagnostic, never a bound. */
  ok: boolean;
  /** Human-readable signal; '' when ok. */
  signal: string;
  /** Leaderboard rows (baseline + mutants) from which a score was read. */
  scoredRows: number;
  /** Non-baseline rows from which a score was read. */
  mutantsScored: number;
  /** Distinct scores among mutants. Witness datum, not a threshold. */
  distinctMutantScores: number;
  /** Every scored mutant matched the baseline exactly (delta +0.000 shape). */
  uniformWithBaseline: boolean;
}

/**
 * Diagnostic for the mutator-diversity question carried since 2026-09-08 (and
 * again 2026-09-29): every darwin leaderboard observed from the pinned
 * @metaharness/darwin@0.10.2 against the :8084 ruvllm endpoint — the 2026-09-07
 * run and every receipt from 2026-09-27 through 2026-10-03 — scores ALL mutants
 * exactly at the baseline (uniform 0.765, Delta over baseline: +0.000). Such a
 * leaderboard carries no selection signal: either the mutator proposed nothing
 * distinguishable, or the endpoint is not really scoring (the 09-08 operator
 * note names the Loom-façade possibility).
 *
 * Like this module's bounds before ADR-0005, this only DETECTS: it adds no
 * violation and changes no exit code, and must not gate anything until a human
 * decides the policy — tonight's live run is itself uniform, so wiring this in
 * as a veto would fail every REQUIRED darwin evaluation until the endpoint or
 * the mutator changes.
 */
export function checkDarwinScoreUniformity(stdout: string): DarwinUniformityReport {
  const scored: { id: string; score: number }[] = [];
  for (const line of stdout.split('\n')) {
    const m = SCORED_ROW_RE.exec(line);
    if (m) scored.push({ id: m[2], score: Number(m[1]) });
  }
  const mutants = scored.filter((r) => r.id !== 'baseline');
  const baselineScore = scored.find((r) => r.id === 'baseline')?.score;
  const distinctMutantScores = new Set(mutants.map((r) => r.score)).size;
  const uniformWithBaseline =
    baselineScore !== undefined &&
    mutants.length >= 2 &&
    mutants.every((r) => r.score === baselineScore);

  let signal = '';
  if (scored.length === 0) {
    signal = 'score uniformity unassessed: 0 scored rows read';
  } else if (uniformWithBaseline) {
    signal = `score uniformity suspect: all ${mutants.length} mutants score exactly the baseline ${baselineScore}`;
  }

  return {
    ok: signal === '',
    signal,
    scoredRows: scored.length,
    mutantsScored: mutants.length,
    distinctMutantScores,
    uniformWithBaseline,
  };
}
