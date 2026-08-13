<div align="center">

<img src="docs/media/hero.jpg" alt="Dream Machine — one honest cycle, every single night" width="100%">

# ☾ Dream Machine

**A config-driven engine for nightly, cloud-scheduled, evidence-gated repository evolution.**

[![CI](https://github.com/ruvnet/dream-machine/actions/workflows/ci.yml/badge.svg)](https://github.com/ruvnet/dream-machine/actions/workflows/ci.yml)
[![CodeQL](https://github.com/ruvnet/dream-machine/actions/workflows/codeql.yml/badge.svg)](https://github.com/ruvnet/dream-machine/actions/workflows/codeql.yml)
[![npm](https://img.shields.io/npm/v/dream-machine?color=8b5cf6)](https://www.npmjs.com/package/dream-machine)
[![license](https://img.shields.io/badge/license-MIT-22d3ee)](LICENSE)
[![docs](https://img.shields.io/badge/docs-ruvnet.github.io%2Fdream--machine-e879f9)](https://ruvnet.github.io/dream-machine/)

**[Website & walkthrough →](https://ruvnet.github.io/dream-machine/)** · **[Dashboard](https://ruvnet.github.io/dream-machine/dashboard.html)** · **[Tutorials](https://ruvnet.github.io/dream-machine/tutorials.html)** · **[ADR-0001](docs/adrs/ADR-0001-dream-machine-engine.md)**

</div>

> Freeze the model. Evolve the harness. **Evaluation is not promotion** — the
> machine never merges; a human does.

```bash
npx dream-machine init --repo owner/name --out dream.config.json
npx dream-machine compile dream.config.json --out PROMPT.md
```

---

## What it is

The Dream Machine turns *"run one research-and-evolution cycle against a
repository every night"* into a **config-compiled engine** instead of a
hand-maintained megaprompt. It wakes up in an isolated cloud session, forms one
falsifiable hypothesis, measures it against the repo's real evaluators, and
writes down what it learned — whether or not the answer was the one it hoped
for.

It is the generalization of two routines already running nightly against
[`ruvnet/ruflo`](https://github.com/ruvnet/ruflo) and
[`ruvnet/metaharness`](https://github.com/ruvnet/metaharness). Both are ~800-line
prompts running the same 26-step pipeline; they differ only in a small,
well-defined per-repo delta. The Dream Machine factors the shared spine into an
engine and the differences into a `dream.config`.

## The nightly pipeline

<div align="center"><img src="docs/media/pipeline.jpg" alt="The 13-stage nightly pipeline, rendered as an orbital scrollytelling narrative" width="100%"></div>

```
ledger → research → frozen hypothesis → concrete candidate → baseline
  → evaluation → adversarial critique → bounded Darwin evolution
  → flywheel evidence → witness → issue → draft PR → durable ledger row
```

Every night ends in exactly one verdict — **`ACCEPT`**, **`REJECT`**, or
**`INCONCLUSIVE`** — never a fourth, never silence. A rejected hypothesis with a
clean measurement is a *successful* night. The system optimizes for shrinking
tomorrow's search space, not for producing PRs.

## Install & CLI

```bash
npm i -g dream-machine       # or use npx

dream-machine init --repo owner/name --out dream.config.json   # scaffold a config
dream-machine compile dream.config.json --out PROMPT.md        # config → routine prompt
dream-machine schedule dream.config.json --out routine.json    # cloud /schedule body
dream-machine ledger verify  --path docs/dream-cycle/LEDGER.md # structural checks
dream-machine ledger signals --path docs/dream-cycle/LEDGER.md # STEP 1.1 learning signals
dream-machine witness stamp  report.md <commit>                # provenance stamp
dream-machine witness verify report.md <commit> <witness>      # 5-step verify
dream-machine tui --path docs/dream-cycle/LEDGER.md            # the dashboard, below
```

### The TUI

<div align="center"><img src="docs/media/tui.svg" alt="dream-machine tui — the nightly dashboard" width="720"></div>

### The management console

A browser dashboard renders the ledger as recent nights, verdict distribution,
and per-night evidence — the same data the TUI shows.

<div align="center"><img src="docs/media/dashboard.jpg" alt="The Dream Machine management console" width="100%"></div>

## What it composes (never reimplements)

The heavy stages delegate to the ruvnet stack as **optional, config-selected
backends** — a night without any of them is a *degraded* night, not a failed one.

| Capability | Package | Used for |
|---|---|---|
| Promotion gate + receipts + replay | [`@metaharness/flywheel`](https://www.npmjs.com/package/@metaharness/flywheel) | evidence retention + promotion gate |
| Bounded evolution | [`@metaharness/darwin`](https://www.npmjs.com/package/@metaharness/darwin) | the fenced Darwin stage |
| Adversarial red/blue | [`@metaharness/redblue`](https://www.npmjs.com/package/@metaharness/redblue) | adversarial critic + reward-hack scan |
| Security scan / genome / audit | [`metaharness`](https://www.npmjs.com/package/metaharness) CLI | security review + discovery |
| Vector memory over prior nights | [`@ruvector/wasm`](https://www.npmjs.com/package/@ruvector/wasm) · [`@ruvector/rvf-wasm`](https://www.npmjs.com/package/@ruvector/rvf-wasm) | optional semantic recall (RVF container) |

## Packages

| Package | What it does |
|---|---|
| [`dream-machine`](packages/cli) | the CLI (`init/compile/schedule/ledger/witness/tui`) + TUI |
| [`@dream-machine/compile`](packages/compile) | `dream.config` → the full routine prompt (deterministic) |
| [`@dream-machine/ledger`](packages/ledger) | the 10-column `LEDGER.md` toolkit + learning signals |
| [`@dream-machine/witness`](packages/witness) | `sha256(sha256(report) + commit)` stamp / verify |
| [`@dream-machine/schedule`](packages/schedule) | the cloud `/schedule` routine body emitter |
| [`@dream-machine/memory`](packages/memory) | optional ruvector/RVF semantic memory, flat-file fallback |

## Safety

The Dream Machine runs autonomously, so its guarantees are enforced in code and
CI, not just documented (see [SECURITY.md](SECURITY.md) and
[ADR-0001](docs/adrs/ADR-0001-dream-machine-engine.md)):

- **Evaluation is not promotion.** The session never merges and never
  self-promotes; it opens *draft* PRs only.
- **Guarded auto-merge.** The optional auto-merge job refuses any PR touching a
  protected path (gates, safety, thresholds, CI, dependency manifests) and
  requires an explicit label plus green required checks. The session never runs
  the merge itself.
- **Optional deps stay optional** (ADR-150) — a CI job proves the engine builds
  and tests with the ruvector wasm backends absent.
- **Witnessed provenance** — every report is bound to its commit by a
  reproducible double-sha256 anyone can re-derive.

## Prior art

The design answers a known failure mode of open-loop autonomous research agents
(Sakana AI's "The AI Scientist" reward-hacking incident; AutoGPT/BabyAGI-era
loops with ~5% follow-through). The promotion gate, adversarial critic,
reward-hack check, and human-only merge boundary exist precisely because of them.

## License

MIT © [rUv](https://github.com/ruvnet)
