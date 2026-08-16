# This is a fork

**`DreamLab-AI/dream-engine`** is DreamLab-AI's fork of
**[`ruvnet/dream-machine`](https://github.com/ruvnet/dream-machine)** by
[rUv](https://github.com/ruvnet). All the foundational design is his; see
[README → Origin & credit](README.md#origin--credit).

## Why the fork exists

We take the engine in an internal direction that intersects the **agentbox**
agent estate — Loom-driven sovereign mutations, the Rust dream-engine acceptance
path, and estate-specific evaluators. That work lives here, not upstream.

## How to tell the two apart

The distinction is deliberately **concentrated**, not spread through the code:

| Signal | `ruvnet/dream-machine` | `DreamLab-AI/dream-engine` (this fork) |
|--------|------------------------|----------------------------------------|
| GitHub repo | `ruvnet/dream-machine` (public) | `DreamLab-AI/dream-engine` (private) |
| README front door | "Dream Machine" | "Dream Engine" + this notice |
| Marker file | — | this `FORK.md` |
| Everything else (packages, CLI, `dream.config.json`, compiled prompt, docs, site) | — | **byte-identical to upstream** |

We intentionally did **not** do a full internal rebrand. Renaming the package
identifiers (`dream-machine`, `@dream-machine/*`), the `dream.config.json`
filename, the CLI surface, or the compiled routine prompt would turn every
upstream change to those files into a merge conflict — so they stay exactly as
rUv ships them.

## Tracking upstream

The `upstream` remote points at rUv's repo (its push URL is disabled so nothing
can be pushed back by accident):

```bash
git remote -v
# origin    https://github.com/DreamLab-AI/dream-engine.git (fetch/push)
# upstream  https://github.com/ruvnet/dream-machine.git    (fetch)
# upstream  DISABLED_no_push_to_ruv                        (push)
```

To pull rUv's ongoing development:

```bash
git fetch upstream
git merge upstream/main          # or: git rebase upstream/main
```

Merges should be near-clean because the fork keeps upstream's naming. The one
file expected to conflict is `README.md` — always resolve it in favour of the
Dream Engine front door (keep ours for the identity block, take theirs for
substantive content changes).

## What does **not** go back to rUv

Nothing is pushed upstream — no branches, no PRs. Our dreaming branches
(`dream/*`) and any fork-specific work stay in `DreamLab-AI/dream-engine`. The
relationship is one-way: we consume upstream, we do not contribute back through
git (any contribution to rUv would be a separate, deliberate PR from a clean
branch).
