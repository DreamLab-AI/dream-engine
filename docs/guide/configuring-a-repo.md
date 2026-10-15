# Configuring a Rust repository

> The driver that reads this file is planned. The field names below describe the
> intended shape and may change before `dream-core` is released.

A repository opts in with a `dream.config.json` at its root. The file is
protected: a candidate patch may not alter it
([ADR-DE-003](../adr/ADR-DE-003-protected-path-guard.md)), and its digest is
frozen into every manifest.

## Evaluators for a Rust workspace

Each evaluator is a command. The engine records the sha256 of the command text
in the manifest, so editing a command starts a new experiment. Five cover most
Rust repositories.

| Evaluator | Command | What it measures |
|---|---|---|
| tests | `cargo test --workspace --locked` | behaviour |
| lints | `cargo clippy --workspace --all-targets -- -D warnings` | warnings as failures |
| format | `cargo fmt --check` | formatting |
| docs | `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps` | rustdoc warnings as failures |
| supply chain | `cargo deny check` | advisories, licences and sources |

## Example

```json
{
  "evaluators": [
    { "name": "tests",  "command": "cargo test --workspace --locked" },
    { "name": "clippy", "command": "cargo clippy --workspace --all-targets -- -D warnings" },
    { "name": "fmt",    "command": "cargo fmt --check" },
    { "name": "docs",   "command": "RUSTDOCFLAGS='-D warnings' cargo doc --no-deps" },
    { "name": "deny",   "command": "cargo deny check" }
  ],
  "protected": [
    "dream.config.json",
    "deny.toml",
    "rust-toolchain.toml",
    ".github/workflows/**"
  ],
  "automerge": false
}
```

The shape of `evaluators` and `protected` is illustrative. Two points are
settled by the ADRs. `automerge` defaults to false
([ADR-DE-006](../adr/ADR-DE-006-automerge-is-an-option.md)). Protected paths
should include every file that changes what an evaluator measures: lint
configuration, toolchain pins, CI definitions and any held-out test set.

## Points to watch

An evaluator with no output, a non-zero exit or a timeout vetoes, so a command
that cannot run in your environment gives `BLOCKED-ENV` every night until it is
fixed. Run each command once by hand on a clean checkout first.

Build scripts and procedural macros execute during `cargo test` and `cargo
clippy`, so the evaluation host runs untrusted code. Sandbox it
([SECURITY.md](../../SECURITY.md)).

Where you want a statistical gate rather than a pass-or-fail one, hold at least
20 per-test cases per policy that uses the sign test or the Wilson bound
([ADR-DE-004](../adr/ADR-DE-004-pairwise-selection-and-fingerprints.md)).
