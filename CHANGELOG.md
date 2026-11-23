# Changelog

Notable changes to the crates in this workspace. Each crate is versioned and
published on its own; entries name the crate they apply to. Versions follow
[Semantic Versioning](https://semver.org/) as Cargo applies it to `0.x`
releases: a change that breaks the public API moves the minor number.

## dream-core 0.5.0 (unreleased)

### Breaking

- **`DraftPr` gains a required `head_sha: String` field.** It is the commit at
  the tip of `branch` that was reviewed and is being pushed. `DraftPr` is a
  plain struct with public fields, so every struct literal that builds one
  stops compiling until it sets the field.
- **`Forge::enable_automerge` takes a third argument, `head_sha: &str`.** Its
  signature is now `fn enable_automerge(&self, repo: &str, pr_url: &str,
  head_sha: &str) -> Result<(), ForgeError>`. A `Forge` implementation that
  overrides the method must add the parameter; one that relies on the default
  (`ForgeError::Unsupported`) needs no change.

These changes landed in commit `6b45ad4` ("fix(seams): pin automerge to the
reviewed head commit") while the crate still declared 0.4.1. That version was
already published without them, so they ship as 0.5.0. A host pinned to
`=0.4.1` keeps building against the published 0.4.1 and does not get them.

### Changed

- `persist::publish` passes `DraftPr::head_sha` to `Forge::enable_automerge`.
- `GhForge::enable_automerge` runs `gh pr merge --auto --squash
  --match-head-commit <head_sha>`, so GitHub merges only while the PR head is
  still that commit. A push after review cancels the automerge instead of
  merging code nobody reviewed.
- `GhForge::enable_automerge` refuses an empty or non-hexadecimal `head_sha`
  with `ForgeError::Failed`. It checks before `gh pr ready`, so a refused PR
  stays a draft. `persist::publish` records the error in
  `Published::automerge_error` and does not enable automerge.
- `GhForge::publish_draft` does not read `head_sha`, and neither does any
  path that never enables automerge.

### Migration

- **Whoever builds a `DraftPr` sets `head_sha`.** This crate never builds one
  for you. Set it to the full commit hash at the tip of `branch` in
  `repo_dir`, read just before publishing (`git rev-parse <branch>`), meaning
  the commit the gate judged. Do not use a tree hash such as
  `PreparedCandidate::tree_hash`: `--match-head-commit` needs a commit.
- A host that calls `Forge::publish_draft` directly, and never
  `persist::publish`, never enables automerge. It still has to set the field
  to compile, and should set it truthfully so the value is right if it later
  moves to `persist::publish`.
- A custom `Forge` that overrides `enable_automerge` should pin the merge to
  `head_sha` the same way, or refuse when it cannot. Ignoring the argument
  brings back the gap this release closes.
- Dependency requirements: `dream-core = "0.5"` (or `=0.5.0`) with
  `dream-gate = "0.3"`. `dream-gate` itself is unchanged.

### Internal

- `quick-xml` 0.41 → 0.42 and `ontomark-compile` 0.6.3 → 0.8.0. Both are
  private dependencies, and none of their types appear in the public API.

## dream-cli 0.4.0 (unreleased)

- Depends on `dream-core` 0.5.0. The CLI's public library API is unchanged
  from the published 0.2.0 (the only source change since then is
  documentation). `dream-core` and `dream-gate` types do not appear in that
  API.

## Unchanged since their last release

- `dream-gate` 0.3.0, `pairwise-gate` 0.1.0 and `failure-fingerprint` 0.1.0:
  no source change since the published versions, so no new release.
