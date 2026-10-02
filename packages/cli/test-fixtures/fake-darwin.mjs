#!/usr/bin/env node
// Test stub standing in for `npx @metaharness/darwin@<pin> evolve ...` so the
// darwin entrypoint script can be exercised without the Loom endpoint or a
// multi-minute evolve run. Every darwin argument is accepted and ignored; the
// stub only honours its own flags:
//   --leaderboard <file>  print this captured leaderboard on stdout
//   --exit <n>            exit with status n (default 0)
//   --report-args         print argv (JSON) and RUVLLM_TIMEOUT_MS on stderr
import { readFileSync } from 'node:fs';
import process from 'node:process';

const argv = process.argv.slice(2);
const flag = (name) => {
  const i = argv.indexOf(name);
  return i === -1 ? undefined : argv[i + 1];
};

if (argv.includes('--report-args')) {
  process.stderr.write(`fake-darwin argv=${JSON.stringify(argv)}\n`);
  process.stderr.write(`fake-darwin RUVLLM_TIMEOUT_MS=${process.env.RUVLLM_TIMEOUT_MS ?? ''}\n`);
}
const leaderboard = flag('--leaderboard');
if (leaderboard) process.stdout.write(readFileSync(leaderboard, 'utf8'));
process.exit(Number(flag('--exit') ?? 0));
