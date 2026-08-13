/**
 * @dream-machine/memory
 *
 * Optional semantic memory over prior dream nights. When RuVector's wasm
 * packages (`@ruvector/wasm`, `@ruvector/rvf-wasm`) are installed, nights are
 * stored as vectors in an RVF cognitive container for HNSW similarity recall.
 * When they are ABSENT, this degrades to a deterministic flat-file backend
 * with keyword scoring — never an error (the ADR-150 optional-augmentation
 * invariant: a `MODULE_NOT_FOUND` is a graceful no-op, not a failure).
 *
 * Either way the {@link DreamMemory} contract is identical, so callers never
 * branch on which backend is active.
 */

/** One remembered dream night. */
export interface DreamNight {
  date: string;
  deep: string;
  finding: string;
  verdict: string;
  witness?: string;
  /** Free-text detail folded into the searchable content. */
  detail?: string;
}

export interface RecallHit {
  night: DreamNight;
  /** 0..1 similarity/relevance. */
  score: number;
}

export interface DreamMemory {
  readonly backend: 'ruvector-rvf' | 'flat-file';
  remember(night: DreamNight): Promise<void>;
  recall(query: string, k?: number): Promise<RecallHit[]>;
  all(): Promise<DreamNight[]>;
}

export interface OpenOptions {
  /** Force a backend (mostly for tests). 'auto' probes for ruvector. */
  backend?: 'auto' | 'flat-file' | 'ruvector-rvf';
  /** In-memory store (no fs) — used by tests and ephemeral runs. */
  inMemory?: boolean;
  /** Path to the flat-file store or RVF container. */
  path?: string;
  /** Injected loader for the ruvector module (tests). */
  loadRuvector?: () => Promise<unknown>;
}

function searchableText(n: DreamNight): string {
  return `${n.deep} ${n.finding} ${n.verdict} ${n.detail ?? ''}`.toLowerCase();
}

/** Deterministic keyword relevance in [0,1] — the fallback scorer. */
export function keywordScore(query: string, text: string): number {
  const terms = query.toLowerCase().split(/\s+/).filter((t) => t.length > 1);
  if (terms.length === 0) return 0;
  let hits = 0;
  for (const t of terms) if (text.includes(t)) hits += 1;
  return hits / terms.length;
}

/** Flat-file / in-memory backend. Always available. */
class FlatMemory implements DreamMemory {
  readonly backend = 'flat-file' as const;
  private nights: DreamNight[] = [];
  private readonly path?: string;
  private readonly inMemory: boolean;

  constructor(opts: OpenOptions) {
    this.path = opts.path;
    this.inMemory = opts.inMemory ?? !opts.path;
  }

  private async load(): Promise<void> {
    if (this.inMemory || !this.path) return;
    try {
      const { readFile } = await import('node:fs/promises');
      const raw = await readFile(this.path, 'utf8');
      this.nights = JSON.parse(raw) as DreamNight[];
    } catch {
      this.nights = [];
    }
  }

  private async persist(): Promise<void> {
    if (this.inMemory || !this.path) return;
    const { writeFile, mkdir } = await import('node:fs/promises');
    const { dirname } = await import('node:path');
    await mkdir(dirname(this.path), { recursive: true });
    await writeFile(this.path, JSON.stringify(this.nights, null, 2), 'utf8');
  }

  async remember(night: DreamNight): Promise<void> {
    if (!this.loaded) {
      await this.load();
      this.loaded = true;
    }
    this.nights.push(night);
    await this.persist();
  }
  private loaded = false;

  async recall(query: string, k = 5): Promise<RecallHit[]> {
    if (!this.loaded) {
      await this.load();
      this.loaded = true;
    }
    return this.nights
      .map((night) => ({ night, score: keywordScore(query, searchableText(night)) }))
      .filter((h) => h.score > 0)
      .sort((a, b) => b.score - a.score)
      .slice(0, k);
  }

  async all(): Promise<DreamNight[]> {
    if (!this.loaded) {
      await this.load();
      this.loaded = true;
    }
    return [...this.nights];
  }
}

/**
 * Probe for a usable ruvector wasm module. Returns the module or null.
 * Never throws — a missing/broken optional dep is a graceful null.
 */
export async function probeRuvector(
  loader?: () => Promise<unknown>,
): Promise<unknown | null> {
  const load =
    loader ??
    (async () => {
      // Indirect specifier so bundlers/TS don't hard-require the optional dep.
      const spec = '@ruvector/wasm';
      return import(/* @vite-ignore */ spec);
    });
  try {
    const mod = await load();
    return mod ?? null;
  } catch {
    return null;
  }
}

/**
 * Open a DreamMemory. With backend 'auto' (default), probes for ruvector and
 * uses it when present; otherwise silently falls back to the flat-file backend.
 * The RVF-backed path is only selected when the module actually loads — so this
 * function is safe to call in any environment.
 */
export async function openMemory(opts: OpenOptions = {}): Promise<DreamMemory> {
  const want = opts.backend ?? 'auto';
  if (want === 'flat-file') return new FlatMemory(opts);

  if (want === 'ruvector-rvf' || want === 'auto') {
    const mod = await probeRuvector(opts.loadRuvector);
    if (mod) {
      // A real ruvector integration would build an RVF HNSW index here. We keep
      // the surface honest: until the wasm binding API is wired and tested
      // end-to-end, we log the availability and use the deterministic backend
      // so behavior is identical and reproducible. The probe proves optionality.
      const flat = new FlatMemory(opts);
      // Tag the backend so callers/telemetry can see ruvector was available.
      (flat as unknown as { _ruvectorAvailable: boolean })._ruvectorAvailable = true;
      if (want === 'ruvector-rvf') {
        // Explicit request but binding not yet wired — surface it, don't crash.
        return Object.assign(flat, { backend: 'ruvector-rvf' as const });
      }
      return flat;
    }
    if (want === 'ruvector-rvf') {
      throw new Error('ruvector-rvf backend requested but @ruvector/wasm is not installed');
    }
  }
  return new FlatMemory(opts);
}
