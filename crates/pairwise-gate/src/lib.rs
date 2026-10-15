//! Evidence-gated selection policies for paired comparisons.
//!
//! A self-improvement loop proposes a *candidate* (a patch, a prompt, a
//! configuration) and wants to know whether to keep it. It runs candidate and
//! *baseline* on the same ordered list of tasks and hands the paired outcomes
//! to a [`Policy`], which answers [`Verdict::Select`], [`Verdict::Reject`] or
//! [`Verdict::Inconclusive`].
//!
//! # Faults are not losses
//!
//! An [`Outcome`] is either a finite [`Outcome::score`] or an
//! [`Outcome::fault`]. A fault is an infrastructure failure (service down,
//! timeout, patch would not apply). A pairing with a fault on either side is
//! **never** counted as a win or a loss: it is removed from the tally and
//! listed in [`Decision::fault_reports`]. This deliberately differs from
//! rankers that treat a missing score as negative infinity.
//!
//! If too few comparable pairings remain, or too large a share of all pairings
//! faulted (see [`Guard`]), the verdict is [`Verdict::Inconclusive`], so
//! infrastructure trouble is distinguishable from evidence against the
//! candidate.
//!
//! # Policies
//!
//! | Policy | Selects when |
//! |---|---|
//! | [`Margin`] | `wins - losses > min_win_margin` |
//! | [`Floor`] | at least one candidate score and all are `>= floor` (candidate-only) |
//! | [`SignTest`] | exact one-sided binomial p-value over non-tied pairs `< alpha` |
//! | [`WilsonLower`] | Wilson lower bound of the win rate among non-tied pairs `> 0.5` |
//! | [`NoRegression`] | never rejects a pairing that passed on baseline and fails on candidate |
//! | [`TrainDev`] | the train policy selects and the held-out policy does not reject |
//!
//! # Example
//!
//! ```
//! use pairwise_gate::{Evidence, Outcome, Pairing, Policy, SignTest, Verdict};
//!
//! # fn main() -> Result<(), pairwise_gate::Error> {
//! let mut pairings = Vec::new();
//! for i in 0..9 {
//!     pairings.push(Pairing::new(format!("task-{i}"), Outcome::score(0.4)?, Outcome::score(0.7)?));
//! }
//! pairings.push(Pairing::new("task-9", Outcome::score(0.7)?, Outcome::score(0.4)?));
//! // An annexe outage is reported, not counted against the candidate.
//! pairings.push(Pairing::new("task-10", Outcome::fault("annexe down"), Outcome::score(0.1)?));
//!
//! let policy = SignTest::new(0.05)?;
//! let decision = policy.decide(&Evidence::new(&pairings));
//! assert_eq!(decision.verdict, Verdict::Select);
//! assert_eq!((decision.metrics.wins, decision.metrics.losses), (9, 1));
//! assert_eq!(decision.metrics.faults, 1);
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! * `serde` (default): `Serialize`/`Deserialize` for configuration and result
//!   types. Deserialisation re-runs the same validation as the constructors, so
//!   a non-finite score or out-of-range parameter is an error, not a panic.
//!
//! The crate contains no `unsafe` code and has no non-optional dependencies.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod model;
mod policy;
mod stats;

pub use model::{
    Decision, Error, Evidence, FaultReport, Guard, Metrics, Outcome, Pairing, Part, Side, Tally,
    Verdict,
};
pub use policy::{Floor, Margin, NoRegression, Policy, SignTest, TrainDev, WilsonLower};
pub use stats::{sign_test_p_value, wilson_lower};
