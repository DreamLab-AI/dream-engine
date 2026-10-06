import { describe, expect, it } from 'vitest';
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

// ADR-0005 lever E: scripts/darwin-entrypoint.sh is the darwin evaluator
// entrypoint. It runs the pinned darwin command THROUGH verify-entrypoint, so
// a bound breach exits non-zero and the annexe records the REQUIRED evaluator
// as FAILED instead of PASSED. These tests drive the real script against a
// stub darwin (test-fixtures/fake-darwin.mjs) printing captured leaderboards;
// they need `npm run build` first, as CI and the annexe build step both do.

const ROOT = fileURLToPath(new URL('../../../', import.meta.url));
const SCRIPT = join(ROOT, 'scripts/darwin-entrypoint.sh');
const FIXTURES = 'packages/cli/test-fixtures';
const FAKE = `${FIXTURES}/fake-darwin.mjs`;
const PIN = '@metaharness/darwin@0.10.2';
const LB_FIVE = `${FIXTURES}/darwin-leaderboard-varied.txt`;
const LB_UNIFORM = `${FIXTURES}/darwin-leaderboard-2026-09-07.txt`;
const LB_SIX = `${FIXTURES}/darwin-leaderboard-six-in-g2.txt`;

function runScript(args: string[], env: Record<string, string> = {}) {
  if (!existsSync(join(ROOT, 'packages/cli/dist/bin.js'))) {
    throw new Error('packages/cli/dist/bin.js missing — run `npm run build` before this suite');
  }
  const r = spawnSync(SCRIPT, args, {
    cwd: ROOT,
    encoding: 'utf8',
    env: { ...process.env, ...env },
  });
  return { code: r.status, stdout: r.stdout, stderr: r.stderr };
}

/** A darwin-shaped command line the script admits, routed to the stub. */
const stub = (...extra: string[]) => [
  'node', FAKE, PIN, 'evolve', '.', '--sandbox', 'mock', ...extra,
];

describe('scripts/darwin-entrypoint.sh', () => {
  it('is checked in executable, as the annexe invokes it directly', () => {
    expect(statSync(SCRIPT).mode & 0o111).not.toBe(0);
  });

  it('exits 0 and keeps the leaderboard in the receipt when g2 holds 5 candidates', () => {
    const r = runScript(stub('--leaderboard', LB_FIVE));
    expect(r.stderr).not.toContain('VIOLATED');
    expect(r.code).toBe(0);
    expect(r.stdout).toContain('g2_v4  [scorePolicy]');
    expect(r.stdout).toContain('darwin: live');
    expect(r.stdout).toContain('darwin bounds ok — depth 2, max 5 candidates/generation');
  });

  it('exits 3 when every mutant scores the same (real 2026-09-07 leaderboard)', () => {
    const r = runScript(stub('--leaderboard', LB_UNIFORM));
    expect(r.code).toBe(3);
    expect(r.stdout).toContain('Delta over baseline: +0.000');
    expect(r.stderr).toContain('darwin bounds VIOLATED');
    expect(r.stderr).toContain('score uniformity: all 9 mutants score 0.765');
  });

  it('exits 3 on a bound breach (6 candidates in g2)', () => {
    const r = runScript(stub('--leaderboard', LB_SIX));
    expect(r.code).toBe(3);
    expect(r.stdout).toContain('g2_v5  [planner]');
    expect(r.stderr).toContain('darwin bounds VIOLATED');
    expect(r.stderr).toContain('g2 candidates=6 > 5');
  });

  it('exits non-zero when darwin itself fails', () => {
    const r = runScript(stub('--exit', '7'));
    expect(r.code).toBe(1);
    expect(r.stdout).toContain('darwin: blocked');
  });

  it('exits non-zero when darwin succeeds silently', () => {
    const r = runScript(stub());
    expect(r.code).toBe(2);
    expect(r.stdout).toContain('darwin: suspicious-silent');
  });

  it('passes arguments through verbatim and the environment to darwin', () => {
    const r = runScript(
      stub('--leaderboard', LB_FIVE, '--note', "two words, a 'quote' and $HOME", '--report-args'),
      { RUVLLM_TIMEOUT_MS: '180000' },
    );
    expect(r.code).toBe(0);
    expect(r.stderr).toContain('"--note","two words, a \'quote\' and $HOME"');
    expect(r.stderr).toContain('fake-darwin RUVLLM_TIMEOUT_MS=180000');
  });

  it.each([
    ['no command', []],
    ['an unpinned darwin', ['npx', '@metaharness/darwin', 'evolve', '.', '--sandbox', 'mock']],
    ['a ranged darwin', ['npx', '@metaharness/darwin@^0.10.2', 'evolve', '.', '--sandbox', 'mock']],
    ['two different pins', [...stub(), '@metaharness/darwin@0.11.0']],
    ['a command that is not darwin', ['node', FAKE, '--leaderboard', LB_FIVE, '--sandbox', 'mock']],
    ['the default real sandbox', ['node', FAKE, PIN, 'evolve', '.', '--leaderboard', LB_FIVE]],
    ['an explicit real sandbox', ['node', FAKE, PIN, 'evolve', '.', '--sandbox', 'real']],
  ])('refuses %s with exit 64 before running anything', (_name, args) => {
    const r = runScript([...args, '--report-args']);
    expect(r.code).toBe(64);
    expect(r.stderr).toContain('darwin-entrypoint:');
    expect(r.stderr).not.toContain('fake-darwin argv=');
  });
});

describe('dream.config.json darwin evaluator', () => {
  const config = JSON.parse(readFileSync(join(ROOT, 'dream.config.json'), 'utf8')) as {
    evaluatorEntrypoints: Record<string, string>;
    extraDisciplines: string[];
  };
  const cmd = config.evaluatorEntrypoints.darwin;

  it('runs darwin through the checked-in entrypoint script', () => {
    expect(cmd).toMatch(/(^|\s)\.\/scripts\/darwin-entrypoint\.sh\s/);
  });

  // The dream-engine annexe's admission (agentbox services/dream-engine
  // readiness.rs) recognises darwin by the package name in the command and
  // refuses it without --sandbox mock|agent. Keeping the full darwin command
  // visible in config, rather than hidden inside the script, keeps that check
  // live.
  it('keeps the pinned darwin command and its sandbox visible to admission', () => {
    expect(cmd).toContain(` ${PIN} evolve . `);
    expect(cmd).toContain('--sandbox mock');
  });

  it('pins the same darwin version the pinned-evaluator discipline records', () => {
    expect(config.extraDisciplines.join('\n')).toContain(PIN);
  });
});
