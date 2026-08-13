# Dream Machine

**A config-driven engine for nightly, cloud-scheduled, evidence-gated repository evolution.**

> Freeze the model. Evolve the harness. Promote only what proves lift — and
> even then, only *recommend* it to a human. The Dream Machine never merges.

The Dream Machine turns "run one honest research-and-evolution cycle against a
repository every night" into a reusable, config-compiled engine instead of a
hand-maintained megaprompt. It is the generalization of two production
routines that already run nightly in Anthropic's cloud scheduler:

- **Ruflo Nightly Dream Cycle** → `ruvnet/ruflo` (cron `0 6 * * *` UTC)
- **MetaHarness Nightly Dream Cycle** → `ruvnet/metaharness` (cron `0 8 * * *` UTC)

Both are ~800-line single prompts running the same 26-step pipeline; they
differ only in a small, well-defined set of per-repo details. The Dream
Machine factors the shared spine into an engine and the differences into a
`dream.config`.

## The pipeline

```
ledger → research → frozen hypothesis → concrete candidate → baseline
  → evaluation → adversarial critique → bounded Darwin evolution
  → flywheel evidence → witness → issue → draft PR → durable ledger row
```

Every night ends in exactly one of three verdicts — `ACCEPT`, `REJECT`,
`INCONCLUSIVE` — never a fourth, never silence. A rejected hypothesis with a
clean measurement is a **successful** night. The system optimizes for
shrinking tomorrow's search space, not for producing PRs.

The one invariant that survives everything: **evaluation is not promotion.**
The nightly session publishes gists, issues, and always-*draft* PRs. Merging
is a human act, every night, with no exceptions.

## What it composes (not reimplements)

The Dream Machine builds on the ruvnet stack as **optional, config-selected
evaluation backends** — never hard dependencies. A night with none of them
available is a *degraded* night, not a failed one.

| Capability | Package | Used for |
|---|---|---|
| Promotion gate + receipts + replay | [`@metaharness/flywheel`](https://www.npmjs.com/package/@metaharness/flywheel) | evidence retention (STEP 13) + promotion gate (STEP 14) |
| Bounded evolution | [`@metaharness/darwin`](https://www.npmjs.com/package/@metaharness/darwin) | the fenced Darwin stage (STEP 12) |
| Adversarial red/blue | [`@metaharness/redblue`](https://www.npmjs.com/package/@metaharness/redblue) | adversarial critic (STEP 10) + reward-hack scan (STEP 11) |
| Security scan / genome / audit | [`metaharness`](https://www.npmjs.com/package/metaharness) CLI | security review (STEP 15) + control-plane discovery |
| Memory / hooks / orchestration | ruflo (`@claude-flow/cli`) | the cloud session's Task/memory substrate |
| Vector memory over prior nights | [`ruvector`](https://www.npmjs.com/package/ruvector) / [`agentdb`](https://www.npmjs.com/package/agentdb) | (roadmap) semantic search over the ledger |

## Planned packages

- **`@dream-machine/compile`** — `dream.config` → the full routine prompt
  (both the scheduler copy and the in-repo mirror become build *outputs*, not
  hand-synced twins).
- **`@dream-machine/ledger`** — parse / append / verify the 10-column
  `LEDGER.md`; compute learning signals as library code.
- **`@dream-machine/witness`** — `stamp` / `verify` the
  `sha256(sha256(gist) + SESSION_COMMIT)` provenance scheme.
- **`@dream-machine/schedule`** — push a compiled prompt to the cloud
  `/schedule` routine and keep the mirror in lockstep.
- **Evaluation adapters** — thin wrappers delegating to flywheel / darwin /
  redblue through `dream.config`.

## Status

Founding design: **[ADR-0001](docs/adrs/ADR-0001-dream-machine-engine.md)**
(Proposed). No engine code has shipped yet — the two production routines are
the working reference implementations the engine will subsume.

## Prior art

The design deliberately answers a known failure mode of open-loop autonomous
research agents (Sakana AI's "The AI Scientist" reward-hacking incident;
AutoGPT/BabyAGI-era loops with no promotion gate and ~5% follow-through). The
promotion gate, the adversarial critic, the reward-hack check, and the
human-only merge boundary exist precisely because of them. See the ADR's
References.

## License

MIT © rUv
