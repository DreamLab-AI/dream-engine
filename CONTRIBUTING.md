# Contributing

Contributions are welcome under `MIT OR Apache-2.0`; by submitting one you
agree it is licensed the same way. If it includes code from elsewhere, say
where it came from in the pull request. Do not add a dependency on an AGPL
crate ([ADR-DE-002](docs/adr/ADR-DE-002-licence-and-provenance.md)).

## Before you open a pull request

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo deny check
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace
```

Every public item carries rustdoc, and examples must compile. The minimum Rust
version is 1.89.

## Rules that matter here

Cryptography goes through established crates (`sha2` for fingerprints). Do not
write your own primitives.

A change to a gate policy, the protected-path guard or a verdict class needs a
test that fails without it, and an ADR if it changes a decision. Use the next
free `ADR-DE-NNN` and add it to [docs/adr/README.md](docs/adr/README.md).

Statistical code is checked against reference values computed independently,
not against its own output.

Keep product specifics out of this repository. Anything that names a particular
forum, relay or key envelope belongs in the consuming product behind the
[seams](docs/adr/ADR-DE-005-seams.md).

Documentation says "planned" for anything not yet in the repository. Please keep
to that, and check a claim against the code (or a test) before you move it out of
"planned".
