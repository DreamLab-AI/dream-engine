---
id: ADR-DE-006
title: Automerge is an option, default off; the human-merged draft pull request is the default
date: 2026-10-15
decision_status: accepted
implementation_status: planned
activation_status: staged
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

Human merge stays the default path. Automerge is a field in the protected
`dream.config.json` and its default is off. Three rules bound it:

1. The field is ignored for self-referential targets, that is a repository
   that contains the gate crates. The engine never merges changes to itself.
2. It is refused unless the target branch has required status checks, so a
   merge cannot bypass the checks that protect the branch.
3. Because the config is protected ([ADR-DE-003](ADR-DE-003-protected-path-guard.md)),
   a candidate patch cannot switch it on.

## Consequences

A repository that wants unattended merges has to opt in and has to have branch
protection first. Everyone else gets a draft pull request each morning. Both
the field and its checks are planned.
