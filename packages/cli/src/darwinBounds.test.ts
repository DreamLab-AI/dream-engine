import { describe, expect, it } from 'vitest';
import {
  DARWIN_BOUNDS,
  checkDarwinBounds,
  checkDarwinBoundsFromStdout,
  checkDarwinScoreUniformity,
  parseLeaderboardRows,
} from './darwinBounds';

// Fixture: shape of the real 2026-09-07 REQUIRED-evaluator leaderboard
// (run ab4ced4e48b76e83), which PASSED with g2 holding five candidates. Under
// ADR-0003's original <=4 bound that was a breach; ADR-0005 aligned the bound
// with darwin@0.10.2's five-surface map, so five is now the compliant ceiling.
const LB_2026_09_07 = [
  'Darwin Mode — leaderboard',
  '  0.765  baseline  [planner]  safety=1.00  pass=0.60 ◀ winner',
  '  0.765  g1_v0  [planner]  safety=1.00  pass=0.60',
  '  0.765  g1_v1  [toolPolicy]  safety=1.00  pass=0.60',
  '  0.765  g1_v2  [reviewer]  safety=1.00  pass=0.60',
  '  0.765  g1_v3  [toolPolicy]  safety=1.00  pass=0.60',
  '  0.765  g2_v0  [reviewer]  safety=1.00  pass=0.60',
  '  0.765  g2_v1  [planner]  safety=1.00  pass=0.60',
  '  0.765  g2_v2  [contextBuilder]  safety=1.00  pass=0.60',
  '  0.765  g2_v3  [toolPolicy]  safety=1.00  pass=0.60',
  '  0.765  g2_v4  [scorePolicy]  safety=1.00  pass=0.60',
  '',
  'Winner: baseline',
  'Lineage: baseline',
  'Delta over baseline: +0.000',
].join('\n');

// One past the ADR-0005 ceiling: g2 gains a sixth candidate.
const LB_SIX_IN_G2 = LB_2026_09_07.replace(
  '  0.765  g2_v4  [scorePolicy]  safety=1.00  pass=0.60',
  [
    '  0.765  g2_v4  [scorePolicy]  safety=1.00  pass=0.60',
    '  0.765  g2_v5  [planner]  safety=1.00  pass=0.60',
  ].join('\n'),
);

describe('parseLeaderboardRows', () => {
  it('parses the real 2026-09-07 leaderboard: 1 baseline + 9 mutants', () => {
    const rows = parseLeaderboardRows(LB_2026_09_07);
    expect(rows).toHaveLength(10);
    expect(rows.filter((r) => r.id === 'baseline')).toHaveLength(1);
  });

  it('returns [] when no leaderboard lines are present', () => {
    expect(parseLeaderboardRows('nothing to see here')).toEqual([]);
  });
});

describe('checkDarwinBounds', () => {
  it('caps candidates per generation at 5 (ADR-0005, amending ADR-0003)', () => {
    expect(DARWIN_BOUNDS.maxCandidatesPerGeneration).toBe(5);
    expect(DARWIN_BOUNDS.maxGenerations).toBe(3);
    expect(DARWIN_BOUNDS.maxPromotedLineages).toBe(1);
  });

  it('accepts the real 2026-09-07 run: g2 held 5 candidates, within the bound', () => {
    const report = checkDarwinBoundsFromStdout(LB_2026_09_07, 0);
    expect(report.parseStatus).toBe('ok');
    expect(report.candidatesPerGeneration).toEqual({ 1: 4, 2: 5 });
    expect(report.maxCandidatesPerGeneration).toBe(5);
    expect(report.ok).toBe(true);
    expect(report.violations).toEqual([]);
  });

  it('flags 6 candidates in one generation (> 5)', () => {
    const report = checkDarwinBoundsFromStdout(LB_SIX_IN_G2, 0);
    expect(report.parseStatus).toBe('ok');
    expect(report.candidatesPerGeneration).toEqual({ 1: 4, 2: 6 });
    expect(report.maxCandidatesPerGeneration).toBe(6);
    expect(report.ok).toBe(false);
    expect(report.violations).toEqual(['g2 candidates=6 > 5']);
  });

  it('never counts baseline rows as generation candidates', () => {
    const report = checkDarwinBoundsFromStdout(LB_2026_09_07, 0);
    expect(report.candidatesPerGeneration[0]).toBeUndefined();
  });

  it('flags more than 3 generations', () => {
    const rows: { id: string; generation: number }[] = [];
    for (let g = 1; g <= 4; g++) {
      for (let v = 0; v < 2; v++) {
        rows.push({ id: `g${g}_v${v}`, generation: g });
      }
    }
    const report = checkDarwinBounds(rows, 0);
    expect(report.ok).toBe(false);
    expect(report.violations.some((v) => v.startsWith('generations=4'))).toBe(true);
  });

  it('flags more than 1 promoted lineage', () => {
    const rows = parseLeaderboardRows(LB_2026_09_07);
    const report = checkDarwinBounds(rows, 2);
    expect(report.violations).toContain('promotedLineages=2 > 1');
  });

  // Correction (b), 2026-09-07 integration review: the fail-visible branch used to
  // live only in the checkDarwinBoundsFromStdout wrapper, so calling the exported
  // pure function directly with [] returned ok:true alongside parseStatus
  // 'unparsable' — a contradictory report, and a silent pass on input the parser
  // never understood.
  it('is fail-visible on zero rows through the pure function, not just the wrapper', () => {
    const report = checkDarwinBounds([], 0);
    expect(report.parseStatus).toBe('unparsable');
    expect(report.ok).toBe(false);
    expect(report.violations).toContain('leaderboard unparsable: 0 rows read');
  });

  // Correction (c), same review: counting distinct generation labels let a sparse
  // run (g1, g3, g5) report generations=3 and pass, though it evolved five deep.
  it('bounds generation DEPTH, not the count of distinct labels', () => {
    const rows = [1, 3, 5].map((g) => ({ id: `g${g}_v0`, generation: g }));
    const report = checkDarwinBounds(rows, 0);
    expect(report.generationsObserved).toBe(3); // cardinality would have passed
    expect(report.generations).toBe(5); // depth is what the bound constrains
    expect(report.ok).toBe(false);
    expect(report.violations).toContain('generations=5 > 3');
  });
});

describe('checkDarwinBoundsFromStdout', () => {
  it('is fail-visible when no leaderboard rows can be parsed', () => {
    const report = checkDarwinBoundsFromStdout('', 0);
    expect(report.parseStatus).toBe('unparsable');
    expect(report.ok).toBe(false);
    expect(report.violations[0]).toBe('leaderboard unparsable: 0 rows read');
  });
});

// Mutator-diversity diagnostic (2026-10-03): the shape seen on every observed
// pinned-darwin run — every mutant scored exactly at the baseline.
const LB_ONE_MUTANT_DIFFERS = LB_2026_09_07.replace(
  '  0.765  g2_v2  [contextBuilder]  safety=1.00  pass=0.60',
  '  0.801  g2_v2  [contextBuilder]  safety=1.00  pass=0.64',
);

describe('checkDarwinScoreUniformity (diagnostic, never a bound)', () => {
  it('flags the observed run shape: every mutant scores exactly the baseline', () => {
    const r = checkDarwinScoreUniformity(LB_2026_09_07);
    expect(r.uniformWithBaseline).toBe(true);
    expect(r.scoredRows).toBe(10);
    expect(r.mutantsScored).toBe(9);
    expect(r.distinctMutantScores).toBe(1);
    expect(r.ok).toBe(false);
    expect(r.signal).toContain('all 9 mutants score exactly the baseline 0.765');
  });

  it('is clean when one mutant score differs from the baseline', () => {
    const r = checkDarwinScoreUniformity(LB_ONE_MUTANT_DIFFERS);
    expect(r.ok).toBe(true);
    expect(r.signal).toBe('');
    expect(r.uniformWithBaseline).toBe(false);
    expect(r.distinctMutantScores).toBe(2);
  });

  it('does not flag a lone mutant that ties the baseline (too little evidence)', () => {
    const lb = [
      '  0.5  baseline  [planner]  safety=1.00  pass=0.30',
      '  0.5  g1_v0  [planner]  safety=1.00  pass=0.30',
    ].join('\n');
    const r = checkDarwinScoreUniformity(lb);
    expect(r.mutantsScored).toBe(1);
    expect(r.uniformWithBaseline).toBe(false);
    expect(r.ok).toBe(true);
  });

  it('says unassessed when no scored rows can be read', () => {
    const r = checkDarwinScoreUniformity('no leaderboard here');
    expect(r.ok).toBe(false);
    expect(r.scoredRows).toBe(0);
    expect(r.signal).toContain('unassessed');
  });
});
