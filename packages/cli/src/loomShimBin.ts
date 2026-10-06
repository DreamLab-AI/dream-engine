#!/usr/bin/env node
/**
 * Run the darwin→Loom shim (ADR-0007) until SIGTERM/SIGINT.
 *
 *   node loomShimBin.js --upstream <loom-url> --url-file <path> [--stats-file <path>]
 *
 * Writes the shim's base URL to --url-file once it is listening, logs one line
 * per mutator call on stderr, and prints a per-verdict summary on exit. With
 * --stats-file, the counts are also rewritten as JSON after every call, for
 * `verify-entrypoint --shim-stats` (ADR-0007).
 * Environment: RUVLLM_TIMEOUT_MS (upstream timeout, default 30000, capped at
 * 280000) and LOOM_SHIM_MIN_MAX_TOKENS (max_tokens floor, default 8192).
 */
import { writeFile } from 'node:fs/promises';
import process from 'node:process';
import {
  DEFAULT_MIN_MAX_TOKENS,
  DEFAULT_TIMEOUT_MS,
  formatStats,
  startLoomShim,
} from './loomShim.js';
import { parseArgs } from './index.js';

function positiveInt(raw: string | undefined, fallback: number, name: string): number {
  if (raw === undefined || raw === '') return fallback;
  const n = Number(raw);
  if (!Number.isInteger(n) || n <= 0) throw new Error(`${name} must be a positive integer, got '${raw}'`);
  return n;
}

async function main(): Promise<void> {
  const { flags } = parseArgs(process.argv.slice(2));
  const upstream = flags.upstream;
  const urlFile = flags['url-file'];
  if (typeof upstream !== 'string' || typeof urlFile !== 'string') {
    throw new Error('usage: loomShimBin --upstream <loom-url> --url-file <path>');
  }
  const statsFile = typeof flags['stats-file'] === 'string' ? flags['stats-file'] : undefined;
  const log = (line: string) => process.stderr.write(`${line}\n`);
  const shim = await startLoomShim({
    upstream,
    timeoutMs: positiveInt(process.env.RUVLLM_TIMEOUT_MS, DEFAULT_TIMEOUT_MS, 'RUVLLM_TIMEOUT_MS'),
    minMaxTokens: positiveInt(
      process.env.LOOM_SHIM_MIN_MAX_TOKENS,
      DEFAULT_MIN_MAX_TOKENS,
      'LOOM_SHIM_MIN_MAX_TOKENS',
    ),
    log,
    statsFile,
  });
  log(`loom-shim: ${shim.url} -> ${upstream} (scaffold off)`);
  await writeFile(urlFile, shim.url, 'utf8');

  const stop = async () => {
    log(`loom-shim: summary ${formatStats(shim.stats())}`);
    await shim.close();
    process.exit(0);
  };
  process.once('SIGTERM', stop);
  process.once('SIGINT', stop);
}

main().catch((e: Error) => {
  process.stderr.write(`loom-shim: ${e.message}\n`);
  process.exit(1);
});
