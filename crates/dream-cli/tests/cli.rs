//! End-to-end tests of the `dream` binary. Offline; every case runs the real
//! executable and checks exit status and output.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn dream(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dream"))
        .args(args)
        .output()
        .expect("run dream")
}
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}
const HEADER: &str = "| Date | Deep | Finding | Issue | PR | Evaluated? | Verdict | Effect | Witness | Prior-night fates |\n|---|---|---|---|---|---|---|---|---|---|\n";
const GOOD_ROW: &str = "| 2026-08-13 | d | f | - | - | yes | ACCEPT | e | abc1234 | - |\n";
fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let f = dir.join(name);
    std::fs::write(&f, body).unwrap();
    f
}

const GOOD_CFG: &str =
    r#"{"repo":"o/r","slots":[{"deep":"d","scan":["src"]}],"protected":["secret/**"]}"#;

#[test]
fn validate_accepts_a_good_config() {
    let d = tempfile::tempdir().unwrap();
    let cfg = write(d.path(), "dream.config.json", GOOD_CFG);
    let o = dream(&["validate", "--config", p(&cfg)]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("ok"));
}

#[test]
fn validate_rejects_an_invalid_config_with_exit_1() {
    let d = tempfile::tempdir().unwrap();
    let f = write(
        d.path(),
        "dream.config.json",
        r#"{"repo":"nope","slots":[]}"#,
    );
    let o = dream(&["validate", "--config", p(&f)]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!err(&o).is_empty());
}

#[test]
fn validate_reports_a_missing_file() {
    let o = dream(&["validate", "--config", "/nonexistent/dream.config.json"]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn ledger_verify_passes_a_good_ledger_and_fails_a_bad_row() {
    let d0 = tempfile::tempdir().unwrap();
    let ledger = write(d0.path(), "L.md", &format!("{HEADER}{GOOD_ROW}"));
    let o = dream(&["ledger", "verify", p(&ledger)]);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));

    let d = tempfile::tempdir().unwrap();
    let bad = write(
        d.path(),
        "L.md",
        "| Date | Deep | Finding | Issue | PR | Evaluated? | Verdict | Effect | Witness | Prior-night fates |\n|---|---|---|---|---|---|---|---|---|---|\n| 2026-13-99 | d | f | - | - | yes | MAYBE | e | abc1234 | - |\n",
    );
    let o = dream(&["ledger", "verify", p(&bad)]);
    assert_eq!(o.status.code(), Some(1));
    assert!(out(&o).contains("row 1") || err(&o).contains("row 1"));
}

#[test]
fn witness_stamp_then_verify_round_trips_and_detects_tampering() {
    let d = tempfile::tempdir().unwrap();
    let report = write(d.path(), "report.md", "findings\nVERDICT: REJECT\n");
    let stamped = dream(&[
        "witness",
        "stamp",
        "--report",
        p(&report),
        "--commit",
        "abc1234",
    ]);
    assert!(stamped.status.success(), "{}", err(&stamped));
    let w = out(&stamped).trim().to_string();
    assert_eq!(w.len(), 64);

    let ok = dream(&[
        "witness",
        "verify",
        "--report",
        p(&report),
        "--commit",
        "abc1234",
        "--witness",
        &w,
    ]);
    assert!(ok.status.success());

    std::fs::write(&report, "findings edited\nVERDICT: ACCEPT\n").unwrap();
    let bad = dream(&[
        "witness",
        "verify",
        "--report",
        p(&report),
        "--commit",
        "abc1234",
        "--witness",
        &w,
    ]);
    assert_eq!(bad.status.code(), Some(1));
}

#[test]
fn witness_with_a_malformed_commit_is_an_error_not_a_mismatch() {
    let d = tempfile::tempdir().unwrap();
    let report = write(d.path(), "r.md", "x");
    let o = dream(&[
        "witness",
        "stamp",
        "--report",
        p(&report),
        "--commit",
        "zzz",
    ]);
    assert_eq!(o.status.code(), Some(2));
}

fn night(dir: &Path) {
    use dream_core::replay::{capture, NightInputs};
    use dream_gate::{
        gate::CandidateState,
        manifest::{digest, ExperimentManifest, ModelIdentity},
        pertest::PairwiseOptions,
    };
    let config = br#"{"repo":"o/r","slots":[{"deep":"d","scan":[]}]}"#.to_vec();
    let manifest = ExperimentManifest {
        schema: 1,
        run_id: "abcdef0123456789".into(),
        night_id: "n".into(),
        repo: "r".into(),
        repo_slug: "o/r".into(),
        date: "2026-10-01".into(),
        day_int: 20261001,
        deep: "d".into(),
        scan: vec![],
        baseline_revision: "0".repeat(40),
        baseline_tree_hash: "1".repeat(40),
        config_digest: digest(&config),
        evaluators: vec![],
        model: ModelIdentity {
            provider: "p".into(),
            model: "m".into(),
            max_tokens: 1,
            fallback: None,
        },
        engine_version: "0".into(),
        created_at: "2026-10-01T00:00:00Z".into(),
    };
    let n = NightInputs {
        manifest,
        report: "nothing\nVERDICT: REJECT\n".into(),
        candidate: CandidateState::NotAttempted,
        extracted_patch: None,
        baseline_receipts: vec![],
        candidate_receipts: vec![],
        config_bytes: config,
        fallback_ledger_path: String::new(),
        candidate_config_digest: None,
        tests: Default::default(),
        options: PairwiseOptions::default(),
        policy: Default::default(),
    };
    capture(dir, &n).unwrap();
}

#[test]
fn replay_verify_accepts_a_captured_night_and_flags_an_edited_decision() {
    let d = tempfile::tempdir().unwrap();
    night(d.path());
    let o = dream(&["replay", "verify", p(d.path())]);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));

    let dec = d.path().join("decision.json");
    let edited = std::fs::read_to_string(&dec)
        .unwrap()
        .replace("REJECT", "ACCEPT");
    std::fs::write(&dec, edited).unwrap();
    let o = dream(&["replay", "verify", p(d.path())]);
    assert_eq!(o.status.code(), Some(1));
    assert!(out(&o).contains("verdict"));
}

#[test]
fn replay_verify_on_a_missing_dir_is_an_error() {
    let o = dream(&["replay", "verify", "/nonexistent-run-dir"]);
    assert_eq!(o.status.code(), Some(2));
}

const OK_PATCH: &str =
    "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b\n";
const BAD_PATCH: &str = "diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml\n--- a/.github/workflows/ci.yml\n+++ b/.github/workflows/ci.yml\n@@ -1 +1 @@\n-a\n+b\n";

#[test]
fn guard_check_prints_touched_paths_for_a_clean_patch() {
    let d = tempfile::tempdir().unwrap();
    let patch = write(d.path(), "p.diff", OK_PATCH);
    let o = dream(&["guard", "check", "--patch", p(&patch)]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o).trim(), "src/a.rs");
}

#[test]
fn guard_check_refuses_a_protected_patch_with_exit_1() {
    let d = tempfile::tempdir().unwrap();
    let patch = write(d.path(), "p.diff", BAD_PATCH);
    let o = dream(&["guard", "check", "--patch", p(&patch)]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains(".github/workflows/ci.yml"));
}

#[test]
fn guard_check_applies_the_config_protected_globs() {
    let d = tempfile::tempdir().unwrap();
    let cfg = write(d.path(), "dream.config.json", GOOD_CFG);
    let patch = write(
        d.path(),
        "p.diff",
        &OK_PATCH.replace("src/a.rs", "secret/a.rs"),
    );
    assert!(dream(&["guard", "check", "--patch", p(&patch)])
        .status
        .success());
    let o = dream(&["guard", "check", "--patch", p(&patch), "--config", p(&cfg)]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn guard_check_refuses_a_malformed_patch() {
    let d = tempfile::tempdir().unwrap();
    let patch = write(d.path(), "p.diff", "not a diff at all");
    assert_eq!(
        dream(&["guard", "check", "--patch", p(&patch)])
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn help_lists_every_subcommand() {
    let o = dream(&["--help"]);
    let t = out(&o);
    for c in ["validate", "ledger", "witness", "replay", "guard"] {
        assert!(t.contains(c), "{c}");
    }
}
