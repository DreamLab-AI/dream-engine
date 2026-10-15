<div align="center">

# dream-engine

### A nightly, evidence-gated loop that lets an agent propose changes to a repository and a gate decide

[![Licence](https://img.shields.io/badge/Licence-MIT%20OR%20Apache--2.0-blue?style=flat-square)](#licence)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-orange?style=flat-square)](Cargo.toml)
[![Status](https://img.shields.io/badge/status-pre--release%3B%20two%20crates%20built%2C%20two%20planned-yellow?style=flat-square)](docs/adr/README.md)

*The model proposes. A pure function of measured evidence disposes. A human merges.*

</div>

---

## What is dream-engine?

dream-engine runs a repository's own checks against a proposed change and
decides, from the numbers alone, whether the change is better, worse, unproven,
or untestable. Once a night it freezes an experiment, asks a model for one
patch, evaluates the baseline and the patched tree on the same testbed, and
hands the two result sets to a gate. The gate is a pure function: the same
evidence always gives the same verdict, and it calls no model. An accepted
change opens as a draft pull request. It is never merged by the engine unless
you have switched that on ([ADR-DE-006](docs/adr/ADR-DE-006-automerge-is-an-option.md)).

The design is deliberately cautious about what counts as evidence. An
evaluator that is missing, silent, blocked, timed out or failing vetoes the
change. A broken testbed is reported as `BLOCKED-ENV`, which is a different
verdict from a bad patch, so infrastructure trouble is never read as a verdict
on the model. A candidate may not touch the files that decide whether it
passes ([ADR-DE-003](docs/adr/ADR-DE-003-protected-path-guard.md)).

## The loop

The engine freezes an experiment manifest (baseline revision, tree hash, the
sha256 of each evaluator command, model identity and a digest of the config)
atomically before any model call. The run id is a pure function of that
manifest, so a restart resumes the same experiment, and a baseline that has
diverged is archived rather than overwritten. It then evaluates the baseline,
calls the model once, applies the patch, evaluates the candidate on the same
testbed, and gates the pair. A run moves through `ManifestFrozen`,
`BaselineEvaluated`, `ModelCalled`, `CandidateEvaluated` and `Gated`. The gate
returns `ACCEPT`, `REJECT`, `INCONCLUSIVE` or `BLOCKED-ENV`. The full account
is in [docs/concepts.md](docs/concepts.md).

## Crates

| Crate | What it does | Status |
|---|---|---|
| [`pairwise-gate`](crates/pairwise-gate) | Selection policies over paired candidate and baseline results: `Margin`, `Floor`, `SignTest`, `WilsonLower`, `NoRegression`, `TrainDev`. Faults never count as losses. | In the repository, `0.1.0`; not yet published |
| [`failure-fingerprint`](crates/failure-fingerprint) | A 16-hex-character SHA-256 identity for a failure mode, and a `New` / `Persisting` / `Fixed` classification across steps. | In the repository, `0.1.0`; not yet published |
| `dream-gate` | The protected-path guard, config-digest binding and the veto classes that turn evaluator evidence into a verdict. | Planned |
| [`dream-core`](crates/dream-core) | Evaluator runners, the candidate worktree with guard-before-apply ordering, the ledger and its signals, harness-stall detection, replay, source reading, and the seams (`Llm`, `MemorySink`, `Notifier`, `Forge`). The nightly driver that sequences them is the host's. | In the repository, `0.1.0`; not yet published |

The two built crates are small and independent of the rest, and each has its
own README. The statistics are checked against reference values computed
independently in exact arithmetic: nine wins out of ten gives an exact
one-sided binomial p of 11/1024, and a Wilson lower bound for eight of ten at
z = 1.96 is 0.4902.

## Quickstart

The two built crates can be used from a checkout today:

```sh
git clone https://github.com/DreamLab-AI/dream-engine
cd dream-engine
cargo test --workspace --locked
```

The driver and its command line (planned, not yet in the repository):

```sh
# planned
cargo install dream-core
dream init          # write dream.config.json for the current repository
dream run --dry-run # freeze the manifest and evaluate the baseline, no model call
dream run           # one nightly cycle
```

Configuration for a Rust repository is in
[docs/guide/configuring-a-repo.md](docs/guide/configuring-a-repo.md).

## Documentation

[docs/README.md](docs/README.md) is the index. The decisions are in
[docs/adr/](docs/adr/README.md). If you are coming from the TypeScript fork,
start with [the migration note](docs/guide/migrating-from-the-ts-fork.md). A
comparison with a related system is in [docs/COMPARISON.md](docs/COMPARISON.md).

## Origin and credit

The nightly, evidence-gated, config-driven design descends from
[ruvnet/dream-machine](https://github.com/ruvnet/dream-machine) by rUv. This
repository is an independent Rust implementation of that design lineage. No
source text from that project is copied. Its MIT licence is reproduced in
[NOTICE](NOTICE) as credit and as a precaution.

## Licence

`MIT OR Apache-2.0`, at your option: [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE). The adapter that connects the engine to a
private forum and Nostr deployment is AGPL-3.0-only, lives in the product that
consumes the engine, and is not part of this repository
([ADR-DE-002](docs/adr/ADR-DE-002-licence-and-provenance.md)).

Diagram corpus (citation-verified, with a register of tensions and debt): [docs/diagrams](docs/diagrams/README.md).
