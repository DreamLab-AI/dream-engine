# ADR-DL-001: Meta-harness optimisation — closing the Dream Engine's outer loop

- **Status:** Proposed (fork-specific; not upstream)
- **Date:** 2026-08-16
- **Owner:** DreamLab-AI (Dream Engine fork)
- **Source:** *AutoDesign: Meta-Harness Optimization for Long-Horizon Agentic
  Design* — Luo et al., [arXiv:2608.13560](https://arxiv.org/abs/2608.13560)
  (cs.CV/cs.AI/cs.CL, Aug 2026)

> This is a fork-only design record. It lives under `docs/dream-engine/` (not
> `docs/adrs/`) so it never touches the upstream ADR set or its index, keeping
> `git merge upstream/main` clean. See [FORK.md](../../FORK.md).

## Context

The Dream Engine freezes the model and evolves the harness: every night it forms
one falsifiable hypothesis about a rotating surface, measures a candidate against
the repo's real evaluators, and records the outcome in `LEDGER.md`. That is an
*inner* loop — it improves one artifact per night under a fixed harness (the
compiled routine prompt, `dream.config.json`, the evaluators, the CLI
orchestration).

AutoDesign attacks the same problem statement — "freeze the model, improve the
scaffolding" — from a poster-design domain, but its algorithm is domain-agnostic
and worth adopting on its merits. It formalises an *outer* loop that the Dream
Engine currently only gestures at.

## What the paper actually does

Two nested loops around a frozen model π_θ:

- **Inner loop (per artifact).** A designer and a critic iterate:
  `y_k = M_design(y_{k-1}, f_{k-1}; x, c)`, `f_k = M_critic(y_k; x, c)` — the
  critic returns *localised* diagnostics that drive the next edit (Eq. 4).
- **Outer loop (Algorithm 1, per harness).** Improve the harness `H` itself:
  1. **Rollout** — run `H_t` over a training set, collect trajectories `τ_t` and
     scores `s_t` (and the same over a held-out dev set).
  2. **Update proposal** — a *coding-agent optimiser* `P(H_t, τ_t, s_t, ℒ)`
     synthesises failure patterns from the trajectories and proposes one
     candidate `H'_{t+1}`.
  3. **Acceptance gate** (Eq. 6) — accept iff training improves **and** dev does
     not regress: `J_train(H') > J_train(H) ∧ J_dev(H') ≥ J_dev(H)`.
  4. **Persist** — append the checkpoint, scores, proposal and decision to an
     **optimisation record `ℒ`** (enables comparison and rollback, no tree
     search).

Four load-bearing safeguards:

- **Frozen optimisation-time evaluator `R_meta`**, constructed once from human
  references and held fixed during the loop, kept **separate** from the external
  benchmark used for final scoring — an explicit anti-reward-hacking split.
- **Held-out dev set never shown to `P`** — an overfitting guard.
- **Exactly one of the harness's components changes per outer iteration**, so
  credit assignment stays interpretable.
- **Human-in-the-loop escape `g_t`** when the search plateaus.

The harness is defined as five components: context/memory, tools/specs,
execution runtime, orchestration, and evaluation/feedback.

## Mapping to the Dream Engine

The correspondence is close enough to be useful as a design mirror:

| AutoDesign | Dream Engine today | Verdict |
|---|---|---|
| Freeze π_θ, evolve harness `H` | "Freeze the model. Evolve the harness." | **Same thesis** |
| Harness = 5 components | `dream.config.json` + compiled prompt + evaluators + CLI orchestration | Present, but **not named as components** |
| Inner loop (designer + critic) | nightly candidate → adversarial critique → bounded Darwin | **Present** |
| Optimisation record `ℒ` | `LEDGER.md` (+ `learningSignals`) | **Present** — the ledger *is* `ℒ` |
| One component per iteration | per-night DEEP-surface rotation | **Present** — validates the rotation |
| `R_meta` frozen, separate from benchmark | "evaluation is not promotion" + reward-hack check + human-only merge | **Present in spirit** |
| Acceptance gate: train↑ ∧ dev not↓ | ACCEPT/REJECT verdict + promotion gate | **Partial** — no held-out dev guard |
| Outer-loop optimiser `P` reads `ℒ`, proposes bounded harness edit | — | **Missing** |
| Held-out dev set overfitting guard | golden snapshots + tests | **Partial** — no held-out *config* set |
| Ledger-driven rollback | witnessed commits; human revert | **Partial** — no signal-driven rollback |

The headline: the Dream Engine already has the record, the credit-assignment
discipline, and the inner loop. It is missing the **outer optimiser** and the
**held-out guard** that make the paper's improvements compound and stay honest.

## Decision / proposal

Adopt the outer loop as a first-class, bounded subsystem. Concretely, in
priority order:

1. **`learningSignals` → a harness-change proposer.** `learningSignals` already
   distils the ledger into signals (`zeroMergeStreak`, duplicate directions,
   etc.) — that is precisely "synthesise failure patterns from trajectories".
   Add a step that turns those signals into at most **one** proposed edit to a
   single named harness component (the AutoDesign restriction), surfaced as a
   draft — never auto-applied. This is the `P` step, scoped to human-gated draft
   output to preserve "the machine never merges".
2. **Name the five harness components explicitly** in `dream.config.json` and
   bind the nightly DEEP rotation to them, so credit assignment is as
   interpretable as the paper's (and so a proposal can say *which* component it
   touches).
3. **A held-out dev guard.** Promote the self-hosted golden snapshot idea (the
   `compiler-parity` test we just integrated) into a small held-out set of
   configs whose compiled output a harness change must not regress — the Dream
   Engine's `J_dev` guard. Cheap, deterministic, and already half-built.
4. **Ledger-driven rollback.** When later nights show a harness change was
   regressive (the ledger `ℒ` supports exactly this comparison), emit a rollback
   proposal rather than only forward hypotheses.

## Consequences

- **Additive and safe.** Every step is a draft/proposal gated by the existing
  human-merge boundary; none weakens "evaluation is not promotion".
- **Fork-local first.** This is a Dream Engine direction, not an upstream ask.
  Keep the work under fork-namespaced paths where it does not fight
  `merge upstream/main`; only touch shared engine files when a change is
  genuinely general and worth offering upstream deliberately.
- **Reward-hacking discipline is reaffirmed**, not replaced: keep the
  optimisation-time evaluator distinct from any final benchmark, exactly as the
  paper insists and as the promotion gate already implies.

## Recommended first step

Ship (2) then (1): name the harness components, then extend `ledger signals`
with an opt-in `--propose` that prints a single bounded, component-scoped harness
suggestion derived from the signals. That is one night's worth of work, reuses
`learningSignals`, and produces a reviewable draft — the smallest real instance
of the outer loop.

## References

- Luo et al., *AutoDesign: Meta-Harness Optimization for Long-Horizon Agentic
  Design*, arXiv:2608.13560, 2026.
- [ADR-0001](../adrs/ADR-0001-dream-machine-engine.md) — the engine (inner loop,
  promotion gate, human-only merge).
- [FORK.md](../../FORK.md) — fork identity and upstream-tracking policy.
