//! Numeric and behavioural tests. Reference values were computed independently
//! with exact integer arithmetic (python3 `fractions`/`math.comb`) for the
//! binomial tail and with 40-digit `decimal` for the Wilson bound; the
//! small-n values (11/1024 = 0.0107 for 9 of 10; Wilson 8/10 at z=1.96 = 0.4902)
//! also match standard published sign-test and Wilson-interval tables.
#![allow(clippy::excessive_precision)]
use pairwise_gate::*;

fn s(x: f64) -> Outcome {
    Outcome::score(x).unwrap()
}
fn pr(b: f64, c: f64) -> Pairing {
    Pairing::new("t", s(b), s(c))
}
fn many(wins: usize, losses: usize, ties: usize) -> Vec<Pairing> {
    let mut v = Vec::new();
    v.extend((0..wins).map(|_| pr(0.0, 1.0)));
    v.extend((0..losses).map(|_| pr(1.0, 0.0)));
    v.extend((0..ties).map(|_| pr(0.5, 0.5)));
    v
}
fn close(a: f64, b: f64, rel: f64) {
    assert!(((a - b) / b).abs() < rel, "{a} vs {b}");
}

#[test]
fn nan_and_inf_rejected() {
    assert!(Outcome::score(f64::NAN).is_err());
    assert!(Outcome::score(f64::INFINITY).is_err());
    assert!(Outcome::score(f64::NEG_INFINITY).is_err());
}

#[test]
fn tally_excludes_faults() {
    let v = vec![
        pr(0.0, 1.0),
        pr(1.0, 0.0),
        pr(1.0, 1.0),
        Pairing::new("a", Outcome::fault("down"), s(1.0)),
        Pairing::new("b", s(1.0), Outcome::fault("timeout")),
        Pairing::new("c", Outcome::fault("x"), Outcome::fault("y")),
    ];
    let t = Tally::from_pairings(&v);
    assert_eq!((t.wins, t.losses, t.ties, t.faults), (1, 1, 1, 3));
    assert_eq!(t.comparable(), 3);
    let d = Margin::new(0).decide(&Evidence::new(&v).with_guard(Guard::new(1, 1.0).unwrap()));
    assert_eq!(d.fault_reports.len(), 4);
    assert_eq!(d.metrics.faults, 3);
}

#[test]
fn margin_semantics() {
    let m = Margin::new(2);
    assert_eq!(
        m.decide(&Evidence::new(&many(3, 0, 0))).verdict,
        Verdict::Select
    );
    assert_eq!(
        m.decide(&Evidence::new(&many(2, 0, 0))).verdict,
        Verdict::Reject
    );
    assert_eq!(
        m.decide(&Evidence::new(&many(5, 3, 0))).verdict,
        Verdict::Reject
    );
    assert_eq!(
        m.decide(&Evidence::new(&many(6, 3, 9))).verdict,
        Verdict::Select
    );
}

#[test]
fn empty_is_inconclusive_not_panic() {
    let d = Margin::new(0).decide(&Evidence::new(&[]));
    assert_eq!(d.verdict, Verdict::Inconclusive);
    let d = SignTest::new(0.05).unwrap().decide(&Evidence::new(&[]));
    assert_eq!(d.verdict, Verdict::Inconclusive);
}

#[test]
fn fault_heavy_is_inconclusive() {
    let mut v = many(10, 0, 0);
    v.extend((0..11).map(|_| Pairing::new("f", Outcome::fault("down"), s(1.0))));
    let d = Margin::new(0).decide(&Evidence::new(&v));
    assert_eq!(d.verdict, Verdict::Inconclusive);
    // exactly at the limit is not over it
    let mut v = many(10, 0, 0);
    v.extend((0..10).map(|_| Pairing::new("f", Outcome::fault("down"), s(1.0))));
    assert_eq!(
        Margin::new(0).decide(&Evidence::new(&v)).verdict,
        Verdict::Select
    );
}

#[test]
fn min_comparable_configurable() {
    let g = Guard::new(5, 0.5).unwrap();
    let d = Margin::new(0).decide(&Evidence::new(&many(4, 0, 0)).with_guard(g));
    assert_eq!(d.verdict, Verdict::Inconclusive);
}

#[test]
fn guard_validation() {
    assert!(Guard::new(1, f64::NAN).is_err());
    assert!(Guard::new(1, 1.5).is_err());
    assert!(Guard::new(1, -0.1).is_err());
}

#[test]
fn floor_policy() {
    let f = Floor::new(0.5).unwrap();
    let v = vec![pr(0.0, 0.5), pr(0.0, 0.9)];
    assert_eq!(f.decide(&Evidence::new(&v)).verdict, Verdict::Select);
    let v = vec![pr(0.9, 0.49), pr(0.0, 0.9)];
    assert_eq!(f.decide(&Evidence::new(&v)).verdict, Verdict::Reject);
    // candidate-only: a baseline fault does not hide a scored candidate
    let v = vec![Pairing::new("a", Outcome::fault("x"), s(0.7))];
    assert_eq!(f.decide(&Evidence::new(&v)).verdict, Verdict::Select);
    assert!(Floor::new(f64::NAN).is_err());
}

#[test]
fn sign_test_reference_values() {
    // exact: sum_{i>=k} C(n,i) / 2^n
    let cases: &[(usize, usize, f64)] = &[
        (10, 9, 1.07421875000000000e-02),
        (10, 8, 5.46875000000000000e-02),
        (10, 10, 9.765625e-04),
        (20, 15, 2.06947326660156250e-02),
        (20, 10, 5.88098526000976562e-01),
        (30, 21, 2.13869726285338402e-02),
        (100, 60, 2.84439668204903955e-02),
        (100, 50, 5.39794618693589356e-01),
        (1000, 550, 8.65268042488158808e-04),
        (10000, 5100, 2.32927638524736934e-02),
        (10000, 5050, 1.61087099897655989e-01),
        (10000, 5200, 3.29675779933622084e-05),
    ];
    for &(n, k, want) in cases {
        close(sign_test_p_value(k, n), want, 1e-9);
    }
    assert_eq!(sign_test_p_value(0, 0), 1.0);
    assert_eq!(sign_test_p_value(0, 7), 1.0);
    // 2^-10000 underflows f64: must be a clean 0, not NaN
    assert_eq!(sign_test_p_value(10000, 10000), 0.0);
    assert!(sign_test_p_value(5000, 10000).is_finite());
}

#[test]
fn sign_test_policy() {
    let p = SignTest::new(0.05).unwrap();
    let d = p.decide(&Evidence::new(&many(9, 1, 5)));
    assert_eq!(d.verdict, Verdict::Select);
    close(d.metrics.p_value.unwrap(), 1.0742187500e-02, 1e-9);
    // 8 of 10: p = 0.0547 > 0.05
    assert_eq!(
        p.decide(&Evidence::new(&many(8, 2, 0))).verdict,
        Verdict::Reject
    );
    // all ties: comparable but no evidence
    assert_eq!(
        p.decide(&Evidence::new(&many(0, 0, 4))).verdict,
        Verdict::Reject
    );
    assert!(SignTest::new(0.0).is_err());
    assert!(SignTest::new(f64::NAN).is_err());
}

#[test]
fn wilson_reference_values() {
    let cases: &[(usize, usize, f64, f64)] = &[
        (8, 10, 1.96, 0.4901568467207234),
        (9, 10, 1.96, 0.5958436145024279),
        (10, 10, 1.96, 0.7224598312333834),
        (15, 20, 1.645, 0.5677811765918621),
        (60, 100, 1.96, 0.5020007846184024),
        (55, 100, 1.96, 0.4524442703164345),
        (5100, 10000, 1.96, 0.5002000007840062),
        (81, 100, 1.281551565545, 0.7548724165205521),
    ];
    for &(k, n, z, want) in cases {
        close(wilson_lower(k, n, z), want, 1e-9);
    }
    assert!(wilson_lower(0, 10, 1.96).abs() < 1e-12);
    assert_eq!(wilson_lower(0, 0, 1.96), 0.0);
}

#[test]
fn wilson_policy() {
    let p = WilsonLower::new(1.96).unwrap();
    assert_eq!(
        p.decide(&Evidence::new(&many(60, 40, 3))).verdict,
        Verdict::Select
    );
    assert_eq!(
        p.decide(&Evidence::new(&many(55, 45, 0))).verdict,
        Verdict::Reject
    );
    let d = p.decide(&Evidence::new(&many(9, 1, 0)));
    assert!(d.metrics.wilson_lower.unwrap() > 0.5);
    assert!(WilsonLower::new(f64::NAN).is_err());
    assert!(WilsonLower::new(-1.0).is_err());
}

#[test]
fn no_regression_policy() {
    let p = NoRegression::new(0.5).unwrap();
    // many wins, but one pass->fail: reject
    let mut v = many(20, 0, 0);
    v.push(pr(0.8, 0.2));
    let d = p.decide(&Evidence::new(&v));
    assert_eq!(d.verdict, Verdict::Reject);
    assert_eq!(d.metrics.regressions, Some(1));
    // fail->fail and fail->pass are not regressions
    let v = vec![pr(0.1, 0.0), pr(0.1, 0.9), pr(0.5, 0.5)];
    assert_eq!(p.decide(&Evidence::new(&v)).verdict, Verdict::Select);
    // a fault on the pass side is not a regression
    let v = vec![
        Pairing::new("x", s(1.0), Outcome::fault("down")),
        pr(1.0, 1.0),
    ];
    assert_eq!(
        p.decide(&Evidence::new(&v).with_guard(Guard::new(1, 0.5).unwrap()))
            .verdict,
        Verdict::Select
    );
}

#[test]
fn train_dev_acceptance_equation() {
    let td = TrainDev::new(
        Box::new(Margin::new(1)),
        Box::new(NoRegression::new(0.5).unwrap()),
    );
    let train = many(5, 0, 0);
    let good_dev = vec![pr(1.0, 1.0)];
    let bad_dev = vec![pr(1.0, 0.0)];
    let sel = td.decide(&Evidence::new(&train).with_heldout(&good_dev));
    assert_eq!(sel.verdict, Verdict::Select);
    assert_eq!(sel.parts.len(), 2);
    assert_eq!(
        td.decide(&Evidence::new(&train).with_heldout(&bad_dev))
            .verdict,
        Verdict::Reject
    );
    // train fails
    let weak = many(1, 0, 0);
    assert_eq!(
        td.decide(&Evidence::new(&weak).with_heldout(&good_dev))
            .verdict,
        Verdict::Reject
    );
    // fault-heavy dev
    let faulty: Vec<Pairing> = (0..3)
        .map(|_| Pairing::new("f", Outcome::fault("down"), s(1.0)))
        .chain(std::iter::once(pr(1.0, 1.0)))
        .collect();
    assert_eq!(
        td.decide(&Evidence::new(&train).with_heldout(&faulty))
            .verdict,
        Verdict::Inconclusive
    );
    // missing dev is not a pass
    assert_eq!(
        td.decide(&Evidence::new(&train)).verdict,
        Verdict::Inconclusive
    );
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip_and_validation() {
    let m: Margin = serde_json::from_str(r#"{"min_win_margin":3}"#).unwrap();
    assert_eq!(m, Margin::new(3));
    assert!(serde_json::from_str::<WilsonLower>(r#"{"z":-2.0}"#).is_err());
    let w: WilsonLower = serde_json::from_str(r#"{"z":1.96}"#).unwrap();
    assert_eq!(w, WilsonLower::new(1.96).unwrap());
    let o = Outcome::fault("down");
    let j = serde_json::to_string(&o).unwrap();
    assert_eq!(serde_json::from_str::<Outcome>(&j).unwrap(), o);
    assert!(serde_json::from_str::<Outcome>(r#"{"Score":null}"#).is_err());
    let g: Guard = serde_json::from_str(r#"{"min_comparable":2,"max_fault_share":0.25}"#).unwrap();
    assert_eq!(g, Guard::new(2, 0.25).unwrap());
    assert!(serde_json::from_str::<Guard>(r#"{"min_comparable":2,"max_fault_share":2}"#).is_err());
    let d = Margin::new(0).decide(&Evidence::new(&many(2, 0, 0)));
    let j = serde_json::to_string(&d).unwrap();
    assert_eq!(serde_json::from_str::<Decision>(&j).unwrap(), d);
}
