# Configuring a Rust repository

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
  "repo": "owner/name",
  "slots": [
    { "deep": "core", "scan": ["src"] }
  ],
  "evaluatorEntrypoints": {
    "tests":  { "cmd": "cargo test --workspace --locked", "required": true, "timeoutSecs": 900, "testFormat": "libtest" },
    "clippy": "cargo clippy --workspace --all-targets -- -D warnings",
    "fmt":    "cargo fmt --all --check",
    "doc":    "RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace",
    "deny":   "cargo deny check"
  },
  "ledgerPath": "docs/dream-cycle/LEDGER.md",
  "autoMerge": false,
  "protected": [
    "deny.toml",
    "rust-toolchain.toml",
    ".github/**",
    "Cargo.lock"
  ]
}
```

`dream validate` accepts this file. Notes on the fields:

- `evaluatorEntrypoints` maps a name to a bare command string (required, every
  deep, 1800 s timeout) or to an object with `cmd`, `required`, `deeps`,
  `timeoutSecs` and `testFormat` (`libtest`, `junit` or `vitest-json`). An
  evaluator with no `testFormat` counts as one case in the gate, so it gets pass
  or fail but no sign test or Wilson bound. Do not pipe a declared-format
  evaluator through `| tail`; the parser needs the full output. An evaluator may
  also name a `fixer` (today only `"rustfmt"`): a built-in the engine runs on the
  files the candidate touched before the gate judges it, so a formatting gate
  never vetoes a night for the model's line-wrapping (ADR-DE-009). It is a name,
  never a command; the layout lands as a second commit and anything that would
  reach a file the candidate did not touch is discarded.
- `protected` is additive. It extends the built-in protected set (the config
  file itself, the evaluator paths and the ledger) and can never remove from
  it; a `!` negation is a validation error.
- `autoMerge` (alias `automerge`) defaults to false
  ([ADR-DE-006](../adr/ADR-DE-006-automerge-is-an-option.md)). It is refused for
  self-referential targets and unless the pull request's real base branch has
  required status checks.
- `adrConvention` takes `"3-digit"`, `"4-digit"`, `{ "pad": 3, "dir": "docs/adr" }`
  or a legacy free-form template such as `"ADR-XX-NNN"`.

Protected paths should include every file that changes what an evaluator
measures: lint configuration, toolchain pins, CI definitions and any held-out
test set.

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
