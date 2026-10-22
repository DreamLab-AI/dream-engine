---
id: ADR-DE-009
title: A candidate is formatted before the gate judges it; layout is never why a night fails
date: 2026-10-22
decision_status: accepted
implementation_status: implemented
activation_status: staged
supersedes: []
owner: jjohare
review_trigger: a new `Fixer` variant, a change to what `format_candidate` is allowed to touch, or a fixer that is not a pure formatter
---

# ADR-DE-009 — A candidate is formatted before the gate judges it; layout is never why a night fails

## Context
Three consecutive dream-engine nights (2026-10-19 to 2026-10-22) ended `REJECT` on
`fmt: required evaluator exited 1`, and the nights themselves, and then the governance
board, blamed the harness: "the code that runs the checks still wraps each one in a
second shell, the wrapping class that broke three nights of checks". That diagnosis was
wrong, and a decision written on it ("checks are stored and run as direct command lists,
never wrapped in a shell string") would have changed nothing. The receipts say what
happened:

- the **baseline** `fmt` receipt on each night passed (`exit=0`, the success echo
  present), so the harness ran the check correctly;
- the **candidate** `fmt` receipt is a plain `rustfmt --check` diff of the model's own
  new lines: an `assert!` or a call the model kept on one line that rustfmt wraps
  (`…/2026-10-22-dream-engine/receipts/candidate/fmt.stdout`);
- `shell_quote` (`dream-gate/src/outcome.rs`) is POSIX single-quote escaping with a
  test for an embedded quote, and the exact double-wrapped command from
  `dream-core/src/runner.rs` returns `exit=0` with its echo when run by hand;
- the 10-22 report listed, as its own biggest uncertainty, "whether the engine's rustfmt
  agrees byte-for-byte".

A language model cannot reproduce rustfmt's line-wrapping from a prompt, and a
required `fmt` gate was vetoing a sound, tested change for a property a formatter
restores mechanically. Formatting is not evidence about the hypothesis; the gate was
measuring the wrong thing.

## Decision
- An evaluator may name a **fixer**: `"fmt": { "cmd": "cargo fmt --all --check", "fixer": "rustfmt" }`.
  A fixer is a *name* from a built-in set (`Fixer`, today only `rustfmt`), never a
  command. The control plane executes nothing a repository's configuration spells out.
- `candidate::run` runs the fixer after the patch is applied and committed in its
  worktree and **before** the tree is hashed for shipping and judged
  (`candidate::format_candidate`). It runs on the control plane, where the worktree is;
  the annexe only ever evaluates.
- The fixer is given **only the files the candidate touched** that it handles
  (`rustfmt --edition <crate edition> -- <files>`). If it changes any other file the
  work is discarded (`Reverted`); the protected-path guard already passed on the
  patch and the fixer must never widen the candidate's footprint. A touched file that
  declares `#[path]` modules is not formatted at all (`Skipped`), since rustfmt would
  follow the declaration out of the worktree.
- On success the layout lands as a **second commit** (`dream: rustfmt`), so a reviewer
  sees the model's diff and the engine's layout apart and the model's own tree is kept
  for audit. `tree_hash` and `applied_patch` become the formatted tree's, and
  `applied_patch` is `git diff --binary <base> HEAD`, re-guarded, so base +
  `applied_patch` still reproduces `tree_hash` and a replay sees the bytes the gate
  judged. `patch_digest` and `patch_bytes` stay the model's.
- Every other outcome (`Failed`: non-zero exit, timeout, could not start; `Reverted`;
  `Skipped`; `Unchanged`) leaves the candidate exactly as the model wrote it: one
  commit, clean worktree. The gate then judges the unformatted tree, which is tonight's
  behaviour. The outcome is recorded on the candidate (`PreparedCandidate::formatting`)
  whatever it was, so a night's artefacts say why.
- The fixer's name travels in the frozen manifest (`EvaluatorIdentity::fixer`), so a
  night's config digest changes when a fixer is added and an older manifest reads back
  unchanged.
- `dream-core` and `dream-gate` are released together as **0.3.0**: `EvaluatorSpec`,
  `EvaluatorIdentity` and `PreparedCandidate` gain a public field.

## Consequences
- A night is no longer vetoed for rustfmt's layout; the fixer is itself a built-in,
  reviewed piece of the engine, so the gate's guarantee is unchanged: the evaluators
  still run on the exact tree that is shipped and judged.
- Out-of-worktree writes by rustfmt through a model-written `#[path]` are closed by the
  skip; a baseline `#[path]` is operator-authored and formatted like any other module.
  `rustfmt.toml` is not a built-in protected path; a repository that wants its formatter
  configuration frozen adds it to `protected`.
- Consumers (the agentbox engine) pin 0.3.0, carry `formatting` into `candidate.json`,
  and say in the draft PR that the layout is rustfmt's and the logic the model's.
- Other fixers are additions to `Fixer`, each with the same footprint rule; a fixer that
  is not a pure formatter (one that changes behaviour) does not belong here.
