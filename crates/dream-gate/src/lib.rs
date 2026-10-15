//! Deterministic, pure acceptance gate for model-written patches.
//!
//! `dream-gate` is the part of an evidence-gated self-improvement loop that
//! must never be argued with. It performs **no network access and spawns no
//! subprocess**; every function is a deterministic function of its inputs.
//!
//! * [`manifest`] freezes what an experiment *is* before the model is called.
//! * [`receipts`] types the raw result of every evaluator run.
//! * [`guard`] refuses patches that touch protected paths (the evaluator's own
//!   gate, CI, manifests, thresholds, held-out tests …). **Fail closed.**
//! * [`gate`] combines manifest, strict verdict, receipts, guard and optional
//!   per-test evidence into a final [`gate::GateDecision`].
//! * [`pertest`] defines optional per-test evidence the gate can weigh with
//!   `pairwise-gate` and `failure-fingerprint`.
//! * [`verdict`], [`witness`], [`patch`], [`outcome`] and [`config`] are the
//!   supporting vocabulary.
//!
//! # Example
//!
//! ```
//! use dream_gate::guard::{check, ProtectedPolicy};
//!
//! let policy = ProtectedPolicy::default();
//! let ok = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n";
//! assert!(check(ok, &policy).is_ok());
//!
//! let evil = "diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml\n--- a/.github/workflows/ci.yml\n+++ b/.github/workflows/ci.yml\n@@ -1 +1 @@\n-a\n+b\n";
//! assert!(check(evil, &policy).is_err());
//! ```
//!
//! The guard must run **before** a patch is applied, and [`gate::decide`]
//! re-derives the touched paths from the patch again as defence in depth.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod config;
pub mod gate;
pub mod guard;
pub mod manifest;
pub mod outcome;
pub mod patch;
pub mod pertest;
pub mod receipts;
pub mod verdict;
pub mod witness;
