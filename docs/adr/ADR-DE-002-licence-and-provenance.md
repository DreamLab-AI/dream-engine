---
id: ADR-DE-002
title: Licence and provenance
date: 2026-10-15
decision_status: accepted
implementation_status: implemented
activation_status: staged
supersedes: []
owner: jjohare
review_trigger: any contribution that carries third-party code, or a request to publish an adapter
---

# ADR-DE-002 — Licence and provenance

## Context

The design descends from `ruvnet/dream-machine` by rUv, which is MIT licensed.
The consuming product has components under AGPL-3.0-only and a private
encryption envelope that must not become a public format.

## Decision

The five crates are `MIT OR Apache-2.0`. They are an independent Rust
implementation of the design lineage and copy no source text from
`dream-machine`. Its MIT notice is reproduced in `NOTICE` as credit and as a
precaution.

The adapter that connects the engine to the estate forum and to Nostr is
`AGPL-3.0-only`. It lives in the consuming product and is never published from
this repository or to crates.io. The zone-crypto envelope used by that product
is also never published; it is a project-specific construction and a bespoke
envelope is not something to ship as a reusable crate.

A permissive crate must never depend on an AGPL crate. Contributions are
accepted under the same dual licence, and a contribution that includes code
from elsewhere has to say where it came from.

## Consequences

Any repository can use the core under either licence. The AGPL boundary is
physical: the adapter is not in this tree, so it cannot be pulled in by
accident. `cargo deny check` runs in CI (`.github/workflows/ci.yml`) against `deny.toml`,
which allows only permissive licences, so an AGPL dependency fails the build.
