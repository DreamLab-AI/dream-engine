# Migrating from the TypeScript fork

This is for operators of the TypeScript fork of `ruvnet/dream-machine`. The
decision is in [ADR-DE-001](../adr/ADR-DE-001-standalone-rust-engine.md).

## What stays

The ledger row contract and the witness hash carry over unchanged, so existing
ledgers keep reading. The loop's shape also carries over: a nightly cycle, an
evidence gate and a draft pull request that a human merges.

## What is dropped

The Darwin mutator is not ported. An autopsy found it structurally
non-discriminative: it mutated comments in orphaned files that nothing
imports, and it scored a uniform 0.985 across 21 variants. A score that does
not move cannot rank anything, so the mutator was producing noise that looked
like evidence. Rather than carry a component that cannot discriminate,
the engine asks the model for a patch and judges that patch on the
repository's real evaluators.

## What is new

| Addition | Where |
|---|---|
| Manifest freeze before any model call, with a run id that is a pure function of the manifest; restarts resume, diverged baselines are archived | [concepts](../concepts.md) |
| Four verdict classes, with `BLOCKED-ENV` kept apart from `REJECT` | [concepts](../concepts.md) |
| A protected-path guard and config-digest binding; the Rust engine had none before this repository | [ADR-DE-003](../adr/ADR-DE-003-protected-path-guard.md) |
| Pairwise selection policies, exact binomial and Wilson bounds, faults excluded from the tally | [ADR-DE-004](../adr/ADR-DE-004-pairwise-selection-and-fingerprints.md) |
| Failure fingerprints and `New` / `Persisting` / `Fixed` | [ADR-DE-004](../adr/ADR-DE-004-pairwise-selection-and-fingerprints.md) |
| Five seams in place of product code in the loop | [ADR-DE-005](../adr/ADR-DE-005-seams.md) |
| Automerge as an opt-in, default off | [ADR-DE-006](../adr/ADR-DE-006-automerge-is-an-option.md) |

Of these, everything except `TrainDev` with held-out dev evaluators is
implemented. Further additions: night idempotency
([ADR-DE-007](../adr/ADR-DE-007-a-night-is-identified-by-its-directory.md)),
replay with `dream replay verify`, per-test parsers for libtest, JUnit and
Vitest JSON, and a dry streak in which `REJECT` now advances (the fork reset it).
Legacy free-form `adrConvention` templates such as `ADR-XX-NNN` still load.

## Order of work

Keep the fork running until your host's Rust driver passes the parity checks
(`crates/dream-core/tests/ts_parity.rs`, fixtures under `tests/heldout/ts-parity`).
This repository supplies the library and the offline `dream` checks; the nightly
driver is the host's.
Product-specific parts (the forum adapter, Nostr identity, the key envelope)
move into the consuming product as implementations of the seams, under their
existing licences ([ADR-DE-002](../adr/ADR-DE-002-licence-and-provenance.md)).
