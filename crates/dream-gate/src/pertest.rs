//! Optional per-test evidence for the gate.
//!
//! An evaluator normally yields one typed receipt (pass/fail/…). When the
//! caller can also supply the outcome of every individual test — on the
//! baseline tree and on the candidate tree — the gate can see what a single
//! exit code hides: a test that *used to pass and now fails*, or a *new kind
//! of failure*, even when the suite was already red.
//!
//! This module defines the neutral input type, [`TestOutcomes`]. Parsing a
//! concrete format (libtest, JUnit, vitest JSON …) is the caller's job.
//!
//! The evidence is **additive**: it can only add vetoes. An evaluator with no
//! [`TestOutcomes`] is judged exactly as before.
//!
//! * A per-test *fault* ([`FaultKind`]: missing, silent, blocked, timed out)
//!   is an infrastructure fact, never a loss: it raises a harness veto
//!   (`BLOCKED-ENV`) and is excluded from every tally.
//! * A failure whose [`failure_fingerprint`] is absent from the baseline is a
//!   **new** failure and raises an evidence veto (`REJECT`).
//! * With at least [`PairwiseOptions::min_cases`] comparable cases the
//!   [`pairwise_gate`] policies apply (`NoRegression`, optional `Floor`,
//!   `Margin`, `SignTest`, `WilsonLower`). With fewer, the evaluator counts as
//!   **one** case (passing iff every test passes) and only `NoRegression` and
//!   `Floor` apply — a statistical test on a handful of cases proves nothing.
//!
//! ```
//! use dream_gate::pertest::{TestCase, TestOutcomes, TestStatus};
//! let t = TestOutcomes {
//!     baseline: vec![TestCase::new("a", TestStatus::Passed)],
//!     candidate: vec![TestCase::new("a", TestStatus::failed("boom"))],
//! };
//! assert_eq!(t.baseline.len(), 1);
//! ```

use std::collections::BTreeMap;

use failure_fingerprint::{advance, Manifest, Observation, Stage};
use pairwise_gate::{
    Evidence, Floor, Margin, NoRegression, Outcome, Pairing, Policy, SignTest, Verdict, WilsonLower,
};
use serde::{Deserialize, Serialize};

use crate::gate::Veto;

/// Why a single test produced no usable result. Never counted as a loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FaultKind {
    /// The test was expected but is absent from the results.
    Missing,
    /// The test ran but produced no output at all.
    Silent,
    /// The test could not be run (environment, transport).
    Blocked,
    /// The test exceeded its time budget.
    TimedOut,
}

impl FaultKind {
    fn label(self) -> &'static str {
        match self {
            FaultKind::Missing => "missing",
            FaultKind::Silent => "silent",
            FaultKind::Blocked => "blocked",
            FaultKind::TimedOut => "timed-out",
        }
    }
}

/// The result of one test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum TestStatus {
    /// The test passed.
    Passed,
    /// The test ran and failed; `cause` is the failure message.
    Failed {
        /// Failure message (normalised before fingerprinting).
        cause: String,
    },
    /// The test produced no usable result.
    Fault {
        /// What kind of fault.
        fault: FaultKind,
    },
}

impl TestStatus {
    /// A failed status with the given cause.
    pub fn failed(cause: impl Into<String>) -> Self {
        TestStatus::Failed {
            cause: cause.into(),
        }
    }
    /// A fault status.
    pub fn fault(kind: FaultKind) -> Self {
        TestStatus::Fault { fault: kind }
    }
}

/// One named test and its result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCase {
    /// Stable test identifier (for example a fully qualified test path).
    pub name: String,
    /// Its result.
    pub status: TestStatus,
}

impl TestCase {
    /// Build a case.
    pub fn new(name: impl Into<String>, status: TestStatus) -> Self {
        Self {
            name: name.into(),
            status,
        }
    }
}

/// Per-test results for one evaluator on both trees.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestOutcomes {
    /// Results on the unmodified baseline tree.
    pub baseline: Vec<TestCase>,
    /// Results on the candidate tree.
    pub candidate: Vec<TestCase>,
}

/// Which pairwise policies apply, and their parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct PairwiseOptions {
    /// Score at or above which a case counts as passed (cases score 1.0 / 0.0).
    pub pass_threshold: f64,
    /// Candidate-only floor: every candidate score must reach it.
    pub floor: Option<f64>,
    /// Require `wins - losses > margin` (only with `min_cases` cases).
    pub min_win_margin: Option<u32>,
    /// One-sided sign-test significance (only with `min_cases` cases).
    pub sign_alpha: Option<f64>,
    /// Wilson lower-bound z value (only with `min_cases` cases).
    pub wilson_z: Option<f64>,
    /// Comparable cases needed before the statistical policies apply.
    pub min_cases: usize,
}

impl Default for PairwiseOptions {
    /// `NoRegression` only; statistical policies are opt-in.
    fn default() -> Self {
        Self {
            pass_threshold: 0.5,
            floor: None,
            min_win_margin: None,
            sign_alpha: None,
            wilson_z: None,
            min_cases: 20,
        }
    }
}

fn score(s: &TestStatus) -> Outcome {
    match s {
        TestStatus::Passed => Outcome::score(1.0).expect("1.0 is finite"),
        TestStatus::Failed { .. } => Outcome::score(0.0).expect("0.0 is finite"),
        TestStatus::Fault { fault } => Outcome::fault(fault.label()),
    }
}

fn all_pass(cases: &[&TestCase]) -> Option<bool> {
    if cases
        .iter()
        .any(|c| matches!(c.status, TestStatus::Fault { .. }))
    {
        return None;
    }
    Some(cases.iter().all(|c| matches!(c.status, TestStatus::Passed)))
}

/// Vetoes for every required evaluator that has per-test evidence.
///
/// `tests` maps evaluator name → outcomes; entries for evaluators not in
/// `required` are ignored. Output order is deterministic (by evaluator name).
pub(crate) fn vetoes(
    required: &[&str],
    tests: &BTreeMap<String, TestOutcomes>,
    opts: &PairwiseOptions,
) -> Vec<Veto> {
    if tests.is_empty() {
        return Vec::new();
    }
    // Invalid options must never silently drop a policy and let a candidate
    // through: they are a configuration fault, so the run is blocked.
    if let Err(e) = build_policies(opts, true) {
        return vec![Veto::harness(
            "pairwise-options",
            format!("invalid pairwise options, per-test evidence cannot be judged: {e}"),
        )];
    }
    let mut out = Vec::new();
    for (name, t) in tests {
        if !required.contains(&name.as_str()) {
            continue;
        }
        out.extend(evaluator_vetoes(name, t, opts));
    }
    out
}

fn evaluator_vetoes(name: &str, t: &TestOutcomes, opts: &PairwiseOptions) -> Vec<Veto> {
    let mut out = Vec::new();
    let base: BTreeMap<&str, &TestCase> = t.baseline.iter().map(|c| (c.name.as_str(), c)).collect();
    let cand: BTreeMap<&str, &TestCase> =
        t.candidate.iter().map(|c| (c.name.as_str(), c)).collect();

    // 1. Faults, either side, and baseline tests the candidate run dropped.
    let mut faults: Vec<String> = Vec::new();
    for (tn, c) in &base {
        if let TestStatus::Fault { fault } = c.status {
            faults.push(format!("baseline {tn}: {}", fault.label()));
        }
    }
    for (tn, c) in &cand {
        if let TestStatus::Fault { fault } = c.status {
            faults.push(format!("candidate {tn}: {}", fault.label()));
        }
    }
    for tn in base.keys() {
        if !cand.contains_key(tn) {
            faults.push(format!("candidate {tn}: missing"));
        }
    }
    if !faults.is_empty() {
        out.push(Veto::harness(
            name,
            format!(
                "{} per-test fault(s), not counted as losses: {}",
                faults.len(),
                summarise(&faults)
            ),
        ));
    }

    // 2. New failure fingerprints against the baseline.
    let observe = |m: &BTreeMap<&str, &TestCase>| -> Vec<Observation> {
        m.iter()
            .filter_map(|(tn, c)| match &c.status {
                TestStatus::Failed { cause } => {
                    Some(Observation::new(*tn, Stage::new(name), cause.as_str()))
                }
                _ => None,
            })
            .collect()
    };
    let (base_manifest, _) = advance(&Manifest::new(0), &observe(&base), 1);
    let (_, class) = advance(&base_manifest, &observe(&cand), 2);
    if !class.new.is_empty() {
        let items: Vec<String> = class
            .new
            .iter()
            .map(|r| format!("{} [{}]", r.task, r.fingerprint))
            .collect();
        out.push(Veto::evidence(
            name,
            format!(
                "{} new failure fingerprint(s) absent from the baseline: {}",
                items.len(),
                summarise(&items)
            ),
        ));
    }

    // 3. Pairwise policies over the comparable cases.
    let comparable: Vec<Pairing> = base
        .iter()
        .filter_map(|(tn, b)| {
            cand.get(tn)
                .map(|c| Pairing::new(*tn, score(&b.status), score(&c.status)))
        })
        .collect();
    let (pairings, statistical) = if comparable.len() >= opts.min_cases {
        (comparable, true)
    } else {
        // Too few cases for statistics: the evaluator is one case.
        let b: Vec<&TestCase> = base.values().copied().collect();
        let c: Vec<&TestCase> = cand.values().copied().collect();
        let one = match (all_pass(&b), all_pass(&c)) {
            (Some(bp), Some(cp)) if !b.is_empty() => vec![Pairing::new(
                name,
                Outcome::score(f64::from(u8::from(bp))).expect("finite"),
                Outcome::score(f64::from(u8::from(cp))).expect("finite"),
            )],
            _ => Vec::new(),
        };
        (one, false)
    };
    if pairings.is_empty() {
        return out;
    }
    let Ok(policies) = build_policies(opts, statistical) else {
        return out; // already reported once by `vetoes`
    };
    let evidence = Evidence::new(&pairings);
    for (label, p) in policies {
        let d = p.decide(&evidence);
        if d.verdict == Verdict::Reject {
            out.push(Veto::evidence(name, format!("{label}: {}", d.reason)));
        }
    }
    out
}

type NamedPolicy = (&'static str, Box<dyn Policy>);

/// Build the configured policies; any invalid parameter is an error.
fn build_policies(
    opts: &PairwiseOptions,
    statistical: bool,
) -> Result<Vec<NamedPolicy>, pairwise_gate::Error> {
    let mut policies: Vec<NamedPolicy> = Vec::new();
    policies.push((
        "no-regression",
        Box::new(NoRegression::new(opts.pass_threshold)?),
    ));
    if let Some(f) = opts.floor {
        policies.push(("floor", Box::new(Floor::new(f)?)));
    }
    if statistical {
        if let Some(m) = opts.min_win_margin {
            policies.push(("margin", Box::new(Margin::new(m))));
        }
        if let Some(a) = opts.sign_alpha {
            policies.push(("sign-test", Box::new(SignTest::new(a)?)));
        }
        if let Some(z) = opts.wilson_z {
            policies.push(("wilson-lower", Box::new(WilsonLower::new(z)?)));
        }
    }
    Ok(policies)
}

fn summarise(items: &[String]) -> String {
    const SHOWN: usize = 5;
    let mut s = items
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    if items.len() > SHOWN {
        s.push_str(&format!("; … and {} more", items.len() - SHOWN));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tests_for(name: &str) -> BTreeMap<String, TestOutcomes> {
        let mut m = BTreeMap::new();
        m.insert(
            name.to_string(),
            TestOutcomes {
                baseline: vec![TestCase::new("a", TestStatus::Passed)],
                candidate: vec![TestCase::new("a", TestStatus::Passed)],
            },
        );
        m
    }

    #[test]
    fn invalid_options_block_instead_of_silently_dropping_a_policy() {
        let t = tests_for("e");
        for bad in [
            PairwiseOptions {
                pass_threshold: f64::NAN,
                ..PairwiseOptions::default()
            },
            PairwiseOptions {
                sign_alpha: Some(1.5),
                ..PairwiseOptions::default()
            },
            PairwiseOptions {
                wilson_z: Some(f64::INFINITY),
                ..PairwiseOptions::default()
            },
            PairwiseOptions {
                floor: Some(f64::NAN),
                ..PairwiseOptions::default()
            },
        ] {
            let v = vetoes(&["e"], &t, &bad);
            assert_eq!(v.len(), 1, "{bad:?}");
            assert_eq!(v[0].subject, "pairwise-options");
            assert_eq!(v[0].class, crate::gate::VetoClass::Harness);
        }
        assert!(vetoes(&["e"], &t, &PairwiseOptions::default()).is_empty());
    }

    #[test]
    fn no_evidence_means_no_validation_and_no_vetoes() {
        let bad = PairwiseOptions {
            pass_threshold: f64::NAN,
            ..PairwiseOptions::default()
        };
        assert!(vetoes(&["e"], &BTreeMap::new(), &bad).is_empty());
    }
}
