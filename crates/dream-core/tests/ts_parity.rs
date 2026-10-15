//! Parity with the reference TypeScript implementation, on shared artefacts.
//!
//! The fixtures under `tests/heldout/ts-parity/` are a real, captured
//! configuration and ledger. Expected values are derived from the reference
//! TypeScript *by reading it*; `derive_expected.py` in the same directory is an
//! independent re-implementation used to compute them (the TypeScript is never
//! executed). Divergences from the reference are asserted explicitly in
//! [`documented_divergences`], not hidden.

use std::path::PathBuf;

use dream_core::compile::{compile, PromptTemplate};
use dream_core::ledger::{
    learning_signals, parse_ledger, parse_prior_fates, validate_ledger, verdict_stats,
    verify_ledger, SignalOptions, CONTRACT_ENFORCE_FROM,
};
use dream_gate::config::{AdrConvention, DreamConfig};
use serde_json::json;

fn fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/heldout/ts-parity")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

// ---------------------------------------------------------------------------
// (a) configuration
// ---------------------------------------------------------------------------

#[test]
fn the_fork_config_is_accepted_as_validate_config_accepts_it() {
    // TS: validateConfig(selfConfig).ok === true, no warnings (every slot has
    // scans); withDefaults(selfConfig).autoMerge === false.
    let cfg = DreamConfig::from_json(fixture("fork-dream.config.json").as_bytes()).unwrap();
    assert_eq!(cfg.slots.len(), 5);
    assert!(cfg.slots.iter().all(|s| !s.scan.is_empty()));
    assert_eq!(cfg.cron, "0 9 * * *");
    assert_eq!(cfg.adr_convention, AdrConvention::FourDigit);
    assert!(!cfg.auto_merge);
    let mut keys: Vec<_> = cfg.bonus_moduli.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["25", "75"]);
    assert_eq!(cfg.evaluator_entrypoints.len(), 2);
    assert!(cfg.evaluator_entrypoints["bench"].required);
    assert_eq!(cfg.ledger_path, "docs/dream-cycle/LEDGER.md");
}

fn base() -> serde_json::Value {
    // The reference `metaharness` fixture of the TS config tests.
    json!({
        "repo": "ruvnet/metaharness", "cron": "0 8 * * *",
        "slots": [
            {"deep": "generator-genome", "scan": ["router", "turn-credit"]},
            {"deep": "flywheel-promotion", "scan": ["evals-verticals", "bench"]}
        ],
        "bonusModuli": {"25": "vertical-packs", "75": "meta-proxy"},
        "buildStep": {"cmd": "npm ci && npm run build", "degradeOnWasmFailure": true},
        "evaluatorEntrypoints": {"bench": "experiments/*/run.mjs"},
        "adrConvention": "3-digit",
        "competitors": ["LangGraph"], "extraDisciplines": ["adr-250-proof-ladder"]
    })
}

fn with(key: &str, v: serde_json::Value) -> Result<DreamConfig, dream_gate::config::ConfigError> {
    let mut b = base();
    b[key] = v;
    DreamConfig::from_json(b.to_string().as_bytes())
}

#[test]
fn the_ts_validate_config_test_cases_agree() {
    assert!(DreamConfig::from_json(base().to_string().as_bytes()).is_ok());
    assert!(with("repo", json!("nope")).is_err());
    assert!(with("cron", json!("nightly")).is_err());
    for cron in ["* * * * *", "*/5 * * * *", "0,30 * * * *", "60 * * * *"] {
        let e = with("cron", json!(cron)).unwrap_err().to_string();
        assert!(e.contains("minimum interval is 1 hour"), "{cron}: {e}");
    }
    assert!(with("cron", json!("15 */2 * * *")).is_ok());
    assert!(with("slots", json!([])).is_err());
    assert!(with("bonusModuli", json!({"x": "y"})).is_err());
    for bad in [
        json!("5-digit"),
        json!(42),
        json!(null),
        json!({"pad": 0, "dir": "docs/adrs"}),
        json!({"pad": 1.5, "dir": "docs/adrs"}),
        json!({"pad": 4, "dir": ""}),
        json!({"pad": 4}),
    ] {
        assert!(with("adrConvention", bad.clone()).is_err(), "{bad}");
    }
    assert!(with("adrConvention", json!("4-digit")).is_ok());
    assert!(with("adrConvention", json!({"pad": 3, "dir": "docs/decisions"})).is_ok());
}

#[test]
fn the_fork_config_compiles_with_the_darwin_pin_intact() {
    // TS: every darwin mention in the compiled prompt is an exact pin, all the
    // same version; the evaluator keeps its wrapper script and sandbox flag.
    // These hold for the mechanism because the evaluator command and the extra
    // disciplines are carried verbatim.
    let cfg = DreamConfig::from_json(fixture("fork-dream.config.json").as_bytes()).unwrap();
    let prompt = compile(&cfg, &cfg.slots[0], 20261001, &[], &PromptTemplate::builtin()).unwrap();
    let mentions: Vec<&str> = prompt
        .match_indices("@metaharness/darwin")
        .map(|(i, _)| {
            let rest = &prompt[i..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ')' || c == '`')
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .collect();
    assert!(!mentions.is_empty());
    let is_pin = |m: &&str| {
        m.strip_prefix("@metaharness/darwin@").is_some_and(|v| {
            let p: Vec<&str> = v.split('.').collect();
            p.len() == 3 && p.iter().all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
        })
    };
    assert!(mentions.iter().all(is_pin), "{mentions:?}");
    let mut uniq = mentions.clone();
    uniq.sort_unstable();
    uniq.dedup();
    assert_eq!(uniq.len(), 1, "{mentions:?}");
    assert!(prompt.contains("./scripts/darwin-entrypoint.sh npx @metaharness/darwin@"));
    assert!(prompt.contains("--sandbox mock"));
}

// ---------------------------------------------------------------------------
// (b) ledger
// ---------------------------------------------------------------------------

/// The one deliberate difference from the TS row contract: dream-gate's
/// finding rules look through the engine's own `VETOED:` prefix, so
/// `VETOED: Given …` is a hypothesis leak here and passes the TS `^given`.
/// Splits the Rust result into (rows both implementations flag, rows only the
/// tightened rules flag); the second list is exactly the VETOED-prefixed rows.
fn split_by_tightening(md: &str, from: &str) -> (Vec<(String, &'static str)>, Vec<(String, &'static str)>) {
    let (mut both, mut tightened) = (vec![], vec![]);
    for line in md.lines() {
        let vetoed = line
            .trim()
            .trim_start_matches('|')
            .split('|')
            .nth(2)
            .is_some_and(|f| f.trim().starts_with("VETOED:"));
        for v in validate_ledger(line, from) {
            let item = (v.date, v.rule);
            let leak_or_pointer = matches!(item.1, "finding-hypothesis-leak" | "finding-pointer");
            if vetoed && leak_or_pointer {
                tightened.push(item);
            } else {
                both.push(item);
            }
        }
    }
    (both, tightened)
}

#[test]
fn the_fork_ledger_complies_from_the_cutoff_as_validate_ledger_reports() {
    // TS rowContract.test.ts: the real ledger complies from the cutoff, i.e.
    // validateLedger(text, "2026-09-06") === [].
    let md = fixture("fork-LEDGER.md");
    assert_eq!(CONTRACT_ENFORCE_FROM, "2026-09-06");
    let (both, tightened) = split_by_tightening(&md, "2026-09-06");
    assert_eq!(both, vec![], "everything the TS flags, and nothing it does not, outside the tightening");
    // DIVERGENCE (deliberate, in dream-gate): four rows from the cutoff on
    // read `VETOED: Given …`, a frozen-hypothesis leak the TS `^given` rule
    // cannot see behind the prefix. This crate flags them.
    let want: Vec<(String, &str)> = ["2026-09-09", "2026-09-27", "2026-09-29", "2026-10-01"]
        .iter()
        .map(|d| (d.to_string(), "finding-hypothesis-leak"))
        .collect();
    assert_eq!(tightened, want);
    // And the full-ledger result is exactly the union.
    assert_eq!(validate_ledger(&md, "2026-09-06").len(), 4);
}

#[test]
fn the_fork_ledger_violations_before_the_cutoff_match_the_reference() {
    // Derived with derive_expected.py (validateLedger(text, "0000-00-00")):
    // 35 violations, in file order and, within a row, in rule order.
    let expected: &[(&str, &str)] = &[
        ("2026-08-13", "finding-too-long"),
        ("2026-08-13", "fates-grammar"),
        ("2026-08-14", "fates-grammar"),
        ("2026-08-14", "finding-too-long"),
        ("2026-08-14", "fates-grammar"),
        ("2026-08-15", "finding-too-long"),
        ("2026-08-15", "fates-grammar"),
        ("2026-08-15", "finding-pointer"),
        ("2026-08-15", "finding-hypothesis-leak"),
        ("2026-08-15", "finding-hypothesis-leak"),
        ("2026-08-15", "finding-pointer"),
        ("2026-08-15", "finding-hypothesis-leak"),
        ("2026-08-15", "accept-without-pr"),
        ("2026-08-15", "finding-too-long"),
        ("2026-08-15", "fates-grammar"),
        ("2026-08-16", "finding-hypothesis-leak"),
        ("2026-08-16", "accept-without-pr"),
        ("2026-08-16", "finding-hypothesis-leak"),
        ("2026-08-16", "accept-without-pr"),
        ("2026-08-16", "finding-too-long"),
        ("2026-08-16", "fates-grammar"),
        ("2026-08-17", "finding-hypothesis-leak"),
        ("2026-08-17", "accept-without-pr"),
        ("2026-08-17", "finding-hypothesis-leak"),
        ("2026-08-17", "accept-without-pr"),
        ("2026-08-28", "finding-too-long"),
        ("2026-09-01", "finding-pointer"),
        ("2026-09-02", "finding-hypothesis-leak"),
        ("2026-09-02", "accept-without-pr"),
        ("2026-09-03", "finding-hypothesis-leak"),
        ("2026-09-03", "accept-without-pr"),
        ("2026-09-04", "finding-hypothesis-leak"),
        ("2026-09-04", "accept-without-pr"),
        ("2026-09-05", "finding-hypothesis-leak"),
        ("2026-09-05", "accept-without-pr"),
    ];
    // Outside the VETOED-prefix tightening (see `split_by_tightening`), the
    // violations are exactly the reference's.
    let (got, _) = split_by_tightening(&fixture("fork-LEDGER.md"), "0000-00-00");
    let want: Vec<(String, &str)> = expected.iter().map(|(d, r)| (d.to_string(), *r)).collect();
    assert_eq!(got.len(), want.len(), "{got:#?}");
    assert_eq!(got, want);
}

#[test]
fn verify_ledger_agrees_on_the_fork_ledger() {
    // TS verifyLedger: ok, no errors, 46 data rows.
    let v = verify_ledger(&fixture("fork-LEDGER.md"));
    assert!(v.ok, "{:?}", v.errors);
    assert!(v.errors.is_empty());
    assert_eq!(v.row_count, 46);
}

#[test]
fn learning_signals_and_stats_agree_on_the_fork_ledger() {
    let rows = parse_ledger(&fixture("fork-LEDGER.md")).rows;
    assert_eq!(rows.len(), 46);
    // TS parsePriorFates: only the explicit #N:FATE tokens, latest wins.
    let fates = parse_prior_fates(&rows);
    let mut f: Vec<_> = fates.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    f.sort();
    assert_eq!(f, [("11", "MERGED"), ("15", "MERGED"), ("7", "MERGED"), ("9", "MERGED")]);
    // TS learningSignals(rows) with default options (window 14).
    let s = learning_signals(&rows, &SignalOptions::default());
    assert!(s.zero_merge_streak, "no PR in the last 14 rows is a merged number");
    assert_eq!(
        s.duplicate_directions,
        [
            "inconclusive see report",
            "given the darwin evaluator at commit",
            "bench red on the 1003 row"
        ]
    );
    assert!(!s.low_score_streak && !s.blocked_eval_streak);
    assert_eq!(s.nights_considered, 14);
    // TS verdictStats.
    let st = verdict_stats(&rows);
    assert_eq!((st.accept, st.reject, st.inconclusive, st.other), (23, 3, 12, 8));
}

#[test]
fn documented_divergences() {
    let md = fixture("fork-LEDGER.md");
    // 1. Width. The TS parser warns on every row that is not 10 cells wide, so
    //    its 15 twelve-column rows each warn ("row has 12 columns, expected
    //    10"). This crate writes twelve columns and still reads the legacy ten,
    //    so neither width warns.
    let p = parse_ledger(&md);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let wide = p.rows.iter().filter(|r| r.reviewer.is_empty() && r.review_minutes.is_empty()).count();
    assert_eq!(wide, 46, "no fork row carries a reviewer");
    // 2. A violation here is (date, rule); the TS adds a free-text `detail`.
    // 3. Witness: the TS accepts a commit of 7-40 hex characters, this crate
    //    7-64 (a sha256-format repository has 64).
    let long = "a".repeat(64);
    assert!(dream_gate::witness::witness("r", &long).is_ok());
    // 4. Config: an absent `cron` is accepted here (no opinion), required by
    //    the TS; a present one is held to the same rule (see dream-gate's
    //    config_ts_parity tests).
    let no_cron = json!({"repo": "o/r", "slots": [{"deep": "a", "scan": []}]});
    assert!(DreamConfig::from_json(no_cron.to_string().as_bytes()).is_ok());
}

// ---------------------------------------------------------------------------
// (c) witness
// ---------------------------------------------------------------------------

const REPORT: &str = "# Dream Cycle 2026-08-13\nverdict: INCONCLUSIVE\n";
const COMMIT: &str = "68402755f017e0df5f493c6ee608218420540d17";

#[test]
fn the_ts_witness_test_vector() {
    // Computed with `python3 -I`: h = sha256(report).hex();
    // witness = sha256((h + commit).encode()).hex().
    let report_hash = "5e921d699372e416cb1259ab8f71c7323cb1d984fac14ce3c3c436ec6a8ca639";
    let witness = "25031322a1eb352f88530ff9a5a8f97cc2bb3cc2a29009b85b4a78018e49b39e";
    assert_eq!(dream_gate::witness::hash_report(REPORT), report_hash);
    assert_eq!(dream_gate::witness::witness(REPORT, COMMIT).unwrap(), witness);
    assert!(dream_gate::witness::verify(REPORT, COMMIT, witness).unwrap());
    // Case-insensitive and trimmed, as TS verify().
    assert!(dream_gate::witness::verify(REPORT, COMMIT, &format!("  {}  ", witness.to_uppercase())).unwrap());
    // Commit is lowercased and trimmed (TS normalizeCommit).
    assert_eq!(
        dream_gate::witness::witness(REPORT, &format!("  {}  ", COMMIT.to_uppercase())).unwrap(),
        witness
    );
    // A single-byte mutation, or another commit, does not verify.
    assert!(!dream_gate::witness::verify("# Dream Cycle 2026-08-13\nverdict: INCONCLUSIVF\n", COMMIT, witness).unwrap());
    assert!(!dream_gate::witness::verify(REPORT, &"a".repeat(40), witness).unwrap());
    // Bad commits are errors, not quiet falses.
    assert!(dream_gate::witness::witness(REPORT, "nothex!!").is_err());
    // Second published vector from the TS tests' style: short commit, 'x' body.
    assert_eq!(
        dream_gate::witness::witness("x", "68402755f017e0df").unwrap(),
        "1ff0eb78f684404aef72b98f87247edbe4b819e23aa1860a71e729ecf2a6fec1"
    );
    assert_eq!(
        dream_gate::witness::witness("genuine report bytes", COMMIT).unwrap(),
        "0e6ccd4ead22621af3b3569cfdb7466d54d0fcfa3c60d8330f09db6faa006c6b"
    );
}
