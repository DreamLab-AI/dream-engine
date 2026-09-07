/**
 * Darwin bound policy — nightly pipeline step 10:
 *   generations ≤ 3, candidates/generation ≤ 4, promoted lineages ≤ 1.
 *
 * Motivation (2026-09-07, run ab4ced4e48b76e83): the REQUIRED darwin
 * evaluator returned outcome=PASSED while its leaderboard listed five
 * candidates in generation 2 (g2_v0..g2_v4) — a silent bound breach.
 * This module makes the bound machine-checkable: additive-only and
 * dependency-free (pattern proven on 2026-09-06).
 *
 * This is DETECTION, not enforcement. Nothing here can veto a night: the
 * nightly gate lives in the external annexe runner, not in this repo. See
 * docs/adrs/ADR-0003-darwin-bound-guard.md.
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
  maxCandidatesPerGeneration: 4,
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
