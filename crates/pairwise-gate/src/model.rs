//! Data model: outcomes, pairings, tallies, guard, evidence and decisions.

use core::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Error returned by validating constructors.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A value that must be finite was `NaN` or infinite.
    NonFinite(&'static str),
    /// A value was outside its permitted range.
    OutOfRange(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NonFinite(w) => write!(f, "{w} must be finite"),
            Error::OutOfRange(w) => write!(f, "{w} is out of range"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq)]
enum Inner {
    Score(f64),
    Fault(String),
}

/// The result of running one side of a pairing on one task.
///
/// Either a finite score (higher is better) or a fault: an infrastructure
/// failure that says nothing about the quality of what was being evaluated.
/// Construct with [`Outcome::score`] (which rejects `NaN` and infinities) or
/// [`Outcome::fault`].
///
/// # Examples
///
/// ```
/// use pairwise_gate::Outcome;
/// assert_eq!(Outcome::score(0.5).unwrap().as_score(), Some(0.5));
/// assert!(Outcome::score(f64::NAN).is_err());
/// assert_eq!(Outcome::fault("timeout").fault_reason(), Some("timeout"));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome(Inner);

impl Outcome {
    /// A finite score. Returns [`Error::NonFinite`] for `NaN` or infinity.
    pub fn score(value: f64) -> Result<Self, Error> {
        if value.is_finite() {
            Ok(Outcome(Inner::Score(value)))
        } else {
            Err(Error::NonFinite("score"))
        }
    }

    /// An infrastructure fault with a human-readable reason.
    #[must_use]
    pub fn fault(reason: impl Into<String>) -> Self {
        Outcome(Inner::Fault(reason.into()))
    }

    /// The score, or `None` for a fault.
    #[must_use]
    pub fn as_score(&self) -> Option<f64> {
        match self.0 {
            Inner::Score(x) => Some(x),
            Inner::Fault(_) => None,
        }
    }

    /// The fault reason, or `None` for a score.
    #[must_use]
    pub fn fault_reason(&self) -> Option<&str> {
        match &self.0 {
            Inner::Fault(r) => Some(r),
            Inner::Score(_) => None,
        }
    }
}

#[cfg(feature = "serde")]
impl Serialize for Outcome {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for Outcome {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Inner::deserialize(d)? {
            Inner::Score(x) => Outcome::score(x).map_err(serde::de::Error::custom),
            Inner::Fault(r) => Ok(Outcome::fault(r)),
        }
    }
}

/// One task run on both sides.
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Outcome, Pairing};
/// let p = Pairing::new("t1", Outcome::score(0.2).unwrap(), Outcome::score(0.9).unwrap());
/// assert_eq!(p.task, "t1");
/// ```
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Pairing {
    /// Task identifier, used only in reports.
    pub task: String,
    /// Outcome of the baseline on this task.
    pub baseline: Outcome,
    /// Outcome of the candidate on this task.
    pub candidate: Outcome,
}

impl Pairing {
    /// Build a pairing.
    #[must_use]
    pub fn new(task: impl Into<String>, baseline: Outcome, candidate: Outcome) -> Self {
        Pairing {
            task: task.into(),
            baseline,
            candidate,
        }
    }
}

/// Which side of a pairing a fault occurred on.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The baseline run.
    Baseline,
    /// The candidate run.
    Candidate,
}

/// A fault that was excluded from the tally, kept for reporting.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct FaultReport {
    /// Task on which the fault occurred.
    pub task: String,
    /// Side that faulted.
    pub side: Side,
    /// Reason supplied with the fault.
    pub reason: String,
}

/// Win/loss/tie/fault counts over a set of pairings.
///
/// A pairing is a *fault* if either side is a fault; faults are never wins,
/// losses or ties.
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Outcome, Pairing, Tally};
/// let s = |x| Outcome::score(x).unwrap();
/// let t = Tally::from_pairings(&[
///     Pairing::new("a", s(0.0), s(1.0)),
///     Pairing::new("b", Outcome::fault("down"), s(1.0)),
/// ]);
/// assert_eq!((t.wins, t.losses, t.ties, t.faults), (1, 0, 0, 1));
/// ```
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tally {
    /// Pairings where the candidate scored strictly higher.
    pub wins: usize,
    /// Pairings where the candidate scored strictly lower.
    pub losses: usize,
    /// Pairings with equal scores.
    pub ties: usize,
    /// Pairings excluded because at least one side faulted.
    pub faults: usize,
}

impl Tally {
    /// Count a slice of pairings.
    #[must_use]
    pub fn from_pairings(pairings: &[Pairing]) -> Self {
        let mut t = Tally::default();
        for p in pairings {
            match (p.baseline.as_score(), p.candidate.as_score()) {
                (Some(b), Some(c)) => {
                    if c > b {
                        t.wins += 1;
                    } else if c < b {
                        t.losses += 1;
                    } else {
                        t.ties += 1;
                    }
                }
                _ => t.faults += 1,
            }
        }
        t
    }

    /// Pairings with a score on both sides: `wins + losses + ties`.
    #[must_use]
    pub fn comparable(&self) -> usize {
        self.wins + self.losses + self.ties
    }

    /// All pairings: comparable plus faulted.
    #[must_use]
    pub fn total(&self) -> usize {
        self.comparable() + self.faults
    }
}

/// Thresholds below which a result is [`Verdict::Inconclusive`].
///
/// Defaults: `min_comparable = 1`, `max_fault_share = 0.5`. A fault share
/// exactly equal to `max_fault_share` is tolerated; only a larger share is
/// inconclusive.
///
/// # Examples
///
/// ```
/// use pairwise_gate::Guard;
/// let g = Guard::new(5, 0.25).unwrap();
/// assert_eq!(g.min_comparable(), 5);
/// assert!(Guard::new(1, 1.5).is_err());
/// assert_eq!(Guard::default().min_comparable(), 1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Guard {
    min_comparable: usize,
    max_fault_share: f64,
}

impl Default for Guard {
    fn default() -> Self {
        Guard {
            min_comparable: 1,
            max_fault_share: 0.5,
        }
    }
}

impl Guard {
    /// Validated constructor; `max_fault_share` must lie in `[0, 1]`.
    pub fn new(min_comparable: usize, max_fault_share: f64) -> Result<Self, Error> {
        if !max_fault_share.is_finite() {
            return Err(Error::NonFinite("max_fault_share"));
        }
        if !(0.0..=1.0).contains(&max_fault_share) {
            return Err(Error::OutOfRange("max_fault_share"));
        }
        Ok(Guard {
            min_comparable,
            max_fault_share,
        })
    }

    /// Minimum number of comparable pairings.
    #[must_use]
    pub fn min_comparable(&self) -> usize {
        self.min_comparable
    }

    /// Maximum tolerated share of faulted pairings.
    #[must_use]
    pub fn max_fault_share(&self) -> f64 {
        self.max_fault_share
    }

    /// Returns a reason string if the evidence is too thin to decide.
    pub(crate) fn check(&self, comparable: usize, faults: usize, total: usize) -> Option<String> {
        if total > 0 && (faults as f64) / (total as f64) > self.max_fault_share {
            return Some(format!(
                "{faults} of {total} pairings faulted, above the allowed share {}",
                self.max_fault_share
            ));
        }
        if comparable < self.min_comparable {
            return Some(format!(
                "{comparable} comparable pairings, fewer than the required {}",
                self.min_comparable
            ));
        }
        None
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for Guard {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            min_comparable: usize,
            max_fault_share: f64,
        }
        let r = Raw::deserialize(d)?;
        Guard::new(r.min_comparable, r.max_fault_share).map_err(serde::de::Error::custom)
    }
}

/// Input to a [`Policy`](crate::Policy): primary pairings, optional held-out
/// pairings, and the [`Guard`].
///
/// Single-set policies read only `primary`; [`TrainDev`](crate::TrainDev)
/// reads `primary` as the training set and `heldout` as the dev set.
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Evidence, Guard};
/// let ev = Evidence::new(&[]).with_guard(Guard::new(3, 0.5).unwrap());
/// assert_eq!(ev.guard.min_comparable(), 3);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Evidence<'a> {
    /// Pairings the main policy is judged on (the train set for `TrainDev`).
    pub primary: &'a [Pairing],
    /// Held-out pairings; empty unless supplied.
    pub heldout: &'a [Pairing],
    /// Inconclusiveness thresholds.
    pub guard: Guard,
}

impl<'a> Evidence<'a> {
    /// Evidence with no held-out set and the default [`Guard`].
    #[must_use]
    pub fn new(primary: &'a [Pairing]) -> Self {
        Evidence {
            primary,
            heldout: &[],
            guard: Guard::default(),
        }
    }

    /// Attach a held-out (dev) set.
    #[must_use]
    pub fn with_heldout(mut self, heldout: &'a [Pairing]) -> Self {
        self.heldout = heldout;
        self
    }

    /// Replace the guard.
    #[must_use]
    pub fn with_guard(mut self, guard: Guard) -> Self {
        self.guard = guard;
        self
    }
}

/// Outcome of applying a policy.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The evidence supports adopting the candidate.
    Select,
    /// The evidence is adequate and does not support the candidate.
    Reject,
    /// The evidence is inadequate (too few comparable pairings, or too many
    /// infrastructure faults) to decide either way.
    Inconclusive,
}

/// Numbers behind a decision. Fields not relevant to a policy are `None`.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metrics {
    /// Candidate strictly better.
    pub wins: usize,
    /// Candidate strictly worse.
    pub losses: usize,
    /// Equal scores.
    pub ties: usize,
    /// Faulted pairings (excluded from the tally).
    pub faults: usize,
    /// One-sided sign-test p-value ([`SignTest`](crate::SignTest)).
    pub p_value: Option<f64>,
    /// Wilson lower bound ([`WilsonLower`](crate::WilsonLower)).
    pub wilson_lower: Option<f64>,
    /// Pass-to-fail pairings ([`NoRegression`](crate::NoRegression)).
    pub regressions: Option<usize>,
    /// Lowest candidate score ([`Floor`](crate::Floor)).
    pub min_candidate: Option<f64>,
}

impl Metrics {
    pub(crate) fn from_tally(t: &Tally) -> Self {
        Metrics {
            wins: t.wins,
            losses: t.losses,
            ties: t.ties,
            faults: t.faults,
            ..Metrics::default()
        }
    }
}

/// A named sub-decision of a composite policy.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Role, for example `"train"` or `"dev"`.
    pub role: String,
    /// The sub-policy's decision.
    pub decision: Decision,
}

/// A verdict with its explanation and supporting numbers.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// Select, reject or inconclusive.
    pub verdict: Verdict,
    /// Human-readable reason.
    pub reason: String,
    /// Counts and test statistics.
    pub metrics: Metrics,
    /// Every excluded fault, per side.
    pub fault_reports: Vec<FaultReport>,
    /// Sub-decisions for composite policies; empty otherwise.
    pub parts: Vec<Part>,
}

pub(crate) fn fault_reports(pairings: &[Pairing]) -> Vec<FaultReport> {
    let mut out = Vec::new();
    for p in pairings {
        for (side, o) in [
            (Side::Baseline, &p.baseline),
            (Side::Candidate, &p.candidate),
        ] {
            if let Some(r) = o.fault_reason() {
                out.push(FaultReport {
                    task: p.task.clone(),
                    side,
                    reason: r.to_owned(),
                });
            }
        }
    }
    out
}
