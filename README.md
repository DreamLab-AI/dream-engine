<div align="center">

# dream-engine

### A nightly, evidence-gated loop that lets an agent propose changes to a repository and a gate decide

[![Licence](https://img.shields.io/badge/Licence-MIT%20OR%20Apache--2.0-blue?style=flat-square)](#licence)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-orange?style=flat-square)](Cargo.toml)
[![crates.io](https://img.shields.io/crates/v/pairwise-gate?style=flat-square&label=pairwise-gate)](https://crates.io/crates/pairwise-gate)
[![crates.io](https://img.shields.io/crates/v/failure-fingerprint?style=flat-square&label=failure-fingerprint)](https://crates.io/crates/failure-fingerprint)
[![Status](https://img.shields.io/badge/status-pre--1.0-yellow?style=flat-square)](docs/adr/README.md)

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

## How a night runs

The engine freezes an experiment manifest (baseline revision, tree hash, the
sha256 of each evaluator command, model identity and a digest of the config)
atomically before any model call. The run id is a pure function of that
manifest, so a restart resumes the same experiment, and a baseline that has
diverged is archived rather than overwritten. It then evaluates the baseline,
calls the model once, applies the patch, evaluates the candidate on the same
testbed, and gates the pair. A run moves through `ManifestFrozen`,
`BaselineEvaluated`, `ModelCalled`, `CandidateEvaluated`, `Gated`, `Persisted` and `Complete`. A finished night is terminal: a restart or a changed HEAD never repeats it
([ADR-DE-007](docs/adr/ADR-DE-007-a-night-is-identified-by-its-directory.md)). The gate
returns `ACCEPT`, `REJECT`, `INCONCLUSIVE` or `BLOCKED-ENV`. The full account
is in [docs/concepts.md](docs/concepts.md).

## Crates

| Crate | What it does |
|---|---|
| [`pairwise-gate`](crates/pairwise-gate) | Selection policies over paired candidate and baseline results: `Margin`, `Floor`, `SignTest`, `WilsonLower`, `NoRegression`, `TrainDev`. Faults never count as losses. |
| [`failure-fingerprint`](crates/failure-fingerprint) | A 16-hex-character SHA-256 identity for a failure mode, and a `New` / `Persisting` / `Fixed` classification across steps. |
| [`dream-gate`](crates/dream-gate) | The pure gate: manifest freezing, typed evaluator receipts, the protected-path guard, config-digest binding and the veto classes that turn evidence into a verdict. |
| [`dream-core`](crates/dream-core) | Evaluator runners, the candidate worktree (guard before apply), the ledger and its signals, per-test output parsers, harness-stall and dry-streak tracking, replay, source reading, night idempotency, automerge policy and the five seams (`Llm`, `MemorySink`, `EvaluatorRunner`, `Notifier`, `Forge`). The nightly driver that sequences them belongs to the host. |
| [`dream-cli`](crates/dream-cli) | The `dream` command: offline checks (`validate`, `ledger verify`, `witness`, `replay verify`, `guard check`). No network, no model. |

The statistics are checked against reference values computed independently in
exact arithmetic: nine wins out of ten gives an exact one-sided binomial p of
11/1024, and a Wilson lower bound for eight of ten at z = 1.96 is 0.4902.

What is not here yet: the nightly driver itself (a host supplies it by
implementing the seams), `TrainDev` with held-out dev evaluators wired into the
nightly path, and post-publish recheck with ledger-driven rollback. The forge
step of automerge is unit-tested against a stub but has not been exercised
against real GitHub.

## Quickstart

Add a library:

```sh
cargo add pairwise-gate
cargo add failure-fingerprint
```

Install the command line and run its checks in a clone:

```sh
cargo install dream-cli          # installs the `dream` binary
git clone https://github.com/DreamLab-AI/dream-engine
cd dream-engine
dream validate                   # checks ./dream.config.json
dream guard check --patch my.diff --config dream.config.json
dream witness stamp --report README.md --commit "$(git rev-parse HEAD)"
```

`dream guard check` exits `0` when the patch is clear, `1` when it touches a
protected path and `2` when it could not run. `dream replay verify RUN_DIR`
re-judges a recorded night and `dream ledger verify PATH` checks a ledger.
There is no `dream run`: the nightly driver is the host's, built from
`dream-core`.

To build and test from source:

```sh
cargo test --workspace --locked
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
