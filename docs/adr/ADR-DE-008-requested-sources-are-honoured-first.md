---
id: ADR-DE-008
title: Sources the previous night asked for are read first, capped, and never asked of the operator
date: 2026-10-17
decision_status: accepted
implementation_status: implemented
activation_status: staged
supersedes: []
owner: jjohare
review_trigger: a change to the source planner's selection order, to `requested_slots`, or to the half-budget reservation
---

# ADR-DE-008 — Sources the previous night asked for are read first, capped, and never asked of the operator

## Context
The tree-read stage (ADR-2114 in the consuming engine) lets an LLM planner pick up to `max_files` files to inline. The planner saw only tonight's evidence and the file index. When a night ended INCONCLUSIVE for want of a file, its report named the file, and the consuming engine turned that line into an operator question. Across four repositories the same request then reached the human three nights running ("operator-approved 10-09, now missed twice"): a model asked to choose files cannot be held to a list, and the human was relaying a list the engine itself had written.

## Decision
- `GatherInput` carries `requested: &[SourceRequest]`: paths, optionally with a line range, that the previous night or the operator asked to see. The struct is `#[non_exhaustive]` with `GatherInput::new` and `with_requested`, so the next field is not a breaking change (this one was: 0.1.1 → 0.2.0).
- `select` honours requested paths **first**, then planner picks, then evidence-mentioned paths. Requested paths take at most `requested_slots(max_files)` = half the file slots (at least one), and `render_section` lets them share at most half the byte budget; beyond that a requested file is clipped with the usual marker, never dropped silently.
- Every requested path passes the same checks as a planner pick: relative, no traversal, tracked at the dispatched commit, not denied, not binary, not oversize, no key material. The report proposes; the tree decides.
- The planner prompt lists the honoured requests as already being read, so it does not spend picks on them.
- The section ends with one line naming which requests were honoured and which were not, with the reason, and tells the model to list further needs under **Sources wanted next night** rather than ask the operator. `SourceRecord` carries the same two lists (`requested_honoured`, `requested_refused`) so the consumer can expire a request once it has been met.
- dream-core stays unaware of report headings and of operators: parsing the previous report, storing requests, expiring them (met once, or after three nights) and alerting on a path requested again after it was inlined are the consumer's job.

## Consequences
- A stale or over-broad request can occupy at most half of tonight's read; the planner always keeps half for what tonight's evidence points at.
- Source supply stops being a governance decision. A human sees a source request only if the consumer's expiry raises an alert about a path that was inlined and asked for again, which indicates the wrong window or a real blocker.
- Consumers building `GatherInput` by struct literal must move to the constructor.
