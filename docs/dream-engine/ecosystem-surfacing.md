# Ecosystem surfacing kit — promoting Dreaming across the mesh

Ready-to-apply copy for surfacing **dreaming as the mesh's nightly self-improvement
loop** across the VisionFlow ecosystem. Staged here (a doc in this fork) so nothing
public changes until three decisions are made — see [Pending decisions](#pending-decisions).

Written to the ecosystem's house discipline (memory `visionflow-site-ontology-messaging-2026-07-30`):
**name it once per surface, in that surface's own voice, honest and dated.** Dreaming
already runs on VisionFlow (`dream.config.json`, `scripts/dream-link-check.sh` as a
dream-cycle evaluator, `docs/dream-cycle/LEDGER.md`), so this narrates a deployed
capability — it does not overclaim.

## Pending decisions

1. **Visibility of `DreamLab-AI/dream-engine`.** It is currently **private**. Public
   READMEs/marketing can only link it with working URLs if it is public. Two variants
   of every block below are provided: **[public-link]** (once dream-engine is public)
   and **[upstream-link]** (link rUv's public `ruvnet/dream-machine` instead; safe now).
2. **Breadth.** Keystone (VisionFlow + marketing) · +hosts (agentbox) · full weave
   (all substrate READMEs).
3. **Mechanism.** PRs per repo (reviewable) vs push to main (the 2026-07-30 house pattern).

---

## 1. VisionFlow — keystone (the canon)

Insert as a short subsection after **The ecosystem** table in `README.md`.

> **Self-improvement: the nightly dream.** The mesh improves itself the way it governs
> — agents propose, a human signs. Every repository here can run a nightly **dream
> cycle**: form one falsifiable hypothesis against tonight's rotation surface, measure
> it on the repo's real evaluators, and open a *draft* PR a human merges. It is the
> judgment-broker boundary wired into development — *evaluation is not promotion*. The
> loop runs on this repository today (`dream.config.json`, a link-integrity evaluator,
> a dated `docs/dream-cycle/LEDGER.md`); the estate-wide orchestrator over the
> [agentbox](https://github.com/DreamLab-AI/agentbox) fleet, and a `did:nostr` identity
> per cycle, are in progress. Engine: **[dream-engine]** — DreamLab's tracking fork of
> rUv's [`ruvnet/dream-machine`](https://github.com/ruvnet/dream-machine), following the
> design formalised in *AutoDesign: Meta-Harness Optimization* ([arXiv:2608.13560](https://arxiv.org/abs/2608.13560)):
> *freeze the model, evolve the harness.*

- **[public-link]** `**[dream-engine]**` → `[dream-engine](https://github.com/DreamLab-AI/dream-engine)`
- **[upstream-link]** drop "**[dream-engine]** — DreamLab's tracking fork of rUv's"
  and read "Engine: rUv's [`ruvnet/dream-machine`](https://github.com/ruvnet/dream-machine),
  run as a DreamLab tracking fork".

Optionally add a badge near the substrate badges:
`[![Self-improvement](https://img.shields.io/badge/self--improvement-nightly%20dream%20loop-8b5cf6?style=flat-square)](#self-improvement-the-nightly-dream)`

## 2. Marketing site (visionflow.info / dreamlab)

A panel in `website/static/index.html` (per memory `visionflow-site-loom-panels-2026-08-11`,
the substrate grid + repo cards live there; `dist/` is CI-rebuilt). Copy:

> **Dreaming — nightly self-improvement for every repo.** The mesh evolves itself under
> the same rule it governs by: an agent proposes, a human signs the merge. Each night a
> repository forms one falsifiable hypothesis, measures it against its own evaluators,
> and opens a draft PR — evidence-gated, witnessed, never self-merged. *Evaluation is
> not promotion.*

Keep any existing substrate-count heading honest: dreaming is a **cross-cutting loop**,
not a seventh substrate — do not bump "Six Substrates" for it. Surface it as a process/
capability panel.

## 3. Per-substrate one-liners (full weave)

One line each, woven into that repo's existing ecosystem/mesh section, in its voice.
Use **[public-link]** `[dream cycle](https://github.com/DreamLab-AI/dream-engine)` or
**[upstream-link]** `[dream cycle](https://github.com/ruvnet/dream-machine)`.

- **agentbox** — *Reproduce, audit, control.* (the orchestrator host)
  > **Self-improvement.** The fleet dreams: a container-local orchestrator runs a nightly
  > [dream cycle](https://github.com/DreamLab-AI/dream-engine) per repository — evidence-gated
  > improvements proposed as draft PRs a human merges, witnessed and ledgered. Reproduce the
  > run, audit the evidence, control the merge.

- **VisionClaw** — *Watch here, judge there.*
  > **Self-improvement.** The same watch-here/judge-there boundary runs on the engine's own
  > code: a nightly [dream cycle](https://github.com/DreamLab-AI/dream-engine) proposes
  > evidence-gated changes as draft PRs — it observes and proposes, it never signs the merge.

- **loom** — *The stable door; models swap behind it.*
  > **Self-improvement.** Dreaming grounds its nightly research through this door: a
  > [dream cycle](https://github.com/DreamLab-AI/dream-engine) can query the reasoned ontology
  > so hypotheses restate checked facts, not parametric guesses — then opens a draft PR a
  > human merges.

- **knowledgeGraph** — the corpus/ontology.
  > **Self-improvement.** The pipeline is dream-able: a nightly
  > [dream cycle](https://github.com/DreamLab-AI/dream-engine) can propose evidence-gated
  > improvements to the corpus build and method as draft PRs a human merges — validated
  > against the same 0-errors/0-warnings gate.

- **solid-pod-rs** — *The exit right sits in the floor.*
  > **Self-improvement.** Even the sovereignty layer evolves under a human signature: a nightly
  > [dream cycle](https://github.com/DreamLab-AI/dream-engine) proposes evidence-gated changes
  > as draft PRs — the merge is never the machine's to make.

- **nostr-rust-forum** — *The one place a decision gets signed.*
  > **Self-improvement.** The forum's own rule — a human decision is the one that's signed —
  > is exactly how the code evolves: a nightly [dream cycle](https://github.com/DreamLab-AI/dream-engine)
  > proposes, a maintainer signs the merge. *Evaluation is not promotion.*

- **dreamlab-ai-website** — *The commercial face.*
  > **Self-improvement.** Behind the front door, the whole estate improves itself nightly — a
  > [dream cycle](https://github.com/DreamLab-AI/dream-engine) per repo, evidence-gated, human-merged.

## Honesty guardrails (do not cross)

- Dreaming runs on **VisionFlow today**; the **estate-wide orchestrator** and the
  **`did:nostr` per-cycle identity** are **in progress** — say so wherever the loop is
  described as ecosystem-wide.
- It is a **cross-cutting loop**, not a new substrate — do not change substrate counts.
- Keep the synthetic-corpus and "not yet shipped" caveats already on each surface intact.
- Credit rUv on every surface that names the engine.

## Rollout checklist (execute on decisions)

- [ ] Decide visibility → pick [public-link] or [upstream-link] variant throughout
- [ ] VisionFlow README keystone subsection
- [ ] Marketing site panel (+ CI redeploy)
- [ ] agentbox one-liner
- [ ] VisionClaw · loom · knowledgeGraph · solid-pod-rs · nostr-rust-forum · dreamlab-ai-website one-liners
- [ ] Mechanism: open PRs or push to main per decision
