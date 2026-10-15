//! Accept/reject parity with the reference TypeScript `validateConfig`
//! (`packages/compile/src/config.ts` of the original dream-machine).
//!
//! Each case is a config the TS validator accepts or rejects; the Rust
//! validator must agree. Known, deliberate divergences are listed in the
//! `divergences` test.

use dream_gate::config::{AdrConvention, DreamConfig, TestFormat};
use serde_json::{json, Value};

fn load(extra: Value) -> Result<DreamConfig, dream_gate::config::ConfigError> {
    let mut v = json!({
        "repo": "owner/name",
        "cron": "0 8 * * *",
        "slots": [{"deep": "a", "scan": ["x", "y"]}]
    });
    v.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    DreamConfig::from_json(v.to_string().as_bytes())
}

#[test]
fn repo_must_be_owner_slash_name_with_word_dot_dash_chars() {
    for ok in ["a/b", "Dream-Lab.AI/dream_engine.rs", "0/9"] {
        assert!(load(json!({"repo": ok})).is_ok(), "{ok}");
    }
    for bad in [
        "", "noslash", "a/b/c", "a/", "/b", "a b/c", "a/b c", "o/r\n", "ö/r", "a/b;rm",
    ] {
        assert!(load(json!({"repo": bad})).is_err(), "{bad:?}");
    }
}

#[test]
fn cron_needs_five_fields_and_a_single_fixed_minute() {
    for ok in ["0 8 * * *", "59 23 * * 1", "5 */6 * * *", " 30 3 * * * "] {
        assert!(load(json!({"cron": ok})).is_ok(), "{ok:?}");
    }
    for bad in [
        "* * * * *",
        "*/5 * * * *",
        "0,30 * * * *",
        "0-10 * * * *",
        "60 8 * * *",
        "00 8 * * *",
        "0 8 * *",
        "0 8 * * * *",
        "a b c",
    ] {
        assert!(load(json!({"cron": bad})).is_err(), "{bad:?}");
    }
}

#[test]
fn bonus_moduli_keys_must_be_integers() {
    assert!(load(json!({"bonusModuli": {"25": "x", "75": "y"}})).is_ok());
    for bad in ["abc", "2.5", "-1", "", "1e3"] {
        assert!(load(json!({"bonusModuli": {bad: "x"}})).is_err(), "{bad:?}");
    }
}

#[test]
fn adr_convention_accepts_the_two_strings_and_a_pad_dir_object() {
    assert_eq!(
        load(json!({"adrConvention": "3-digit"}))
            .unwrap()
            .adr_convention,
        AdrConvention::ThreeDigit
    );
    assert_eq!(
        load(json!({"adrConvention": "4-digit"}))
            .unwrap()
            .adr_convention,
        AdrConvention::FourDigit
    );
    let c = load(json!({"adrConvention": {"pad": 5, "dir": "docs/decisions"}})).unwrap();
    assert_eq!(
        c.adr_convention,
        AdrConvention::Custom {
            pad: 5,
            dir: "docs/decisions".into()
        }
    );
    assert_eq!(
        load(json!({})).unwrap().adr_convention,
        AdrConvention::Unset
    );
    for bad in [
        json!("5-digit"),
        json!(""),
        json!(null),
        json!(7),
        json!({"pad": 0, "dir": "d"}),
        json!({"pad": -1, "dir": "d"}),
        json!({"pad": 1.5, "dir": "d"}),
        json!({"pad": 3, "dir": "   "}),
        json!({"pad": 3}),
        json!({"dir": "d"}),
    ] {
        assert!(
            load(json!({"adrConvention": bad.clone()})).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn adr_convention_round_trips_through_serde() {
    let c = load(json!({"adrConvention": {"pad": 2, "dir": "adr"}})).unwrap();
    let back: DreamConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(back.adr_convention, c.adr_convention);
    let unset = load(json!({})).unwrap();
    let back: DreamConfig = serde_json::from_str(&serde_json::to_string(&unset).unwrap()).unwrap();
    assert_eq!(back.adr_convention, AdrConvention::Unset);
}

#[test]
fn slots_need_a_deep_but_empty_scan_is_only_a_warning() {
    assert!(load(json!({"slots": [{"deep": "a", "scan": []}]})).is_ok());
    assert!(load(json!({"slots": []})).is_err());
    assert!(load(json!({"slots": [{"deep": "", "scan": ["x"]}]})).is_err());
}

#[test]
fn test_format_is_an_optional_evaluator_field() {
    let c = load(json!({"evaluatorEntrypoints": {
        "unit": {"cmd": "cargo test 2>&1", "testFormat": "libtest"},
        "ui": {"cmd": "npx vitest run --reporter=json", "testFormat": "vitest-json"},
        "j": {"cmd": "mvn test", "testFormat": "junit"},
        "plain": "make check"
    }}))
    .unwrap();
    assert_eq!(
        c.evaluator_entrypoints["unit"].test_format,
        Some(TestFormat::Libtest)
    );
    assert_eq!(
        c.evaluator_entrypoints["ui"].test_format,
        Some(TestFormat::VitestJson)
    );
    assert_eq!(
        c.evaluator_entrypoints["j"].test_format,
        Some(TestFormat::Junit)
    );
    assert_eq!(c.evaluator_entrypoints["plain"].test_format, None);
    assert!(
        load(json!({"evaluatorEntrypoints": {"x": {"cmd": "c", "testFormat": "tap"}}})).is_err()
    );
}

#[test]
fn divergences() {
    // Documented, deliberate: the Rust config has no opinion on an absent
    // cron (the TS validator requires one); a present cron is held to the
    // same rule.
    let no_cron = json!({"repo": "o/r", "slots": [{"deep": "a", "scan": []}]});
    assert!(DreamConfig::from_json(no_cron.to_string().as_bytes()).is_ok());
}
