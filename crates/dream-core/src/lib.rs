//! The nightly evidence-gated evolution loop around [`dream_gate`].
//!
//! [`dream_gate`] judges; this crate does the impure work around the judgement
//! and keeps the record that lets a judgement be replayed. A host supplies a
//! model ([`seams::Llm`]), optionally a memory ([`seams::MemorySink`]), an
//! operator channel ([`seams::Notifier`]) and a code forge ([`seams::Forge`]),
//! and drives the pieces below.
//!
//! * [`runner`]: how an evaluator command is executed (`LocalRunner`,
//!   `ScriptedRunner`, or your own).
//! * [`candidate`] and [`persist`]: build the candidate on an isolated git
//!   worktree and re-run the required evaluators. [`candidate::run`] enforces
//!   the order *refuse, guard, worktree, build, evaluate*: a protected patch
//!   never reaches git, the build or an evaluator.
//! * [`testfmt`]: per-test outcomes from `libtest`, JUnit XML or Vitest JSON.
//! * [`ledger`], [`streak`]: the markdown ledger and its signals; harness-stall
//!   detection and the dry streak.
//! * [`replay`]: capture a night's gate inputs and re-run the pure gate.
//! * [`source`], [`context`], [`compile`]: what the model is shown, and the
//!   prompt it is shown it in.
//! * [`screen`]: the optional Jev option, ordering or deferring candidate
//!   hypotheses under a budget before the measured loop; off is the identity.
//! * [`readiness`], [`runstate`], [`roster`], [`sweep`]: admission,
//!   restart-safe identity, fair rotation and branch ageing.
//!
//! The crate reads no environment variables and links no async runtime.
//!
//! # Example
//!
//! A scripted evaluator, judged by the pure gate:
//!
//! ```
//! use dream_core::runner::{EvaluatorRunner, ScriptedRunner};
//! use dream_gate::receipts::{EvaluatorReceipt, Phase};
//!
//! let runner = ScriptedRunner::new().passing("cargo test", "test result: ok. 3 passed");
//! let exec = runner.run("/work", "cargo test", 600);
//! let receipt = EvaluatorReceipt::from_exec("tests", "cargo test", Phase::Candidate, true, 600, exec);
//! assert!(receipt.outcome.is_pass());
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod automerge;
pub mod candidate;
pub mod compile;
pub mod context;
pub(crate) mod git;
pub mod ledger;
pub mod persist;
pub mod readiness;
pub mod replay;
pub mod roster;
pub mod runner;
pub mod runstate;
pub mod screen;
pub mod seams;
pub mod source;
pub mod streak;
pub mod sweep;
pub mod testfmt;
