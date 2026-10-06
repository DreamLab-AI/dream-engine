import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
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
// with darwin@0.10.2's five-surface map, so five is within bounds. Every mutant
// scores 0.765, though, so ADR-0006 fails it as a non-discriminating scorer.
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

// A discriminating leaderboard of the same 4 + 5 shape: mutant scores differ,
// so the scorer is actually judging. This is the compliant ceiling.
const LB_VARIED = readFileSync(
  new URL('../test-fixtures/darwin-leaderboard-varied.txt', import.meta.url),
  'utf8',
);

// One past the ADR-0005 ceiling: g2 gains a sixth candidate.
const LB_SIX_IN_G2 = readFileSync(
  new URL('../test-fixtures/darwin-leaderboard-six-in-g2.txt', import.meta.url),
  'utf8',
);

describe('parseLeaderboardRows', () => {
  it('parses the real 2026-09-07 leaderboard: 1 baseline + 9 mutants', () => {
    const rows = parseLeaderboardRows(LB_2026_09_07);
    expect(rows).toHaveLength(10);
    expect(rows.filter((r) => r.id === 'baseline')).toHaveLength(1);
    expect(rows.every((r) => r.score === 0.765)).toBe(true);
  });

  it('reads each row\'s score and generation', () => {
    const rows = parseLeaderboardRows(LB_VARIED);
    expect(rows[0]).toEqual({ id: 'g2_v1', generation: 2, score: 0.812 });
    expect(rows.find((r) => r.id === 'baseline')).toEqual({ id: 'baseline', generation: 0, score: 0.765 });
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

  it('accepts 5 candidates in g2 with discriminating scores, within the bound', () => {
    const report = checkDarwinBoundsFromStdout(LB_VARIED, 0);
    expect(report.parseStatus).toBe('ok');
    expect(report.candidatesPerGeneration).toEqual({ 1: 4, 2: 5 });
    expect(report.maxCandidatesPerGeneration).toBe(5);
    expect(report.mutantsScored).toBe(9);
    expect(report.distinctMutantScores).toBe(9);
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
    const rows = parseLeaderboardRows(LB_VARIED);
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

// Score uniformity (2026-10-03, wired in 2026-10-06): the shape seen on every
// observed pinned-darwin run — every mutant scored identically — means the scorer
// is not discriminating, so the leaderboard's winner is an artefact.
describe('score uniformity', () => {
  it('flags the real 2026-09-07 run: all 9 mutants score 0.765, equal to baseline', () => {
    const report = checkDarwinBoundsFromStdout(LB_2026_09_07, 0);
    expect(report.ok).toBe(false);
    expect(report.mutantsScored).toBe(9);
    expect(report.distinctMutantScores).toBe(1);
    // Within the step-10 bounds: uniformity is the only violation.
    expect(report.violations).toEqual([
      'score uniformity: all 9 mutants score 0.765, equal to baseline — scorer not discriminating',
    ]);
  });

  it('flags mutants that agree with each other but not the baseline', () => {
    const lb = [
      '  0.700  baseline  [planner]  safety=1.00  pass=0.50',
      '  0.900  g1_v0  [planner]  safety=1.00  pass=0.80',
      '  0.900  g1_v1  [reviewer]  safety=1.00  pass=0.80',
    ].join('\n');
    const r = checkDarwinScoreUniformity(parseLeaderboardRows(lb));
    expect(r.violation).toBe('score uniformity: all 2 mutants score 0.9, baseline 0.7 — scorer not discriminating');
  });

  it('is clean when a single mutant score differs', () => {
    const lb = LB_2026_09_07.replace(
      '  0.765  g2_v2  [contextBuilder]  safety=1.00  pass=0.60',
      '  0.801  g2_v2  [contextBuilder]  safety=1.00  pass=0.64',
    );
    const report = checkDarwinBoundsFromStdout(lb, 0);
    expect(report.distinctMutantScores).toBe(2);
    expect(report.ok).toBe(true);
    expect(report.violations).toEqual([]);
  });

  it('does not flag fewer than 2 scored mutants (too little evidence)', () => {
    const lb = [
      '  0.5  baseline  [planner]  safety=1.00  pass=0.30',
      '  0.5  g1_v0  [planner]  safety=1.00  pass=0.30',
    ].join('\n');
    const report = checkDarwinBoundsFromStdout(lb, 0);
    expect(report.mutantsScored).toBe(1);
    expect(report.ok).toBe(true);
  });

  it('ignores hand-built rows with no score', () => {
    const rows = [0, 1, 2].map((v) => ({ id: `g1_v${v}`, generation: 1 }));
    const r = checkDarwinScoreUniformity(rows);
    expect(r.mutantsScored).toBe(0);
    expect(r.violation).toBe('');
  });
});
