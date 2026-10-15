//! The repository's own `dream.config.json` must pass dream-gate's validation
//! and keep the properties that make it safe to let the engine dream this repo.

use std::path::PathBuf;

use dream_gate::config::DreamConfig;
use dream_gate::guard::ProtectedPolicy;

fn load() -> DreamConfig {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dream.config.json");
    DreamConfig::load(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn the_repo_config_validates() {
    load().validate().unwrap();
}

#[test]
fn automerge_is_off() {
    assert!(!load().auto_merge);
}

#[test]
fn the_evaluators_are_the_workspace_gates_and_there_is_no_darwin() {
    let c = load();
    let cmd = |k: &str| c.evaluator_entrypoints[k].cmd.clone();
    assert_eq!(cmd("tests"), "cargo test --workspace --locked");
    assert_eq!(cmd("clippy"), "cargo clippy --workspace --all-targets -- -D warnings");
    assert_eq!(cmd("fmt"), "cargo fmt --all --check");
    assert_eq!(cmd("doc"), "RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace");
    assert_eq!(cmd("deny"), "cargo deny check");
    assert!(cmd("parity").contains("--test ts_parity"));
    assert!(c
        .evaluator_entrypoints
        .keys()
        .all(|k| !k.to_lowercase().contains("darwin")));
    assert!(c
        .evaluator_entrypoints
        .values()
        .all(|e| !e.cmd.to_lowercase().contains("darwin")));
}

#[test]
fn held_out_tests_gate_crates_and_policy_files_are_protected() {
    let c = load();
    let p = ProtectedPolicy::from_config(&c);
    for path in [
        "tests/heldout/ts-parity/fork-LEDGER.md",
        "crates/dream-core/tests/ts_parity.rs",
        "crates/dream-gate/src/gate.rs",
        "crates/pairwise-gate/src/lib.rs",
        "crates/failure-fingerprint/src/lib.rs",
        "deny.toml",
        "dream.config.json",
        ".github/workflows/ci.yml",
        ".github/CODEOWNERS",
    ] {
        assert!(p.is_protected(path), "{path} must be protected");
    }
    assert!(!p.is_protected("crates/dream-core/src/ledger.rs"));
}
