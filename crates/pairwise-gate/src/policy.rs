//! The [`Policy`] trait and its implementations.

use crate::model::{
    fault_reports, Decision, Error, Evidence, Metrics, Pairing, Part, Tally, Verdict,
};
use crate::stats::{sign_test_p_value, wilson_lower};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A pure, deterministic selection rule.
///
/// Every implementation first applies the [`Guard`](crate::Guard) in the
/// supplied [`Evidence`]: too few comparable pairings, or too large a fault
/// share, yields [`Verdict::Inconclusive`] before the rule itself is consulted.
pub trait Policy: Send + Sync {
    /// Judge the evidence.
    fn decide(&self, evidence: &Evidence<'_>) -> Decision;
}

fn make(verdict: Verdict, reason: String, metrics: Metrics, pairings: &[Pairing]) -> Decision {
    Decision {
        verdict,
        reason,
        metrics,
        fault_reports: fault_reports(pairings),
        parts: Vec::new(),
    }
}

/// Shared skeleton for the pairwise (tally-based) policies.
fn pairwise(
    ev: &Evidence<'_>,
    rule: impl FnOnce(&Tally, &mut Metrics) -> (Verdict, String),
) -> Decision {
    let t = Tally::from_pairings(ev.primary);
    let mut m = Metrics::from_tally(&t);
    let (verdict, reason) = match ev.guard.check(t.comparable(), t.faults, t.total()) {
        Some(why) => (Verdict::Inconclusive, why),
        None => rule(&t, &mut m),
    };
    make(verdict, reason, m, ev.primary)
}

fn finite(v: f64, what: &'static str) -> Result<f64, Error> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(Error::NonFinite(what))
    }
}

macro_rules! validated_deserialize {
    ($ty:ident, $field:ident) => {
        #[cfg(feature = "serde")]
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                struct Raw {
                    $field: f64,
                }
                let r = Raw::deserialize(d)?;
                $ty::new(r.$field).map_err(serde::de::Error::custom)
            }
        }
    };
}

/// Select iff `wins - losses > min_win_margin`.
///
/// Ties and faults do not move the margin. Note that with `min_win_margin = 0`
/// a candidate must win strictly more often than it loses.
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Evidence, Margin, Outcome, Pairing, Policy, Verdict};
/// let s = |x| Outcome::score(x).unwrap();
/// let v = vec![Pairing::new("a", s(0.0), s(1.0)), Pairing::new("b", s(0.0), s(1.0))];
/// assert_eq!(Margin::new(1).decide(&Evidence::new(&v)).verdict, Verdict::Select);
/// assert_eq!(Margin::new(2).decide(&Evidence::new(&v)).verdict, Verdict::Reject);
/// ```
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Margin {
    /// The margin `wins - losses` must strictly exceed.
    pub min_win_margin: u32,
}

impl Margin {
    /// Build a margin policy.
    #[must_use]
    pub fn new(min_win_margin: u32) -> Self {
        Margin { min_win_margin }
    }
}

impl Policy for Margin {
    fn decide(&self, ev: &Evidence<'_>) -> Decision {
        pairwise(ev, |t, _| {
            let net = t.wins as i128 - t.losses as i128;
            if net > i128::from(self.min_win_margin) {
                (
                    Verdict::Select,
                    format!("net wins {net} exceed margin {}", self.min_win_margin),
                )
            } else {
                (
                    Verdict::Reject,
                    format!(
                        "net wins {net} do not exceed margin {}",
                        self.min_win_margin
                    ),
                )
            }
        })
    }
}

/// Select iff every scored candidate result is `>= floor` and there is at
/// least one. Candidate-only: baseline scores and baseline faults are ignored,
/// so a pairing whose baseline faulted but whose candidate scored still counts.
/// Only candidate faults count toward the fault share and `metrics.faults`.
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Evidence, Floor, Outcome, Pairing, Policy, Verdict};
/// let s = |x| Outcome::score(x).unwrap();
/// let v = vec![Pairing::new("a", s(0.0), s(0.6))];
/// assert_eq!(Floor::new(0.5).unwrap().decide(&Evidence::new(&v)).verdict, Verdict::Select);
/// assert_eq!(Floor::new(0.7).unwrap().decide(&Evidence::new(&v)).verdict, Verdict::Reject);
/// ```
#[cfg_attr(feature = "serde", derive(Serialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Floor {
    floor: f64,
}

impl Floor {
    /// Build a floor policy; `floor` must be finite.
    pub fn new(floor: f64) -> Result<Self, Error> {
        Ok(Floor {
            floor: finite(floor, "floor")?,
        })
    }

    /// The minimum acceptable candidate score.
    #[must_use]
    pub fn floor(&self) -> f64 {
        self.floor
    }
}
validated_deserialize!(Floor, floor);

impl Policy for Floor {
    fn decide(&self, ev: &Evidence<'_>) -> Decision {
        let mut m = Metrics::from_tally(&Tally::from_pairings(ev.primary));
        let scores: Vec<f64> = ev
            .primary
            .iter()
            .filter_map(|p| p.candidate.as_score())
            .collect();
        let faults = ev.primary.len() - scores.len();
        m.faults = faults;
        let min = scores.iter().copied().reduce(f64::min);
        m.min_candidate = min;
        let (verdict, reason) = match ev.guard.check(scores.len(), faults, ev.primary.len()) {
            Some(why) => (Verdict::Inconclusive, why),
            None => {
                let below = scores.iter().filter(|&&x| x < self.floor).count();
                if below == 0 && !scores.is_empty() {
                    (
                        Verdict::Select,
                        format!(
                            "all {} candidate scores >= floor {}",
                            scores.len(),
                            self.floor
                        ),
                    )
                } else if scores.is_empty() {
                    (Verdict::Reject, "no candidate scores".to_owned())
                } else {
                    (
                        Verdict::Reject,
                        format!("{below} candidate scores below floor {}", self.floor),
                    )
                }
            }
        };
        let mut d = make(verdict, reason, m, ev.primary);
        d.fault_reports.retain(|r| r.side == crate::Side::Candidate);
        d
    }
}

/// Exact one-sided sign test: select iff the binomial tail probability of at
/// least `wins` successes in `wins + losses` fair trials is below `alpha`.
///
/// Ties are dropped. With no non-tied pairs the p-value is 1 and the verdict is
/// [`Verdict::Reject`]. See [`sign_test_p_value`].
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Evidence, Outcome, Pairing, Policy, SignTest, Verdict};
/// let s = |x| Outcome::score(x).unwrap();
/// let v: Vec<_> = (0..6).map(|i| Pairing::new(format!("{i}"), s(0.0), s(1.0))).collect();
/// // p = 1/64 = 0.0156
/// let d = SignTest::new(0.05).unwrap().decide(&Evidence::new(&v));
/// assert_eq!(d.verdict, Verdict::Select);
/// ```
#[cfg_attr(feature = "serde", derive(Serialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignTest {
    alpha: f64,
}

impl SignTest {
    /// Build a sign-test policy; `alpha` must lie in `(0, 1]`.
    pub fn new(alpha: f64) -> Result<Self, Error> {
        finite(alpha, "alpha")?;
        if alpha > 0.0 && alpha <= 1.0 {
            Ok(SignTest { alpha })
        } else {
            Err(Error::OutOfRange("alpha"))
        }
    }

    /// The significance level.
    #[must_use]
    pub fn alpha(&self) -> f64 {
        self.alpha
    }
}
validated_deserialize!(SignTest, alpha);

impl Policy for SignTest {
    fn decide(&self, ev: &Evidence<'_>) -> Decision {
        pairwise(ev, |t, m| {
            let n = t.wins + t.losses;
            let p = sign_test_p_value(t.wins, n);
            m.p_value = Some(p);
            if p < self.alpha {
                (
                    Verdict::Select,
                    format!(
                        "sign test p = {p:.6} < alpha {} ({} of {n})",
                        self.alpha, t.wins
                    ),
                )
            } else {
                (
                    Verdict::Reject,
                    format!(
                        "sign test p = {p:.6} >= alpha {} ({} of {n})",
                        self.alpha, t.wins
                    ),
                )
            }
        })
    }
}

/// Select iff the Wilson score lower bound on the win rate among non-tied
/// pairs exceeds `0.5`. See [`wilson_lower`].
///
/// With no non-tied pairs the bound is `0` and the verdict is
/// [`Verdict::Reject`].
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Evidence, Outcome, Pairing, Policy, Verdict, WilsonLower};
/// let s = |x| Outcome::score(x).unwrap();
/// let v: Vec<_> = (0..20).map(|i| Pairing::new(format!("{i}"), s(0.0), s(1.0))).collect();
/// let d = WilsonLower::new(1.96).unwrap().decide(&Evidence::new(&v));
/// assert_eq!(d.verdict, Verdict::Select);
/// ```
#[cfg_attr(feature = "serde", derive(Serialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WilsonLower {
    z: f64,
}

impl WilsonLower {
    /// Build a Wilson policy; `z` must be finite and non-negative.
    pub fn new(z: f64) -> Result<Self, Error> {
        finite(z, "z")?;
        if z >= 0.0 {
            Ok(WilsonLower { z })
        } else {
            Err(Error::OutOfRange("z"))
        }
    }

    /// The normal quantile.
    #[must_use]
    pub fn z(&self) -> f64 {
        self.z
    }
}
validated_deserialize!(WilsonLower, z);

impl Policy for WilsonLower {
    fn decide(&self, ev: &Evidence<'_>) -> Decision {
        pairwise(ev, |t, m| {
            let n = t.wins + t.losses;
            let lo = wilson_lower(t.wins, n, self.z);
            m.wilson_lower = Some(lo);
            if n > 0 && lo > 0.5 {
                (
                    Verdict::Select,
                    format!("Wilson lower bound {lo:.4} > 0.5 ({} of {n})", t.wins),
                )
            } else {
                (
                    Verdict::Reject,
                    format!("Wilson lower bound {lo:.4} <= 0.5 ({} of {n})", t.wins),
                )
            }
        })
    }
}

/// Reject if any comparable pairing where the baseline passed
/// (`score >= pass_threshold`) has a candidate below the threshold, however
/// many wins there are elsewhere; otherwise select ("no regression observed").
///
/// This is a guard, not a proof of improvement; pair it with an improvement
/// policy via [`TrainDev`]. Faulted pairings are never regressions.
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Evidence, NoRegression, Outcome, Pairing, Policy, Verdict};
/// let s = |x| Outcome::score(x).unwrap();
/// let v = vec![Pairing::new("a", s(0.9), s(0.1))];
/// let d = NoRegression::new(0.5).unwrap().decide(&Evidence::new(&v));
/// assert_eq!(d.verdict, Verdict::Reject);
/// ```
#[cfg_attr(feature = "serde", derive(Serialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoRegression {
    pass_threshold: f64,
}

impl NoRegression {
    /// Build the guard; `pass_threshold` must be finite.
    pub fn new(pass_threshold: f64) -> Result<Self, Error> {
        Ok(NoRegression {
            pass_threshold: finite(pass_threshold, "pass_threshold")?,
        })
    }

    /// The score at or above which a run counts as passed.
    #[must_use]
    pub fn pass_threshold(&self) -> f64 {
        self.pass_threshold
    }
}
validated_deserialize!(NoRegression, pass_threshold);

impl Policy for NoRegression {
    fn decide(&self, ev: &Evidence<'_>) -> Decision {
        let thr = self.pass_threshold;
        let regressions = ev
            .primary
            .iter()
            .filter(|p| match (p.baseline.as_score(), p.candidate.as_score()) {
                (Some(b), Some(c)) => b >= thr && c < thr,
                _ => false,
            })
            .count();
        pairwise(ev, |_, m| {
            m.regressions = Some(regressions);
            if regressions > 0 {
                (
                    Verdict::Reject,
                    format!("{regressions} pairings passed on baseline but fail on candidate"),
                )
            } else {
                (Verdict::Select, "no pass-to-fail regressions".to_owned())
            }
        })
    }
}

/// The acceptance equation: select iff `train` selects on
/// [`Evidence::primary`] **and** `dev` does not reject on
/// [`Evidence::heldout`].
///
/// * Train `Reject`/`Inconclusive` is returned as is.
/// * Dev `Reject` rejects; dev `Inconclusive` (a fault-heavy, thin or missing
///   held-out set) makes the whole decision `Inconclusive`, never a silent
///   pass. Dev `Select` completes the selection.
///
/// Choose a dev policy whose `Reject` means "regressed", such as
/// [`NoRegression`]; a strict improvement rule such as [`Margin`] rejects ties.
/// Both sub-decisions are attached in [`Decision::parts`]; `metrics` and
/// `fault_reports` are those of the training set.
///
/// # Examples
///
/// ```
/// use pairwise_gate::{Evidence, Margin, NoRegression, Outcome, Pairing, Policy, TrainDev, Verdict};
/// let s = |x| Outcome::score(x).unwrap();
/// let train = vec![Pairing::new("a", s(0.0), s(1.0)), Pairing::new("b", s(0.0), s(1.0))];
/// let dev = vec![Pairing::new("c", s(1.0), s(1.0))];
/// let td = TrainDev::new(Box::new(Margin::new(1)), Box::new(NoRegression::new(0.5).unwrap()));
/// let d = td.decide(&Evidence::new(&train).with_heldout(&dev));
/// assert_eq!(d.verdict, Verdict::Select);
/// ```
pub struct TrainDev {
    /// Policy that must select on the training pairings.
    pub train: Box<dyn Policy>,
    /// Policy that must not reject on the held-out pairings.
    pub dev: Box<dyn Policy>,
}

impl TrainDev {
    /// Combine a train policy and a dev policy.
    #[must_use]
    pub fn new(train: Box<dyn Policy>, dev: Box<dyn Policy>) -> Self {
        TrainDev { train, dev }
    }
}

impl core::fmt::Debug for TrainDev {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TrainDev").finish_non_exhaustive()
    }
}

impl Policy for TrainDev {
    fn decide(&self, ev: &Evidence<'_>) -> Decision {
        let tr = self.train.decide(&Evidence {
            primary: ev.primary,
            heldout: &[],
            guard: ev.guard,
        });
        let dv = self.dev.decide(&Evidence {
            primary: ev.heldout,
            heldout: &[],
            guard: ev.guard,
        });
        let (verdict, reason) = match (tr.verdict, dv.verdict) {
            (Verdict::Select, Verdict::Select) => (
                Verdict::Select,
                "train selects and held-out dev does not regress".to_owned(),
            ),
            (Verdict::Reject, _) => (Verdict::Reject, format!("train rejects: {}", tr.reason)),
            (Verdict::Inconclusive, _) => (
                Verdict::Inconclusive,
                format!("train inconclusive: {}", tr.reason),
            ),
            (Verdict::Select, Verdict::Reject) => (
                Verdict::Reject,
                format!("train selects but dev rejects: {}", dv.reason),
            ),
            (Verdict::Select, Verdict::Inconclusive) => (
                Verdict::Inconclusive,
                format!("train selects but dev inconclusive: {}", dv.reason),
            ),
        };
        Decision {
            verdict,
            reason,
            metrics: tr.metrics.clone(),
            fault_reports: tr.fault_reports.clone(),
            parts: vec![
                Part {
                    role: "train".to_owned(),
                    decision: tr,
                },
                Part {
                    role: "dev".to_owned(),
                    decision: dv,
                },
            ],
        }
    }
}
