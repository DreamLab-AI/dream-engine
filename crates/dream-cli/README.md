# dream-cli

```sh
cargo install dream-cli   # installs the `dream` binary
```

The `dream` command: offline checks for the [dream-engine](https://github.com/DreamLab-AI/dream-engine) gate. No network, no model.

```text
dream validate [--config PATH]               validate a dream.config.json
dream ledger verify PATH                     structurally verify a ledger
dream witness stamp --report F --commit H    print the witness binding F to H
dream witness verify --report F --commit H --witness W
dream replay verify RUN_DIR                  re-judge a recorded night
dream guard check --patch FILE [--config P]  print touched paths, or the violation
```

`dream validate` defaults to `./dream.config.json`. There is no `dream run`: the
nightly driver is the host's, built from `dream-core`.

Exit codes: `0` passed, `1` the check ran and said no, `2` the check could not run.

`replay verify` shows that a run directory's stored inputs and decision agree;
it is not tamper evidence by itself. Bind the directory with a witness for that.

Licensed under MIT OR Apache-2.0.
