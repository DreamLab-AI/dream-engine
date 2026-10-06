/**
 * Loopback shim between darwin's ruvllm mutator and the Ontology Loom (ADR-0007).
 *
 * Reproduced 2026-10-06: every darwin@0.10.2 night since 2026-09-07 scored all
 * mutants 0.765 because no mutation ever happened. Its RuvllmMutator posts a
 * bare OpenAI chat request; the Loom treats any prompt without
 * `loom_options.scaffold=false` as an ontology lookup and answers in ~15 ms
 * with a verbatim ontology page ("_Served verbatim from the Ontology Loom ...
 * no model generation was performed._"). darwin's validator rejects that page
 * ("secret handling"), leaves the surface unchanged, and the deterministic mock
 * sandbox scores the unchanged variant exactly like the baseline.
 *
 * Turning the scaffold off exposes a second defect: the reasoning model takes
 * ~60 s per mutation and darwin aborts its fetch after a hard-coded 30 s,
 * ignoring RUVLLM_TIMEOUT_MS. That abort only covers the wait for response
 * headers (darwin clears the timer once `fetch` resolves), so this shim sends
 * headers at once and enforces RUVLLM_TIMEOUT_MS on the upstream call itself.
 *
 * The shim never invents a mutation. Anything other than a complete model
 * completion (a scaffold page, a completion cut off by max_tokens, an error,
 * a timeout) becomes `{"choices":[]}`, which darwin records as a no-op.
 */
import { writeFileSync } from 'node:fs';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';

/** Opening of the Loom's verbatim-ontology answer: proof no model ran. */
export const LOOM_SCAFFOLD_MARKER = '_Served verbatim from the Ontology Loom';

/** Reasoning models need room to think before the file; the estate floor is 1536. */
export const DEFAULT_MIN_MAX_TOKENS = 8192;

/** darwin's own default when RUVLLM_TIMEOUT_MS is unset. */
export const DEFAULT_TIMEOUT_MS = 30_000;

/**
 * Node's fetch (undici) gives up on a response body after 300 s. The headers
 * are already sent, so the upstream timeout must stay below that for darwin to
 * receive the shim's `{"choices":[]}` rather than a socket error.
 */
export const MAX_TIMEOUT_MS = 280_000;

export type LoomVerdict = 'ok' | 'scaffold' | 'truncated' | 'empty' | 'upstream-error';

type Json = Record<string, unknown>;

export interface PrepareOptions {
  minMaxTokens: number;
}

/** Force `loom_options.scaffold=false` and floor `max_tokens`; keep every other field. */
export function prepareLoomRequest(body: Json, opts: PrepareOptions): Json {
  const loomOptions =
    body.loom_options && typeof body.loom_options === 'object' ? (body.loom_options as Json) : {};
  const requested = typeof body.max_tokens === 'number' ? body.max_tokens : 0;
  return {
    ...body,
    max_tokens: Math.max(requested, opts.minMaxTokens),
    loom_options: { ...loomOptions, scaffold: false },
  };
}

interface Choice {
  finish_reason?: string;
  message?: { content?: unknown };
}

/** Classify a Loom response; anything but a complete model completion is emptied. */
export function sanitizeLoomResponse(body: unknown): { verdict: LoomVerdict; body: Json } {
  const empty = (verdict: LoomVerdict) => ({ verdict, body: { choices: [] } });
  const choice = (body as { choices?: Choice[] } | null)?.choices?.[0];
  const content = choice?.message?.content;
  if (typeof content !== 'string' || content.trim() === '') return empty('empty');
  if (content.trimStart().startsWith(LOOM_SCAFFOLD_MARKER)) return empty('scaffold');
  if (choice?.finish_reason === 'length') return empty('truncated');
  return { verdict: 'ok', body: body as Json };
}

export interface LoomShimOptions {
  /** The Loom door, with or without a trailing `/v1`. */
  upstream: string;
  timeoutMs: number;
  minMaxTokens: number;
  log: (line: string) => void;
  /** Loopback port to bind; 0 picks a free one. */
  port?: number;
  /**
   * Rewritten with the per-verdict counts (JSON) after every mutator call, so
   * `verify-entrypoint --shim-stats` can read them once darwin exits. Never
   * written before the first call: no file means the shim served nothing.
   */
  statsFile?: string;
}

export interface LoomShim {
  /** Base URL to hand darwin as `--ruvllm-url`. */
  url: string;
  stats(): Record<LoomVerdict, number>;
  close(): Promise<void>;
}

/** Strip trailing slashes and one trailing `/v1`: darwin appends `/v1/chat/completions`. */
function upstreamBase(upstream: string): string {
  return upstream.replace(/\/+$/, '').replace(/\/v1$/, '');
}

function contentPreview(body: Json): string {
  const content = (body as { choices?: Choice[] }).choices?.[0]?.message?.content;
  return typeof content === 'string' ? `${content.length} chars` : 'no content';
}

async function readBody(req: IncomingMessage): Promise<string> {
  let body = '';
  for await (const chunk of req) body += chunk;
  return body;
}

export async function startLoomShim(opts: LoomShimOptions): Promise<LoomShim> {
  const base = upstreamBase(opts.upstream);
  const timeoutMs = Math.min(Math.max(1, opts.timeoutMs), MAX_TIMEOUT_MS);
  const counts: Record<LoomVerdict, number> = {
    ok: 0,
    scaffold: 0,
    truncated: 0,
    empty: 0,
    'upstream-error': 0,
  };
  let seq = 0;
  // Synchronous write: the count must be on disk before darwin sees the reply.
  const record = (verdict: LoomVerdict) => {
    counts[verdict]++;
    if (opts.statsFile) writeFileSync(opts.statsFile, JSON.stringify(counts), 'utf8');
  };

  const handle = async (req: IncomingMessage, res: ServerResponse) => {
    const id = ++seq;
    const started = Date.now();
    const raw = await readBody(req);
    // Headers first: darwin's 30 s abort covers only the wait for them.
    res.writeHead(200, { 'content-type': 'application/json' });
    res.flushHeaders();

    let verdict: LoomVerdict;
    let out: Json;
    let detail: string;
    try {
      const parsed = raw ? (JSON.parse(raw) as Json) : {};
      const upstreamRes = await fetch(`${base}${req.url ?? '/v1/chat/completions'}`, {
        method: req.method ?? 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(prepareLoomRequest(parsed, { minMaxTokens: opts.minMaxTokens })),
        signal: AbortSignal.timeout(timeoutMs),
      });
      const text = await upstreamRes.text();
      let json: unknown = null;
      try {
        json = JSON.parse(text);
      } catch {
        // Non-JSON from upstream is classified as empty below.
      }
      ({ verdict, body: out } = sanitizeLoomResponse(json));
      detail = `http ${upstreamRes.status}, ${verdict === 'ok' ? contentPreview(out) : verdict}`;
    } catch (e) {
      verdict = 'upstream-error';
      out = { choices: [] };
      detail = (e as Error).message;
    }
    record(verdict);
    opts.log(`loom-shim: #${id} ${verdict} in ${Date.now() - started}ms (${detail})`);
    res.end(JSON.stringify(out));
  };

  const server = createServer((req, res) => {
    handle(req, res).catch((e: Error) => {
      record('upstream-error');
      opts.log(`loom-shim: request failed: ${e.message}`);
      if (!res.headersSent) res.writeHead(200, { 'content-type': 'application/json' });
      res.end('{"choices":[]}');
    });
  });
  // Darwin waits on the body for up to MAX_TIMEOUT_MS; Node's default 300 s
  // request timeout must not cut it first.
  server.requestTimeout = 0;
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(opts.port ?? 0, '127.0.0.1', () => resolve());
  });
  const { port } = server.address() as AddressInfo;

  return {
    url: `http://127.0.0.1:${port}`,
    stats: () => ({ ...counts }),
    close: () =>
      new Promise<void>((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}

/** Render the per-verdict counts as one receipt line. */
export function formatStats(stats: Record<LoomVerdict, number>): string {
  return Object.entries(stats)
    .map(([k, v]) => `${k}=${v}`)
    .join(' ');
}
