//! The nightly evidence-gated evolution loop around [`dream_gate`].
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub(crate) mod git;
pub mod runner;
pub mod seams;
pub mod ledger;
pub mod readiness;
pub mod roster;
pub mod runstate;
pub mod sweep;
pub mod compile;
pub mod source;
pub mod context;
pub mod persist;
pub mod candidate;
