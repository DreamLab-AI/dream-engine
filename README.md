<div align="center">

<img src="docs/media/hero.jpg" alt="Dream Engine — one honest cycle, every single night" width="100%">

# ☾ Dream Engine

**A config-driven engine for nightly, cloud-scheduled, evidence-gated repository evolution.**

[![CI](https://github.com/DreamLab-AI/dream-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/DreamLab-AI/dream-engine/actions/workflows/ci.yml)
[![CodeQL](https://github.com/DreamLab-AI/dream-engine/actions/workflows/codeql.yml/badge.svg)](https://github.com/DreamLab-AI/dream-engine/actions/workflows/codeql.yml)
[![license](https://img.shields.io/badge/license-MIT-22d3ee)](LICENSE)

**[ADR-0001](docs/adrs/ADR-0001-dream-machine-engine.md)**

</div>

> Freeze the model. Evolve the harness. **Evaluation is not promotion** — the
> machine never merges; a human does.

> **Dream Engine is how the [Dynamic Agentic Mesh](https://github.com/DreamLab-AI/VisionFlow)
> improves itself** — the nightly self-improvement loop any repository in the
> ecosystem can run: form one falsifiable hypothesis, measure it against the
> repo's real evaluators, open a *draft* PR a human merges. It is
> [DreamLab-AI](https://github.com/DreamLab-AI)'s continuation of rUv's
> **[`ruvnet/dream-machine`](https://github.com/ruvnet/dream-machine)**
> ([live walkthrough](https://ruvnet.github.io/dream-machine/)) — derived from it,
> crediting it, and [tracking it upstream](FORK.md). See
> [In the VisionFlow ecosystem](#in-the-visionflow-ecosystem),
> [History & lineage](#history--lineage), and [Origin & credit](#origin--credit).

```bash
npx dream-machine init --repo owner/name --out dream.config.json
npx dream-machine compile dream.config.json --out PROMPT.md
```

---

## What it is

The Dream Engine turns *"run one research-and-evolution cycle against a
repository every night"* into a **config-compiled engine** instead of a
hand-maintained megaprompt. It wakes up in an isolated cloud session, forms one
falsifiable hypothesis, measures it against the repo's real evaluators, and
writes down what it learned — whether or not the answer was the one it hoped
for.

It is the generalization of two routines already running nightly against
[`ruvnet/ruflo`](https://github.com/ruvnet/ruflo) and
[`ruvnet/metaharness`](https://github.com/ruvnet/metaharness). Both are ~800-line
prompts running the same 26-step pipeline; they differ only in a small,
well-defined per-repo delta. The Dream Engine factors the shared spine into an
engine and the differences into a `dream.config`.

## In the VisionFlow ecosystem

Dreaming is the [Dynamic Agentic Mesh](https://github.com/DreamLab-AI/VisionFlow)'s
**nightly self-improvement substrate** — the mesh's own thesis turned reflexive. The
mesh promotes the human from *router* to *judgment broker*: agents observe and
propose; a human holds the decision at the intersections a machine should not close
on its own. Dreaming applies that boundary to the repositories' **own evolution** —
every night an agent proposes, and the merge is a human signature. *Evaluation is not
promotion* is the same rule the [forum](https://github.com/DreamLab-AI/nostr-rust-forum)
enforces for governance, wired into the development loop.

It composes with the rest of the mesh rather than duplicating it:

- **Grounded** — a night's research can query the [Ontology Loom](https://github.com/DreamLab-AI/loom)
  so hypotheses are grounded in the shared, reasoned ontology rather than parametric
  guesswork.
- **Witnessed** — every report is bound to its commit by a reproducible
  double-sha256, the same provenance discipline the mesh uses for its evidence.
- **Ledgered** — the append-only `LEDGER.md` is the honest, dated status record the
  ecosystem canon asks of every substrate.
- **Sovereign-ready** — the estate direction runs it as a container-local
  orchestrator over the [agentbox](https://github.com/DreamLab-AI/agentbox) fleet,
  one repository per cycle.

**Maturity — stated honestly.** The single-repo loop is real and running: it drives
[`DreamLab-AI/VisionFlow`](https://github.com/DreamLab-AI/VisionFlow) itself today
(`dream.config.json`, a link-integrity evaluator, a dated `docs/dream-cycle/LEDGER.md`).
The estate-wide orchestrator, and a `did:nostr` identity so a dream cycle is a
first-class signed mesh actor, are in progress — tracked in the ledger, not claimed here.

## The nightly pipeline

<div align="center"><a href="https://ruvnet.github.io/dream-machine/#pipeline" title="See it animate on the live site"><img src="docs/media/pipeline.jpg" alt="The 13-stage nightly pipeline, rendered as an orbital scrollytelling narrative" width="100%"></a></div>

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

<div align="center"><a href="https://ruvnet.github.io/dream-machine/dashboard.html" title="Open the live management dashboard"><img src="docs/media/dashboard.jpg" alt="The Dream Engine management console" width="100%"></a></div>

## Schedule it — nightly, autonomous, always improving

The Dream Engine is built to run itself on a schedule. In **Claude Code**, the
built-in **`/schedule`** command creates a cloud routine that runs the pipeline
against your repo every night — autonomous research, evaluation, and
improvement that compounds while you sleep.

**1 — generate the routine body from your config:**

```bash
npx dream-machine schedule dream.config.json --env <your-cloud-env-id> --out routine.json
```

**2 — create the routine.** In Claude Code, type `/schedule`, choose a nightly
cron (e.g. `0 9 * * *` UTC) and your target repo, and paste the routine body
from `routine.json` — or, better, paste the tiny **self-hosting prompt** below.

**The prompt to paste into `/schedule` (recommended).** Rather than freeze a full
prompt, point the routine at a bootstrap that compiles tonight's instructions
from your committed `dream.config.json`, so the schedule can never drift from the
repo. This is the exact prompt that runs against this repository every night —
just change the repo slug:

```text
You are the Dream Engine nightly runner for DreamLab-AI/dream-engine, checked out fresh on main.

STEP A — build the engine (a fresh checkout has no dist/):
  npm ci && npm run build || true          # a wasm/NAPI failure is a recorded degradation, not a stop

STEP B — compile tonight's instructions from the committed config:
  npx dream-machine compile dream.config.json --out /tmp/tonight.md
  cat /tmp/tonight.md

STEP C — follow /tmp/tonight.md EXACTLY: the full 26-step pipeline
  (ledger → research → frozen hypothesis → candidate → baseline → evaluation →
   adversarial critique → bounded Darwin → evidence → witness → issue → DRAFT PR → ledger row).

Invariants: end in exactly one of ACCEPT | REJECT | INCONCLUSIVE (INCONCLUSIVE
with LLM_EVAL=blocked when there's no API key is a legitimate, successful night).
Evaluation is not promotion — NEVER merge, NEVER self-promote. Publish a public
gist + a labeled issue + a DRAFT PR, and append exactly one row to
docs/dream-cycle/LEDGER.md every run. Never weaken a test; never force-push.
```

> Prefer to keep the whole prompt inline instead of the bootstrap?
> `npx dream-machine compile dream.config.json` prints the full 26-step routine —
> paste that into `/schedule` directly.

**What each night does** — research SOTA for tonight's rotation surface → freeze a
falsifiable hypothesis → build a concrete candidate → evaluate parent vs.
candidate on your real benchmarks → adversarial critique + reward-hack check →
bounded Darwin evolution → witnessed evidence → gist + issue + **draft** PR → one
ledger row. Win, lose, or draw, it records what it learned so tomorrow's search
space is smaller. **rUv's upstream runs exactly this loop on itself** (cron
`0 9 * * *`) — browse its
[dream-cycle issues](https://github.com/ruvnet/dream-machine/issues?q=label%3Adream-cycle),
gists, and draft PRs to see it in action. In the mesh, the same loop drives
[`DreamLab-AI/VisionFlow`](https://github.com/DreamLab-AI/VisionFlow) today, with the
estate-wide orchestrator following (see [In the VisionFlow ecosystem](#in-the-visionflow-ecosystem)).

> No `dream.config`? `npx dream-machine init --repo owner/name` scaffolds one.
> A night with no API key still runs — it reports `LLM_EVAL=blocked`, an honest
> `INCONCLUSIVE`, rather than faking a result.

### Or: the optional GitHub Actions "dream" (no cloud agent needed)

Prefer to stay entirely inside GitHub? [`.github/workflows/dream-nightly.yml`](.github/workflows/dream-nightly.yml)
runs the **research + hypothesis** half of the pipeline from a plain CI runner
using an [OpenRouter](https://openrouter.ai) model, files a witnessed
`dream-cycle` research issue, and opens a draft PR that appends one ledger row.

```yaml
# add repo secret OPENROUTER_API_KEY (+ optional var OPENROUTER_MODEL),
# then uncomment the schedule in dream-nightly.yml:
schedule:
  - cron: '0 9 * * *'
```

Honest scope: candidate evaluation, bounded Darwin, and the promotion gate need
the agentic `/schedule` session, so this CI path is **research-only** — every
night is an `INCONCLUSIVE` research night, and with no key it degrades to
`LLM_EVAL=blocked` rather than fabricating a finding. It's disabled by default
and also runs on demand via **Run workflow** (with a dry-run option).

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

The Dream Engine runs autonomously, so its guarantees are enforced in code and
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

## History & lineage

The name is older than the tool. M.M. Waldrop's *The Dream Machine* (2001) told the
story of J.C.R. Licklider and the people who made computing personal — the machine
that dreams a better version of itself is not a new idea.

The engine descends from **rUv's [`ruvnet/dream-machine`](https://github.com/ruvnet/dream-machine)**:
a config-driven generalisation of two routines already running nightly against
[`ruvnet/ruflo`](https://github.com/ruvnet/ruflo) and
[`ruvnet/metaharness`](https://github.com/ruvnet/metaharness), and a sibling of rUv's
broader **DREAM AI** work on recursive self-optimisation. Its guardrails answer a
documented failure mode of open-loop autonomous research agents — Sakana AI's "The AI
Scientist" reward-hacking incident, AutoGPT/BabyAGI-era loops with ~5% follow-through.
The promotion gate, adversarial critic, reward-hack check, and human-only merge
boundary exist precisely because of them.

The research thread keeps sharpening the design. **AutoDesign: Meta-Harness
Optimization for Long-Horizon Agentic Design**
([arXiv:2608.13560](https://arxiv.org/abs/2608.13560)) states the same thesis
formally — *freeze the model, evolve the harness* — with an outer loop that reads an
optimisation record and proposes one bounded, gated harness change. That is the
direction of [ADR-DL-001](docs/dream-engine/ADR-DL-001-meta-harness-optimisation.md);
its first step (`ledger signals --propose`) already ships here.

## Origin & credit

Dream Engine began as **[`ruvnet/dream-machine`](https://github.com/ruvnet/dream-machine)**
by [rUv](https://github.com/ruvnet) — a config-driven engine for nightly,
evidence-gated repository evolution. All the foundational design (the
compile→schedule→witness pipeline, the promotion gate, the adversarial critic,
the human-only merge boundary) is his.

[DreamLab-AI](https://github.com/DreamLab-AI) forked it to take the engine in an
internal direction that intersects the **agentbox** agent estate — sovereign
mutations via the Loom, the Rust dream-engine acceptance path, estate-specific
evaluators, and an orchestrator that runs the loop across the whole mesh.

**Why the internal name is *Dream Engine*.** In the ecosystem's own vocabulary,
"dreaming" is the self-improvement substrate and the Rust acceptance path is the
"dream engine" — so the fork's front door adopts that name to place it inside the
mesh and to distinguish the ecosystem's estate-wide loop from rUv's upstream tool.
The divergence is real (multi-repo orchestration, `did:nostr` provenance,
Loom-grounded research), but the rename is deliberately skin-deep: it lives at
[`DreamLab-AI/dream-engine`](https://github.com/DreamLab-AI/dream-engine) and
**tracks upstream** — rUv's ongoing development is pulled in via the `upstream`
remote (`git fetch upstream && git merge upstream/main`); we do not push branches or
PRs back. To keep those merges clean, the fork keeps rUv's naming
(`dream-machine`, `@dream-machine/*`, `dream.config.json`, the compiled routine
prompt) byte-for-byte — only this README's branding reads *Dream Engine*. The full
rationale and sync recipe live in [FORK.md](FORK.md).

## License

MIT — original work © [rUv](https://github.com/ruvnet) `<ruv@ruv.net>`;
fork modifications © DreamLab-AI contributors. See [LICENSE](LICENSE).
