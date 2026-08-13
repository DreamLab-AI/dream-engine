import { describe, it, expect } from 'vitest';
import { createHash } from 'node:crypto';
import { stamp, verify, hashReport, witnessFromHash, normalizeCommit, verifySteps } from './index.js';

const COMMIT = '68402755f017e0df5f493c6ee608218420540d17';

function refWitness(report: string, commit: string): string {
  const rh = createHash('sha256').update(report).digest('hex');
  return createHash('sha256').update(rh + commit).digest('hex');
}

describe('witness stamp', () => {
  it('matches an independent double-sha256 reference', () => {
    const report = '# Dream Cycle 2026-08-13\nverdict: INCONCLUSIVE\n';
    const w = stamp(report, COMMIT);
    expect(w.reportHash).toBe(createHash('sha256').update(report).digest('hex'));
    expect(w.witness).toBe(refWitness(report, COMMIT));
    expect(w.sessionCommit).toBe(COMMIT);
  });

  it('is deterministic', () => {
    expect(stamp('x', COMMIT).witness).toBe(stamp('x', COMMIT).witness);
  });

  it('hashReport handles Uint8Array and string identically', () => {
    const s = 'nebula';
    expect(hashReport(s)).toBe(hashReport(new TextEncoder().encode(s)));
  });

  it('witnessFromHash equals stamp.witness', () => {
    const report = 'body';
    const w = stamp(report, COMMIT);
    expect(witnessFromHash(w.reportHash, COMMIT)).toBe(w.witness);
  });

  it('rejects a bad commit', () => {
    expect(() => stamp('x', 'nothex!!')).toThrow(/hex/);
    expect(() => normalizeCommit('ZZZ')).toThrow();
  });

  it('lowercases + trims the commit', () => {
    expect(normalizeCommit('  68402755F017E0DF  ')).toBe('68402755f017e0df');
  });

  it('witnessFromHash rejects a non-64-hex report hash', () => {
    expect(() => witnessFromHash('abc', COMMIT)).toThrow(/64 hex/);
  });
});

describe('witness verify', () => {
  it('accepts a genuine stamp', () => {
    const report = 'genuine report bytes';
    const w = stamp(report, COMMIT);
    expect(verify(report, COMMIT, w.witness).ok).toBe(true);
  });

  it('accepts case-insensitively and trims', () => {
    const w = stamp('r', COMMIT);
    expect(verify('r', COMMIT, `  ${w.witness.toUpperCase()}  `).ok).toBe(true);
  });

  it('rejects a single-byte mutation of the report', () => {
    const w = stamp('report', COMMIT);
    const r = verify('reporu', COMMIT, w.witness);
    expect(r.ok).toBe(false);
    expect(r.reason).toMatch(/mismatch/);
  });

  it('rejects a different commit', () => {
    const w = stamp('report', COMMIT);
    const other = 'a'.repeat(40);
    expect(verify('report', other, w.witness).ok).toBe(false);
  });

  it('rejects a malformed claimed witness', () => {
    expect(verify('report', COMMIT, 'deadbeef').ok).toBe(false);
  });

  it('reports a bad-commit reason without throwing', () => {
    const r = verify('report', 'nothex', 'a'.repeat(64));
    expect(r.ok).toBe(false);
    expect(r.reason).toMatch(/hex/);
  });
});

describe('verifySteps text', () => {
  it('embeds the raw url and the double-hash recipe', () => {
    const t = verifySteps('https://example/raw');
    expect(t).toContain('https://example/raw');
    expect(t).toContain('sha256sum');
    expect(t).toContain('SESSION_COMMIT');
  });
});
