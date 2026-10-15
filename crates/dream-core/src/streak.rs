//! Cross-night failure streaks: harness stalls and the dry streak.
//!
//! Two different questions are answered here, and they must not be confused.
//!
//! **Is the harness stuck?** Environment failures (the gate's *harness*-class
//! vetoes: an evaluator that never ran, timed out, was blocked or silent) are
//! fingerprinted with [`failure_fingerprint`] and carried from night to night in
//! a per-repo [`Manifest`] ([`failures_path`]). A `BLOCKED-ENV` night whose
//! failures are exactly the failures of the night before (at least one, none
//! new, none fixed) raises [`Notice::HarnessStuck`]: the candidate is not at
//! fault and no amount of further nights will change that without a human.
//! *A fixed failure that returns counts as new*: [`failure_fingerprint::advance`]
//! drops a fixed record from the next manifest, so its history restarts.
//!
//! **Is the repo dry?** The dry streak counts trailing nights whose verdict
//! was `REJECT` or `INCONCLUSIVE`; `ACCEPT` resets it; `BLOCKED-ENV`,
//! `HANDOFF` and `OPERATOR` neither advance nor reset it, because they are
//! operational facts rather than evidence about the repository. (An earlier
//! engine counted only `INCONCLUSIVE` and treated `REJECT` as a reset; a
//! falsified hypothesis is still no progress, so `REJECT` now counts.) Reading
//! the ledger goes through [`crate::ledger::parse_ledger`], not a column index.
//!
//! # Example
//!
//! ```
//! use dream_core::streak::{advance_failures, next_dry_streak};
//! use dream_gate::gate::{Veto, VetoClass};
//! use failure_fingerprint::Manifest;
//!
//! let veto = Veto { class: VetoClass::Harness, subject: "tests".into(), reason: "timed out after 1800s".into() };
//! let night1 = advance_failures(&Manifest::new(0), "BLOCKED-ENV", &[veto.clone()]);
//! assert!(!night1.stuck);
//! let night2 = advance_failures(&night1.manifest, "BLOCKED-ENV", &[veto]);
//! assert!(night2.stuck);
//! assert_eq!(next_dry_streak(3, "BLOCKED-ENV"), 3); // a stall does not make the repo dry
//! assert_eq!(next_dry_streak(3, "REJECT"), 4);
//! ```

use std::path::{Path, PathBuf};

use dream_gate::gate::{Veto, VetoClass};
use dream_gate::manifest::write_atomic;
use failure_fingerprint::{advance, Classification, Manifest, Observation};
use tracing::warn;

use crate::ledger::{parse_ledger, LedgerRow};
use crate::seams::Notice;

/// What one night did to the failure manifest.
#[derive(Debug, Clone)]
pub struct StreakUpdate {
    /// The manifest to persist for tomorrow.
    pub manifest: Manifest,
    /// How tonight's failures relate to last night's.
    pub classification: Classification,
    /// True when tonight is a `BLOCKED-ENV` night with exactly last night's
    /// (non-empty) failures.
    pub stuck: bool,
}

/// Advance the failure manifest by one night.
///
/// Only [`VetoClass::Harness`] vetoes are fingerprinted (task = the veto's
/// subject, stage = `harness`, cause = its reason, normalised by
/// [`failure_fingerprint`] so volatile numbers and paths do not split an
/// identity). The step is `prev.step + 1`: it counts recorded nights, not
/// calendar days, because the roster may skip a repo for days.
pub fn advance_failures(prev: &Manifest, verdict: &str, vetoes: &[Veto]) -> StreakUpdate {
    let observations: Vec<Observation> = vetoes
        .iter()
        .filter(|v| v.class == VetoClass::Harness)
        .map(|v| Observation::new(v.subject.as_str(), "harness", v.reason.as_str()))
        .collect();
    let (manifest, classification) = advance(prev, &observations, prev.step + 1);
    let stuck = verdict == "BLOCKED-ENV"
        && !classification.persisting.is_empty()
        && classification.new.is_empty()
        && classification.fixed.is_empty();
    StreakUpdate {
        manifest,
        classification,
        stuck,
    }
}

/// Where a repo's failure manifest lives: `failures.json` in the repo's state
/// directory, the parent of the per-night run directories that hold each
/// night's `run-state.json` ([`crate::runstate`]).
pub fn failures_path(repo_state_dir: &Path) -> PathBuf {
    repo_state_dir.join("failures.json")
}

/// Load the repo's manifest, advance it with tonight's vetoes and write it
/// back atomically.
///
/// A missing file starts afresh; a corrupt one also starts afresh, with a
/// warning, rather than wedging the night: the worst case is one missed
/// stall notice.
pub fn record_night(
    repo_state_dir: &Path,
    repo: &str,
    verdict: &str,
    vetoes: &[Veto],
) -> std::io::Result<StreakUpdate> {
    let path = failures_path(repo_state_dir);
    let prev = match std::fs::read_to_string(&path) {
        Ok(text) => Manifest::from_json(&text).unwrap_or_else(|e| {
            warn!(repo, error = %e, "failure manifest unreadable — starting afresh");
            Manifest::new(0)
        }),
        Err(_) => Manifest::new(0),
    };
    let update = advance_failures(&prev, verdict, vetoes);
    write_atomic(&path, update.manifest.to_json().as_bytes())?;
    Ok(update)
}

/// The operator notice for a stalled harness, when tonight is stuck.
pub fn harness_stuck_notice(repo: &str, night_id: &str, update: &StreakUpdate) -> Option<Notice> {
    update.stuck.then(|| Notice::HarnessStuck {
        repo: repo.to_string(),
        night_id: night_id.to_string(),
        fingerprints: update
            .classification
            .persisting
            .iter()
            .map(|r| r.fingerprint.to_string())
            .collect(),
    })
}

/// The new dry streak after a night with `verdict`: `REJECT` and
/// `INCONCLUSIVE` advance it, `ACCEPT` resets it, anything else leaves it.
pub fn next_dry_streak(prev: usize, verdict: &str) -> usize {
    match verdict {
        "REJECT" | "INCONCLUSIVE" => prev + 1,
        "ACCEPT" => 0,
        _ => prev,
    }
}

/// The dry streak at the end of a list of rows (see [`next_dry_streak`]).
pub fn dry_streak(rows: &[LedgerRow]) -> usize {
    rows.iter()
        .fold(0, |s, r| next_dry_streak(s, r.verdict.trim()))
}

/// [`dry_streak`] over a ledger body.
pub fn dry_streak_text(ledger: &str) -> usize {
    dry_streak(&parse_ledger(ledger).rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{empty_ledger, render_row, LedgerRow};
    use crate::seams::Notice;
    use dream_gate::gate::{Veto, VetoClass};

    fn harness(subject: &str, reason: &str) -> Veto {
        Veto {
            class: VetoClass::Harness,
            subject: subject.into(),
            reason: reason.into(),
        }
    }
    fn evidence(subject: &str, reason: &str) -> Veto {
        Veto {
            class: VetoClass::Evidence,
            subject: subject.into(),
            reason: reason.into(),
        }
    }

    #[test]
    fn identical_blocked_env_nights_raise_harness_stuck() {
        let mut m = Manifest::new(0);
        let v1 = [harness("tests", "required evaluator timed out after 1800s")];
        let night1 = advance_failures(&m, "BLOCKED-ENV", &v1);
        assert!(!night1.stuck, "first sighting is new, not stuck");
        assert_eq!(night1.classification.new.len(), 1);
        m = night1.manifest;
        // Same failure, different volatile number in the text.
        let v2 = [harness("tests", "required evaluator timed out after 1801s")];
        let night2 = advance_failures(&m, "BLOCKED-ENV", &v2);
        assert!(night2.stuck, "{:?}", night2.classification);
        assert_eq!(night2.classification.persisting.len(), 1);
        assert_eq!(night2.manifest.records[0].count, 2);
    }

    #[test]
    fn a_new_or_fixed_fingerprint_is_not_stuck() {
        let m = advance_failures(&Manifest::new(0), "BLOCKED-ENV", &[harness("a", "boom")]).manifest;
        // A second, different fault appears alongside: something is new.
        let n = advance_failures(&m, "BLOCKED-ENV", &[harness("a", "boom"), harness("b", "kaput")]);
        assert!(!n.stuck && n.classification.new.len() == 1);
        // One fault went away: the set is not identical to last night's.
        let m2 = n.manifest;
        let n = advance_failures(&m2, "BLOCKED-ENV", &[harness("a", "boom")]);
        assert!(!n.stuck && n.classification.fixed.len() == 1);
    }

    #[test]
    fn stuck_only_applies_to_blocked_env_nights() {
        let v = [harness("tests", "boom")];
        let m = advance_failures(&Manifest::new(0), "BLOCKED-ENV", &v).manifest;
        for verdict in ["REJECT", "INCONCLUSIVE", "ACCEPT", "HANDOFF"] {
            assert!(!advance_failures(&m, verdict, &v).stuck, "{verdict}");
        }
        assert!(advance_failures(&m, "BLOCKED-ENV", &v).stuck);
    }

    #[test]
    fn only_harness_class_vetoes_are_fingerprinted() {
        let v = [evidence("tests", "regressed"), harness("tests", "boom")];
        let n = advance_failures(&Manifest::new(0), "BLOCKED-ENV", &v);
        assert_eq!(n.manifest.records.len(), 1);
        let none = advance_failures(&Manifest::new(0), "REJECT", &[evidence("tests", "regressed")]);
        assert!(none.manifest.records.is_empty());
    }

    #[test]
    fn a_fixed_failure_that_returns_counts_as_new() {
        let v = [harness("tests", "boom")];
        let n1 = advance_failures(&Manifest::new(0), "BLOCKED-ENV", &v);
        let healthy = advance_failures(&n1.manifest, "REJECT", &[]);
        assert_eq!(healthy.classification.fixed.len(), 1);
        assert!(healthy.manifest.records.is_empty());
        let back = advance_failures(&healthy.manifest, "BLOCKED-ENV", &v);
        assert!(!back.stuck, "documented: history is dropped once fixed");
        assert_eq!(back.manifest.records[0].count, 1);
    }

    #[test]
    fn the_manifest_persists_per_repo_across_nights_and_fails_open_when_corrupt() {
        let d = tempfile::tempdir().unwrap();
        let v = [harness("tests", "boom")];
        let u1 = record_night(d.path(), "alpha", "BLOCKED-ENV", &v).unwrap();
        assert!(!u1.stuck);
        assert!(failures_path(d.path()).exists());
        let u2 = record_night(d.path(), "alpha", "BLOCKED-ENV", &v).unwrap();
        assert!(u2.stuck);
        assert_eq!(u2.manifest.step, 2, "step counts recorded nights");
        // A different directory is a different repo.
        let other = tempfile::tempdir().unwrap();
        assert!(!record_night(other.path(), "beta", "BLOCKED-ENV", &v).unwrap().stuck);
        // Corrupt file: start afresh rather than wedge the night.
        std::fs::write(failures_path(d.path()), "{ not json").unwrap();
        let u3 = record_night(d.path(), "alpha", "BLOCKED-ENV", &v).unwrap();
        assert!(!u3.stuck);
        assert_eq!(u3.manifest.step, 1);
    }

    #[test]
    fn stuck_notice_names_the_repo_and_the_fingerprints() {
        let v = [harness("tests", "boom")];
        let m = advance_failures(&Manifest::new(0), "BLOCKED-ENV", &v).manifest;
        let u = advance_failures(&m, "BLOCKED-ENV", &v);
        let n = harness_stuck_notice("alpha", "2026-10-15-alpha", &u).unwrap();
        match &n {
            Notice::HarnessStuck {
                repo,
                night_id,
                fingerprints,
            } => {
                assert_eq!((repo.as_str(), night_id.as_str()), ("alpha", "2026-10-15-alpha"));
                assert_eq!(fingerprints.len(), 1);
                assert_eq!(fingerprints[0].len(), 16);
            }
            other => panic!("{other:?}"),
        }
        let calm = advance_failures(&Manifest::new(0), "REJECT", &[]);
        assert!(harness_stuck_notice("alpha", "n", &calm).is_none());
    }

    fn row(verdict: &str) -> LedgerRow {
        LedgerRow::unreviewed(
            "2026-10-01".into(),
            "d".into(),
            "a finding".into(),
            "NONE".into(),
            "NONE".into(),
            "yes".into(),
            verdict.into(),
            String::new(),
            "w".into(),
            String::new(),
        )
    }
    fn ledger(verdicts: &[&str]) -> String {
        let mut s = empty_ledger();
        for v in verdicts {
            s.push_str(&render_row(&row(v)));
            s.push('\n');
        }
        s
    }

    #[test]
    fn dry_streak_counts_trailing_reject_and_inconclusive() {
        assert_eq!(dry_streak_text(&ledger(&["ACCEPT", "INCONCLUSIVE", "REJECT", "INCONCLUSIVE"])), 3);
        assert_eq!(dry_streak_text(&ledger(&["REJECT", "REJECT"])), 2, "REJECT now counts");
        assert_eq!(dry_streak_text(&ledger(&["INCONCLUSIVE", "ACCEPT"])), 0);
    }

    #[test]
    fn operational_rows_neither_advance_nor_reset_the_streak() {
        let l = ledger(&["REJECT", "BLOCKED-ENV", "HANDOFF", "OPERATOR", "INCONCLUSIVE", "BLOCKED-ENV"]);
        assert_eq!(dry_streak_text(&l), 2);
        assert_eq!(dry_streak_text(""), 0);
        assert_eq!(dry_streak_text(&empty_ledger()), 0);
    }

    #[test]
    fn dry_streak_reads_legacy_ten_column_rows_too() {
        let l = "| Date | Deep | Finding | Issue | PR | Evaluated? | Verdict | Effect | Witness | Prior-night fates |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n| 2026-08-01 | d | f | NONE | NONE | yes | REJECT | | w | |\n| 2026-08-02 | d | f | NONE | NONE | yes | INCONCLUSIVE | | w | |\n";
        assert_eq!(dry_streak_text(l), 2);
    }

    #[test]
    fn next_dry_streak_rules() {
        assert_eq!(next_dry_streak(2, "REJECT"), 3);
        assert_eq!(next_dry_streak(2, "INCONCLUSIVE"), 3);
        assert_eq!(next_dry_streak(2, "ACCEPT"), 0);
        for v in ["BLOCKED-ENV", "HANDOFF", "OPERATOR", "garbage"] {
            assert_eq!(next_dry_streak(2, v), 2, "{v}");
        }
    }
}
