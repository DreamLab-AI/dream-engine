#!/usr/bin/env node
// Test stub standing in for `npx @metaharness/darwin@<pin> evolve ...` so the
// darwin entrypoint script can be exercised without the Loom endpoint or a
// multi-minute evolve run. Every darwin argument is accepted and ignored; the
// stub only honours its own flags:
//   --leaderboard <file>  print this captured leaderboard on stdout
//   --exit <n>            exit with status n (default 0)
//   --report-args         print argv (JSON) and RUVLLM_TIMEOUT_MS on stderr
//   --call-mutator        make ONE mutator call the way darwin@0.10.2's
//                         RuvllmMutator does (POST <--ruvllm-url>/v1/chat/completions,
//                         aborting if headers take longer than FAKE_DARWIN_ABORT_MS,
//                         default 300 — darwin's hard-coded 30 s, scaled down) and
//                         print the outcome on stderr
/* global fetch, AbortController, setTimeout, clearTimeout */
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
if (argv.includes('--call-mutator')) {
  const base = (flag('--ruvllm-url') ?? 'http://localhost:8080').replace(/\/$/, '');
  const controller = new AbortController();
  const tid = setTimeout(() => controller.abort(), Number(process.env.FAKE_DARWIN_ABORT_MS ?? 300));
  let res;
  try {
    res = await fetch(`${base}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        model: flag('--ruvllm-model') ?? 'local',
        messages: [{ role: 'user', content: 'improve the planner surface' }],
        max_tokens: 2000,
        temperature: 0.4,
      }),
      signal: controller.signal,
    });
    clearTimeout(tid);
  } catch (e) {
    process.stderr.write(`fake-darwin mutator unreachable (${e.message})\n`);
  }
  if (res) {
    const j = await res.json();
    const content = j.choices?.[0]?.message?.content;
    process.stderr.write(
      content ? `fake-darwin mutator content=${JSON.stringify(content)}\n` : 'fake-darwin mutator no content\n',
    );
  }
}
const leaderboard = flag('--leaderboard');
if (leaderboard) process.stdout.write(readFileSync(leaderboard, 'utf8'));
process.exit(Number(flag('--exit') ?? 0));
