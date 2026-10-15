# Security

## Reporting

Report vulnerabilities privately through GitHub's private vulnerability
reporting on this repository (Security tab, "Report a vulnerability"). Please
do not open a public issue. Include the version or commit, what you observed
and how to reproduce it.

## Threat model in brief

Candidate patches are untrusted. They are produced by a model that may have been
steered by repository content, and they must be treated like a pull request from
a stranger.

Build scripts execute on the evaluation host. `cargo test`, `cargo clippy` and
`cargo doc` run `build.rs` files and procedural macros from the candidate tree,
so a patch can run arbitrary code during evaluation. The evaluation host must
be sandboxed: no credentials, no write access to anything outside the checkout,
and restricted network access. The engine's guards do not replace this.

The guards that exist or are planned address a different risk, a candidate
changing the judge. The protected-path guard and config-digest binding are
described in [ADR-DE-003](docs/adr/ADR-DE-003-protected-path-guard.md) and are
planned. A violation is a rejection. Evaluator and testbed failures are
reported as `BLOCKED-ENV` and never as a pass.

The human-merged draft pull request is the main control on what lands.
Automerge is off by default and bounded
([ADR-DE-006](docs/adr/ADR-DE-006-automerge-is-an-option.md)).

## Out of scope

The adapter for a private forum or Nostr deployment and its key envelope are not
part of this repository ([ADR-DE-002](docs/adr/ADR-DE-002-licence-and-provenance.md)).
