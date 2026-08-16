# Security Policy

## Reporting a vulnerability

Please report security issues privately via GitHub Security Advisories
(**Security → Report a vulnerability**) on this repository, or email
ruv@ruv.net. Do not open a public issue for a suspected vulnerability.

We aim to acknowledge within 72 hours.

## Threat model & design guarantees

The Dream Machine runs an autonomous nightly loop against a repository. Its
safety rests on a small set of invariants, enforced in code and CI:

- **Evaluation is not promotion.** The nightly session never merges and never
  self-promotes candidate state. It only opens *draft* PRs.
- **Guarded auto-merge.** The optional auto-merge job (`.github/workflows/automerge.yml`)
  refuses any PR that touches a protected path (evaluation gates, safety,
  thresholds, CI workflows, dependency manifests) and requires an explicit
  `automerge-safe` label plus all required checks green.
- **Least-privilege CI.** Each workflow declares the minimum `permissions:`
  it needs; `contents: read` is the default.
- **Optional dependencies are optional.** The ruvector/RVF wasm backends are
  peer-optional; a missing module is a graceful no-op (verified by the
  `no-optional-deps` CI job).
- **Witnessed provenance.** Every nightly report is bound to its commit by a
  reproducible double-sha256 witness that any third party can re-derive.

## Supply chain

- npm publishes use OIDC provenance (`--provenance`).
- Dependabot watches npm and GitHub Actions.
- CodeQL (`security-extended`) runs on every push, PR, and weekly.
