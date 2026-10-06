import { afterEach, describe, expect, it } from 'vitest';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import {
  LOOM_SCAFFOLD_MARKER,
  prepareLoomRequest,
  sanitizeLoomResponse,
  startLoomShim,
  type LoomShim,
} from './loomShim.js';

// ADR-0007: darwin@0.10.2's RuvllmMutator posts a bare OpenAI request to the
// Ontology Loom. The Loom answers a non-ontology prompt with a verbatim
// ontology page unless the request carries loom_options.scaffold=false; the
// page fails darwin's validator, every mutation becomes a no-op, and every
// mutant scores the baseline's 0.765.

const SCAFFOLD_PAGE =
  `${LOOM_SCAFFOLD_MARKER} (generation: visionGraph@ae913f93); no model generation was performed._\n\n` +
  '## Package Manager (artificial-intelligence, maturity: mature)\nA package manager is a tool ...';

const completion = (content: string, finish = 'stop') => ({
  choices: [{ index: 0, finish_reason: finish, message: { role: 'assistant', content } }],
});

describe('prepareLoomRequest', () => {
  it('turns the Loom scaffold off and keeps the rest of the request', () => {
    const out = prepareLoomRequest(
      { model: 'qwen3.8-27B', messages: [{ role: 'user', content: 'x' }], max_tokens: 2000, temperature: 0.4 },
      { minMaxTokens: 1536 },
    );
    expect(out.loom_options).toEqual({ scaffold: false });
    expect(out.model).toBe('qwen3.8-27B');
    expect(out.temperature).toBe(0.4);
    expect(out.messages).toEqual([{ role: 'user', content: 'x' }]);
  });

  it('keeps other loom_options but always forces scaffold false', () => {
    const out = prepareLoomRequest({ loom_options: { scaffold: true, trace: 1 } }, { minMaxTokens: 1536 });
    expect(out.loom_options).toEqual({ scaffold: false, trace: 1 });
  });

  it('raises max_tokens to the floor so reasoning cannot crowd out the file', () => {
    expect(prepareLoomRequest({ max_tokens: 2000 }, { minMaxTokens: 8192 }).max_tokens).toBe(8192);
    expect(prepareLoomRequest({}, { minMaxTokens: 8192 }).max_tokens).toBe(8192);
    expect(prepareLoomRequest({ max_tokens: 16000 }, { minMaxTokens: 8192 }).max_tokens).toBe(16000);
  });
});

describe('sanitizeLoomResponse', () => {
  it('passes a model completion through untouched', () => {
    const r = sanitizeLoomResponse(completion('export const maxAttempts = 4;\n'));
    expect(r.verdict).toBe('ok');
    expect(r.body).toEqual(completion('export const maxAttempts = 4;\n'));
  });

  it('empties a verbatim ontology page so darwin records a no-op, not a bogus file', () => {
    const r = sanitizeLoomResponse(completion(SCAFFOLD_PAGE));
    expect(r.verdict).toBe('scaffold');
    expect(r.body.choices).toEqual([]);
  });

  it('empties a completion cut off by max_tokens: a truncated file must never be written', () => {
    const r = sanitizeLoomResponse(completion('export function plan() {\n  return [', 'length'));
    expect(r.verdict).toBe('truncated');
    expect(r.body.choices).toEqual([]);
  });

  it('reports a response with no content', () => {
    expect(sanitizeLoomResponse({ error: { message: 'boom' } }).verdict).toBe('empty');
    expect(sanitizeLoomResponse(completion('')).verdict).toBe('empty');
  });
});

describe('startLoomShim', () => {
  let upstream: Server | undefined;
  let shim: LoomShim | undefined;

  afterEach(async () => {
    await shim?.close();
    await new Promise<void>((resolve) => (upstream ? upstream.close(() => resolve()) : resolve()));
    upstream = shim = undefined;
  });

  /** A fake Loom that records each request body and answers after `delayMs`. */
  async function fakeLoom(reply: unknown, delayMs = 0) {
    const seen: Record<string, unknown>[] = [];
    upstream = createServer(async (req, res) => {
      let body = '';
      for await (const chunk of req) body += chunk;
      seen.push({ url: req.url, ...JSON.parse(body) });
      setTimeout(() => {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(JSON.stringify(reply));
      }, delayMs);
    });
    await new Promise<void>((resolve) => upstream!.listen(0, '127.0.0.1', resolve));
    return { url: `http://127.0.0.1:${(upstream.address() as AddressInfo).port}`, seen };
  }

  const post = (url: string, body: unknown, signal?: AbortSignal) =>
    fetch(`${url}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      signal,
    });

  it('forwards to the Loom with scaffold off and returns the completion', async () => {
    const loom = await fakeLoom(completion('export const maxAttempts = 5;\n'));
    shim = await startLoomShim({ upstream: loom.url, timeoutMs: 5000, minMaxTokens: 8192, log: () => {} });
    const res = await post(shim.url, { model: 'm', messages: [], max_tokens: 2000 });
    const json = (await res.json()) as ReturnType<typeof completion>;
    expect(json.choices[0].message.content).toBe('export const maxAttempts = 5;\n');
    expect(loom.seen).toHaveLength(1);
    expect(loom.seen[0]).toMatchObject({
      url: '/v1/chat/completions',
      loom_options: { scaffold: false },
      max_tokens: 8192,
    });
  });

  it('accepts an upstream given with a trailing /v1, as the Loom door is usually written', async () => {
    const loom = await fakeLoom(completion('x\n'));
    shim = await startLoomShim({ upstream: `${loom.url}/v1/`, timeoutMs: 5000, minMaxTokens: 1536, log: () => {} });
    await (await post(shim.url, { messages: [] })).json();
    expect(loom.seen[0].url).toBe('/v1/chat/completions');
  });

  // darwin@0.10.2 aborts its fetch 30 s after sending and never reads
  // RUVLLM_TIMEOUT_MS; a reasoning model behind the Loom takes ~60 s. The
  // abort only covers the wait for response headers, so the shim sends them
  // at once and enforces RUVLLM_TIMEOUT_MS on the upstream call itself.
  it('sends headers before the Loom answers, so a short client header timeout cannot fire', async () => {
    const loom = await fakeLoom(completion('slow but real\n'), 600);
    shim = await startLoomShim({ upstream: loom.url, timeoutMs: 5000, minMaxTokens: 1536, log: () => {} });
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 200); // darwin's 30 s, scaled down
    const res = await post(shim.url, { messages: [] }, controller.signal);
    clearTimeout(timer);
    const json = (await res.json()) as ReturnType<typeof completion>;
    expect(json.choices[0].message.content).toBe('slow but real\n');
  });

  it('answers with no choices when the Loom exceeds the timeout', async () => {
    const loom = await fakeLoom(completion('too late\n'), 1000);
    const lines: string[] = [];
    shim = await startLoomShim({ upstream: loom.url, timeoutMs: 150, minMaxTokens: 1536, log: (l) => lines.push(l) });
    const json = (await (await post(shim.url, { messages: [] })).json()) as { choices: unknown[] };
    expect(json.choices).toEqual([]);
    expect(lines.join('\n')).toMatch(/upstream-error/);
  });

  it('answers with no choices when the Loom serves a verbatim ontology page', async () => {
    const loom = await fakeLoom(completion(SCAFFOLD_PAGE));
    const lines: string[] = [];
    shim = await startLoomShim({ upstream: loom.url, timeoutMs: 5000, minMaxTokens: 1536, log: (l) => lines.push(l) });
    const json = (await (await post(shim.url, { messages: [] })).json()) as { choices: unknown[] };
    expect(json.choices).toEqual([]);
    expect(lines.join('\n')).toMatch(/scaffold/);
  });

  it('counts each verdict for the receipt', async () => {
    const loom = await fakeLoom(completion('ok\n'));
    shim = await startLoomShim({ upstream: loom.url, timeoutMs: 5000, minMaxTokens: 1536, log: () => {} });
    await (await post(shim.url, { messages: [] })).json();
    await (await post(shim.url, { messages: [] })).json();
    expect(shim.stats()).toMatchObject({ ok: 2, scaffold: 0, truncated: 0, empty: 0, 'upstream-error': 0 });
  });
});
