# failure-fingerprint

[![crates.io](https://img.shields.io/crates/v/failure-fingerprint)](https://crates.io/crates/failure-fingerprint)
[![docs.rs](https://img.shields.io/docsrs/failure-fingerprint)](https://docs.rs/failure-fingerprint)

```sh
cargo add failure-fingerprint
```

Stable identity for failure modes across the nights or steps of an
evidence-gated self-improvement loop. A loop can tell a **new** failure from a
**persisting** one from a **fixed** one, and a proposer reads structured
records instead of prose.

- `normalize_cause` strips volatile parts (absolute paths, long opaque ids,
  digit runs, whitespace) and truncates on a `char` boundary. Long identifiers
  are replaced, which keeps key material and credentials out of persisted causes.
- `fingerprint(task, stage, cause)` is the first 8 bytes of SHA-256 over the
  task, stage and normalised cause joined with `0x1F`, shown as 16 hex chars. That is 64 bits: enough to separate failure modes within
  one repository, not collision-resistant across repositories.
- `advance(prev, observations, step)` returns the next `Manifest` and a
  `Classification` (`new`, `persisting`, `fixed`), deterministically sorted.
- Manifests serialise as `{"kind":"failure-manifest/1","step":..,"records":[..]}`;
  an unknown `kind` is rejected with a typed error.

```rust
use failure_fingerprint::{advance, Manifest, Observation, Stage};

let night1 = [Observation::new("build", Stage::new("launch"), "panic at /tmp/a1/b/c.rs:41")];
let (m1, c1) = advance(&Manifest::new(0), &night1, 1);
assert_eq!(c1.new.len(), 1);

let night2 = [Observation::new("build", Stage::new("launch"), "panic at /var/z9/b/c.rs:97")];
let (_m2, c2) = advance(&m1, &night2, 2);
assert_eq!(c2.persisting.len(), 1);
```

The stage vocabulary belongs to the caller (for example `launch`, `trajectory`,
`exit`, `evaluator`). The crate contains no `unsafe` code.

## Licence

Licensed under either of MIT or Apache-2.0 at your option.
