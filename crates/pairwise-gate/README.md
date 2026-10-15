# pairwise-gate

[![crates.io](https://img.shields.io/crates/v/pairwise-gate)](https://crates.io/crates/pairwise-gate)
[![docs.rs](https://img.shields.io/docsrs/pairwise-gate)](https://docs.rs/pairwise-gate)

```sh
cargo add pairwise-gate
```

Evidence-gated selection policies for comparing a **candidate** against a
**baseline** on the same ordered list of tasks. Each policy returns
`Select`, `Reject` or `Inconclusive`, with a reason and the numbers behind it.

The distinguishing rule: a **fault** (service down, timeout, patch would not
apply) is never a loss and never a win. Faulted pairings are excluded from the
tally, reported, and, if there are too many or too few comparable pairings
remain, produce `Inconclusive`, so infrastructure trouble is distinguishable
from evidence against the candidate.

| Policy | Selects when |
|---|---|
| `Margin { min_win_margin }` | `wins - losses > margin` |
| `Floor { floor }` | at least one candidate score, all `>= floor` (candidate-only) |
| `SignTest { alpha }` | exact one-sided binomial p-value over non-tied pairs `< alpha` |
| `WilsonLower { z }` | Wilson lower bound of the win rate among non-tied pairs `> 0.5` |
| `NoRegression { pass_threshold }` | rejects any pass-to-fail pairing, regardless of wins elsewhere |
| `TrainDev { train, dev }` | train selects and held-out dev does not reject |

```rust
use pairwise_gate::{Evidence, Outcome, Pairing, Policy, SignTest, Verdict};

let mut pairings = Vec::new();
for i in 0..9 {
    pairings.push(Pairing::new(format!("task-{i}"), Outcome::score(0.4)?, Outcome::score(0.7)?));
}
pairings.push(Pairing::new("task-9", Outcome::score(0.7)?, Outcome::score(0.4)?));
pairings.push(Pairing::new("task-10", Outcome::fault("runner offline"), Outcome::score(0.1)?));

let decision = SignTest::new(0.05)?.decide(&Evidence::new(&pairings));
assert_eq!(decision.verdict, Verdict::Select);
assert_eq!(decision.metrics.faults, 1);
# Ok::<(), pairwise_gate::Error>(())
```

The sign test and Wilson bound are meaningful only with enough per-test cases;
the dream-engine gate requires at least 20. The sign test uses an exact binomial tail computed in log space, stable to
`n = 10 000` and beyond, with no statistics dependency. Reference values in the
test suite were computed independently with exact integer arithmetic.

## Features

* `serde` (default): serialisation of configuration and result types.
  Deserialisation applies the constructors' validation. `NaN` scores are
  rejected at construction.

No `unsafe`, and no dependencies without the `serde` feature.

## Licence

Licensed under either of MIT or Apache-2.0 at your option.
