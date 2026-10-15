//! Configs written for earlier engines took any string for `adrConvention`.
//! A free-form numbering template must keep loading.

use dream_gate::config::AdrConvention;

fn parse(s: &str) -> Result<AdrConvention, serde_json::Error> {
    serde_json::from_str(s)
}

#[test]
fn legacy_template_loads_and_sets_pad_from_the_n_run() {
    let c = parse("\"ADR-OM-NNN\"").expect("legacy template must load");
    assert_eq!(c.pad(), 3);
    assert_eq!(parse("\"ADR-NNNNN\"").unwrap().pad(), 5);
}

#[test]
fn template_round_trips_as_the_same_string() {
    let c = parse("\"ADR-OM-NNN\"").unwrap();
    assert_eq!(serde_json::to_string(&c).unwrap(), "\"ADR-OM-NNN\"");
}

#[test]
fn text_with_no_numbering_run_is_still_rejected() {
    assert!(parse("\"5-digit\"").is_err());
    assert!(parse("\"whatever\"").is_err());
    assert!(parse("\"\"").is_err());
}

#[test]
fn longest_run_wins() {
    assert_eq!(parse("\"N-ADR-NNNN\"").unwrap().pad(), 4);
}

#[test]
fn named_forms_are_unchanged() {
    assert_eq!(parse("\"3-digit\"").unwrap(), AdrConvention::ThreeDigit);
    assert_eq!(parse("\"4-digit\"").unwrap(), AdrConvention::FourDigit);
}
