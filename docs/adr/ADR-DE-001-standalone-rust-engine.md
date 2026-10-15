---
id: ADR-DE-001
title: Externalise the Rust dream engine as a standalone permissive repository, replacing the TypeScript fork
date: 2026-10-15
decision_status: accepted
implementation_status: in-progress
activation_status: staged
supersedes: []
owner: jjohare
review_trigger: a second consumer that needs a seam the five traits do not cover, or a licence objection to the permissive core
---

# ADR-DE-001 — A standalone Rust engine replaces the TypeScript fork

## Context

The nightly loop began as a TypeScript fork of `ruvnet/dream-machine`, patched
inside one product. A Rust engine followed, also inside that product. Both
copies carried product-specific code (a forum adapter, Nostr identity) next to
the loop, so nobody else could use the loop without taking the product with it,
and the product carried two engines.

## Decision

Move the engine into its own repository, `DreamLab-AI/dream-engine`, as a Rust
workspace with permissive licensing. The TypeScript fork is retired once the
Rust driver reaches parity on the points listed in the
[migration note](../guide/migrating-from-the-ts-fork.md). The workspace holds
four crates: `pairwise-gate`, `failure-fingerprint`, `dream-gate` and
`dream-core`. Everything that touches a product (a forum, a relay, a key
envelope) stays in the consuming product behind the seams in
[ADR-DE-005](ADR-DE-005-seams.md).

## Consequences

One engine to maintain and one place to fix gate bugs. The loop can be adopted
by repositories that have nothing to do with the original product. The cost is
an adapter layer in each consumer and a stable seam contract that has to be
versioned. As of this record `pairwise-gate` and `failure-fingerprint` are in
the repository; `dream-gate` and `dream-core` are planned.
