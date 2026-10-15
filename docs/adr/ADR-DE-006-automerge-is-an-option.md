---
id: ADR-DE-006
title: Automerge is an option, default off; the human-merged draft pull request is the default
date: 2026-10-15
decision_status: accepted
implementation_status: implemented
activation_status: default-off
supersedes: []
owner: jjohare
review_trigger: a repository that wants automerge and has no required status checks, or evidence that human review is not catching accepted-but-wrong patches
---

# ADR-DE-006 — Automerge is an option, default off

## Context

An accepted verdict says the evaluators the repository chose did not object. It
does not say the change is wanted. The nightly drafts open as draft pull
requests and a human merges them.

## Decision

Human merge stays the default path. Automerge is a field (`autoMerge`, alias `automerge`) in the protected
`dream.config.json` and its default is off. Three rules bound it:

1. The field is ignored for self-referential targets, that is a repository
   that contains the gate crates. The engine never merges changes to itself.
2. It is refused unless the pull request's real base branch has required status
   checks (an empty list, a 404 or an API error all refuse), so a merge cannot
   bypass the checks that protect the branch. When it proceeds, the engine runs
   `gh pr ready` and then `gh pr merge --auto --squash`.
3. Because the config is protected ([ADR-DE-003](ADR-DE-003-protected-path-guard.md)),
   a candidate patch cannot switch it on.

## Consequences

A repository that wants unattended merges has to opt in and has to have branch
protection first. Everyone else gets a draft pull request each morning. Both
the field and its checks are implemented in `dream_core::automerge` and
`dream_core::persist::publish` and unit-tested against a stub forge. The `gh`
calls have not yet been exercised against real GitHub, so keep the option off
until they have.

The policy applies only on the `persist::publish` path. A host that publishes
through `Forge::publish_draft` directly gets draft pull requests and no merging,
whatever the field says. The first consuming host does exactly that, so the
option is inert there until the host is switched to `persist::publish`; that is
a deliberate, separate step, taken only after the `gh` calls have been
exercised.
