//! Restart-safe run identity and the phase journal.
//!
//! A loop whose identity is `topic + date` and whose "last run date" lives in
//! process memory can repeat a night after a restart, and repeated runs can
//! overwrite artefacts even while ledger rows accumulate.
//!
//! A run journal fixes both halves. A *night* is identified by its directory
//! (repo, date, deep slot); a run within it by the manifest's deterministic
//! [`dream_gate::manifest::run_id`], and this module writes a small durable record
//! beside it after every phase transition. On restart the engine reads it and
//! learns, without guessing, whether this night already finished (skip it),
//! died part-way (resume, with the attempt counted) or has burned through its
//! attempt budget (abandon it loudly rather than looping).
//!
//! The run id folds in the baseline HEAD, which the engine's own ledger commit
//! moves, so it cannot recognise a finished night after the fact. The terminal
//! rule is therefore night-level: once a night is `Persisted` or `Complete` it
//! is over regardless of run id, and only [`reset_night`] reopens it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use dream_gate::manifest::write_atomic;

/// Schema version of the run-state file.
pub const RUNSTATE_SCHEMA: u32 = 1;

/// Default number of attempts a single night gets before it is abandoned.
/// Two: one clean try plus one recovery from a crash or a reboot.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 2;

/// Where a run got to. Ordered by progress, so `>=` comparisons are meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// The run record exists; nothing has happened yet.
    Initialised,
    /// The experiment manifest is frozen on disk.
    ManifestFrozen,
    /// Every applicable evaluator ran on the baseline tree.
    BaselineEvaluated,
    /// The model has been called and its report stored.
    ModelCalled,
    /// The candidate tree was built and re-evaluated.
    CandidateEvaluated,
    /// The gate has ruled.
    Gated,
    /// Persistence has begun: the verdict is recorded and the draft PR and
    /// ledger row are being (or have been) written. Entered *before* those
    /// effects, so a crash can lose a publish but never repeat one.
    Persisted,
    /// The night is finished.
    Complete,
    /// Out of attempts. Terminal, and never silently retried.
    Abandoned,
}

impl Phase {
    /// Stable kebab-case label for the phase.
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Initialised => "initialised",
            Phase::ManifestFrozen => "manifest-frozen",
            Phase::BaselineEvaluated => "baseline-evaluated",
            Phase::ModelCalled => "model-called",
            Phase::CandidateEvaluated => "candidate-evaluated",
            Phase::Gated => "gated",
            Phase::Persisted => "persisted",
            Phase::Complete => "complete",
            Phase::Abandoned => "abandoned",
        }
    }
}

/// The durable record of one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunState {
    /// Schema version ([`RUNSTATE_SCHEMA`]).
    pub schema: u32,
    /// The manifest's deterministic run id.
    pub run_id: String,
    /// Night identifier (`<date>-<repo>`).
    pub night_id: String,
    /// Repository name.
    pub repo: String,
    /// Night date, `YYYY-MM-DD`.
    pub date: String,
    /// The last phase reached.
    pub phase: Phase,
    /// How many times this run has been started, including the current one.
    pub attempts: u32,
    /// Attempt budget for this night.
    pub max_attempts: u32,
    /// The phase the most recent restart resumed from, if it was a restart.
    pub resumed_from: Option<Phase>,
    /// RFC 3339 time the run was first recorded.
    pub first_seen: String,
    /// RFC 3339 time of the last update.
    pub updated_at: String,
    /// The most recent failure, if any.
    pub last_error: Option<String>,
    /// The verdict token, once known.
    pub verdict: Option<String>,
}

/// What [`begin`] found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// No prior record — a first attempt.
    Fresh(RunState),
    /// A prior attempt died part-way; this attempt continues it.
    Resumed(RunState),
    /// The run already finished. The caller must NOT re-run it.
    AlreadyComplete(RunState),
    /// The attempt budget is exhausted. The caller must record the abandonment
    /// and move on rather than looping.
    Abandoned(RunState),
}

impl Resume {
    /// The run record, whichever way the resume went.
    pub fn state(&self) -> &RunState {
        match self {
            Resume::Fresh(s)
            | Resume::Resumed(s)
            | Resume::AlreadyComplete(s)
            | Resume::Abandoned(s) => s,
        }
    }

    /// True when the caller should proceed with the night.
    pub fn should_run(&self) -> bool {
        matches!(self, Resume::Fresh(_) | Resume::Resumed(_))
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join("run-state.json")
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn save(dir: &Path, state: &RunState) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(&path(dir), &bytes)
}

/// Read the journal for `dir`, if one exists and parses.
pub fn load(dir: &Path) -> Option<RunState> {
    serde_json::from_str(&std::fs::read_to_string(path(dir)).ok()?).ok()
}

/// The night-level terminal rule: has the night `(repo, date, night_id)` in
/// `dir` already reached [`Phase::Persisted`] or [`Phase::Complete`]?
///
/// A night is identified by its directory (repo, date, deep slot), **not** by
/// the run id. The run id folds in the baseline HEAD, and the engine's own
/// ledger commit moves HEAD, so after persistence the run id of a restart no
/// longer matches the journal's. Once the externally visible effects have
/// begun, the night is over whatever the run id says. A `Persisted` record
/// (the process died before closing out) is finalised to `Complete` with the
/// verdict recorded at [`persist`]. Callers that would otherwise freeze a new
/// manifest should ask this first. Returns `None` for any other state,
/// including a journal for a different night.
pub fn finished_night(dir: &Path, repo: &str, date: &str, night_id: &str) -> Option<RunState> {
    let mut prior = load(dir)?;
    if prior.repo != repo || prior.date != date || prior.night_id != night_id {
        return None;
    }
    match prior.phase {
        Phase::Complete => Some(prior),
        Phase::Persisted => {
            prior.phase = Phase::Complete;
            prior.updated_at = now();
            // Best effort: the answer does not depend on the write landing.
            let _ = save(dir, &prior);
            Some(prior)
        }
        _ => None,
    }
}

/// Archive (rename, never delete) the night's run-state and manifest as
/// `run-state-superseded-<timestamp>.json` / `manifest-superseded-<timestamp>.json`,
/// so the next [`begin`] is fresh. This is the operator's explicit way to run a
/// different experiment on a night that already finished. Absent files are
/// skipped, so resetting an untouched night is a no-op.
pub fn reset_night(dir: &Path) -> std::io::Result<()> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    for stem in ["run-state", "manifest"] {
        let from = dir.join(format!("{stem}.json"));
        if from.exists() {
            std::fs::rename(&from, dir.join(format!("{stem}-superseded-{stamp}.json")))?;
        }
    }
    Ok(())
}

/// Open (or reopen) the journal for a run, deciding what this attempt is.
///
/// A night that already reached `Persisted`/`Complete` is
/// [`Resume::AlreadyComplete`] whatever `run_id` says (see [`finished_night`]).
///
/// Writes the updated record before returning, so a crash immediately after
/// this call still leaves the attempt counted — an interrupted run can never
/// masquerade as a fresh one.
pub fn begin(
    dir: &Path,
    run_id: &str,
    night_id: &str,
    repo: &str,
    date: &str,
    max_attempts: u32,
) -> std::io::Result<Resume> {
    std::fs::create_dir_all(dir)?;
    if let Some(done) = finished_night(dir, repo, date, night_id) {
        return Ok(Resume::AlreadyComplete(done));
    }
    match load(dir) {
        Some(prior) if prior.run_id == run_id => {
            if prior.phase == Phase::Abandoned || prior.attempts >= max_attempts {
                let mut s = prior;
                s.phase = Phase::Abandoned;
                s.updated_at = now();
                save(dir, &s)?;
                return Ok(Resume::Abandoned(s));
            }
            let mut s = prior;
            s.resumed_from = Some(s.phase);
            s.attempts += 1;
            s.updated_at = now();
            s.last_error = None;
            save(dir, &s)?;
            Ok(Resume::Resumed(s))
        }
        // No record, or a record for a different experiment (the baseline or
        // the config moved). Either way this is a fresh run; the manifest
        // freeze has already archived the superseded document.
        _ => {
            let s = RunState {
                schema: RUNSTATE_SCHEMA,
                run_id: run_id.into(),
                night_id: night_id.into(),
                repo: repo.into(),
                date: date.into(),
                phase: Phase::Initialised,
                attempts: 1,
                max_attempts,
                resumed_from: None,
                first_seen: now(),
                updated_at: now(),
                last_error: None,
                verdict: None,
            };
            save(dir, &s)?;
            Ok(Resume::Fresh(s))
        }
    }
}

/// Record forward progress. Never moves the phase backwards, so a resumed run
/// re-walking earlier steps cannot rewrite history.
pub fn advance(dir: &Path, state: &mut RunState, phase: Phase) -> std::io::Result<()> {
    if phase > state.phase {
        state.phase = phase;
    }
    state.updated_at = now();
    save(dir, state)
}

/// Mark persistence as begun: advance to [`Phase::Persisted`] and record the
/// verdict. Call this **before** any externally visible effect (forge request,
/// ledger commit), so a crash afterwards can never repeat them.
pub fn persist(dir: &Path, state: &mut RunState, verdict: &str) -> std::io::Result<()> {
    state.verdict = Some(verdict.to_string());
    advance(dir, state, Phase::Persisted)
}

/// Close the run out. `verdict` is the ledger verdict actually written.
pub fn complete(dir: &Path, state: &mut RunState, verdict: &str) -> std::io::Result<()> {
    state.phase = Phase::Complete;
    state.verdict = Some(verdict.to_string());
    state.updated_at = now();
    save(dir, state)
}

/// Record a failure without closing the run, so the next attempt resumes.
pub fn fail(dir: &Path, state: &mut RunState, error: &str) -> std::io::Result<()> {
    state.last_error = Some(error.chars().take(500).collect());
    state.updated_at = now();
    save(dir, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = "0123456789abcdef";

    fn begin_default(dir: &Path) -> Resume {
        begin(
            dir,
            RUN,
            "2026-09-05-alpha",
            "alpha",
            "2026-09-05",
            DEFAULT_MAX_ATTEMPTS,
        )
        .unwrap()
    }

    #[test]
    fn a_first_attempt_is_fresh_and_durable() {
        let d = tempfile::tempdir().unwrap();
        let r = begin_default(d.path());
        assert!(matches!(r, Resume::Fresh(_)));
        assert!(r.should_run());
        let on_disk = load(d.path()).unwrap();
        assert_eq!(on_disk.attempts, 1);
        assert_eq!(on_disk.phase, Phase::Initialised);
    }

    /// The interrupted-persistence case worth exercising: a run that
    /// died after the model call resumes from exactly where it stopped, and the
    /// attempt is counted rather than lost.
    #[test]
    fn an_interrupted_run_resumes_from_its_last_phase() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        advance(d.path(), &mut s, Phase::ManifestFrozen).unwrap();
        advance(d.path(), &mut s, Phase::ModelCalled).unwrap();
        fail(d.path(), &mut s, "process killed mid-persist").unwrap();
        drop(s); // the process dies here

        let r = begin_default(d.path());
        let Resume::Resumed(s2) = r.clone() else {
            panic!("expected a resume, got {r:?}")
        };
        assert_eq!(s2.resumed_from, Some(Phase::ModelCalled));
        assert_eq!(s2.attempts, 2);
        assert!(r.should_run());
        assert_eq!(
            s2.last_error, None,
            "the new attempt clears the stale error"
        );
    }

    #[test]
    fn a_completed_run_is_never_silently_repeated() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        complete(d.path(), &mut s, "REJECT").unwrap();

        let r = begin_default(d.path());
        assert!(matches!(r, Resume::AlreadyComplete(_)), "got {r:?}");
        assert!(!r.should_run());
        assert_eq!(r.state().verdict.as_deref(), Some("REJECT"));
        assert_eq!(
            r.state().attempts,
            1,
            "a skipped repeat does not burn an attempt"
        );
    }

    #[test]
    fn attempts_are_bounded_and_exhaustion_is_terminal() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(begin_default(d.path()), Resume::Fresh(_)));
        assert!(matches!(begin_default(d.path()), Resume::Resumed(_))); // attempt 2
        let r = begin_default(d.path());
        assert!(matches!(r, Resume::Abandoned(_)), "got {r:?}");
        assert!(!r.should_run());
        assert_eq!(load(d.path()).unwrap().phase, Phase::Abandoned);
        // Still abandoned on the next look — no accidental revival.
        assert!(matches!(begin_default(d.path()), Resume::Abandoned(_)));
    }

    #[test]
    fn a_different_run_id_starts_a_fresh_journal() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        advance(d.path(), &mut s, Phase::CandidateEvaluated).unwrap();
        // New baseline ⇒ new run id ⇒ a genuinely new experiment (the night
        // had not started persisting; a finished night is covered separately).
        let r = begin(
            d.path(),
            "fedcba9876543210",
            "2026-09-05-alpha",
            "alpha",
            "2026-09-05",
            2,
        )
        .unwrap();
        assert!(matches!(r, Resume::Fresh(_)), "got {r:?}");
        assert_eq!(r.state().attempts, 1);
    }

    #[test]
    fn advance_never_moves_backwards() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        advance(d.path(), &mut s, Phase::Gated).unwrap();
        advance(d.path(), &mut s, Phase::ManifestFrozen).unwrap();
        assert_eq!(load(d.path()).unwrap().phase, Phase::Gated);
    }

    #[test]
    fn a_corrupt_journal_does_not_wedge_the_night() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(path(d.path()), "{ this is not json").unwrap();
        assert!(matches!(begin_default(d.path()), Resume::Fresh(_)));
    }

    const OTHER: &str = "fedcba9876543210";

    fn begin_as(dir: &Path, run_id: &str, night_id: &str, date: &str) -> Resume {
        let repo = "alpha";
        begin(dir, run_id, night_id, repo, date, DEFAULT_MAX_ATTEMPTS).unwrap()
    }

    #[test]
    fn a_complete_night_is_complete_whatever_the_run_id() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        complete(d.path(), &mut s, "ACCEPT").unwrap();
        let r = begin_as(d.path(), OTHER, "2026-09-05-alpha", "2026-09-05");
        assert!(matches!(r, Resume::AlreadyComplete(_)), "got {r:?}");
        assert_eq!(r.state().verdict.as_deref(), Some("ACCEPT"));
        assert_eq!(load(d.path()).unwrap().run_id, RUN, "journal untouched");
    }

    #[test]
    fn a_persisted_night_is_finished_with_its_recorded_verdict() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        persist(d.path(), &mut s, "ACCEPT").unwrap();
        assert_eq!(load(d.path()).unwrap().phase, Phase::Persisted);
        let r = begin_as(d.path(), OTHER, "2026-09-05-alpha", "2026-09-05");
        assert!(matches!(r, Resume::AlreadyComplete(_)), "got {r:?}");
        let on_disk = load(d.path()).unwrap();
        assert_eq!(on_disk.phase, Phase::Complete, "finalised on disk");
        assert_eq!(on_disk.verdict.as_deref(), Some("ACCEPT"));
        // The same run id after Persisted is terminal too.
        assert!(matches!(
            begin_default(d.path()),
            Resume::AlreadyComplete(_)
        ));
    }

    #[test]
    fn a_gated_night_with_a_moved_baseline_is_still_fresh() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        advance(d.path(), &mut s, Phase::Gated).unwrap();
        let r = begin_as(d.path(), OTHER, "2026-09-05-alpha", "2026-09-05");
        assert!(matches!(r, Resume::Fresh(_)), "got {r:?}");
        assert_eq!(r.state().run_id, OTHER);
    }

    #[test]
    fn a_different_date_or_deep_slot_is_an_independent_night() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        complete(d.path(), &mut s, "ACCEPT").unwrap();
        // Same directory, different night identity: not the finished night.
        let r = begin_as(d.path(), OTHER, "2026-09-06-alpha", "2026-09-06");
        assert!(matches!(r, Resume::Fresh(_)), "date: {r:?}");
        let d2 = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d2.path()) else {
            panic!()
        };
        complete(d2.path(), &mut s, "ACCEPT").unwrap();
        let r = begin_as(d2.path(), OTHER, "2026-09-05-alpha-deep2", "2026-09-05");
        assert!(matches!(r, Resume::Fresh(_)), "night id: {r:?}");
    }

    #[test]
    fn an_abandoned_night_with_a_moved_baseline_is_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let _ = begin_default(d.path());
        let _ = begin_default(d.path());
        assert!(matches!(begin_default(d.path()), Resume::Abandoned(_)));
        // Different run id: existing behaviour, a fresh run.
        let r = begin_as(d.path(), OTHER, "2026-09-05-alpha", "2026-09-05");
        assert!(matches!(r, Resume::Fresh(_)), "got {r:?}");
    }

    #[test]
    fn finished_night_reports_without_touching_an_unfinished_one() {
        let d = tempfile::tempdir().unwrap();
        assert!(finished_night(d.path(), "alpha", "2026-09-05", "2026-09-05-alpha").is_none());
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        advance(d.path(), &mut s, Phase::Gated).unwrap();
        assert!(finished_night(d.path(), "alpha", "2026-09-05", "2026-09-05-alpha").is_none());
        complete(d.path(), &mut s, "REJECT").unwrap();
        let f = finished_night(d.path(), "alpha", "2026-09-05", "2026-09-05-alpha").unwrap();
        assert_eq!(f.verdict.as_deref(), Some("REJECT"));
    }

    #[test]
    fn reset_night_archives_and_the_next_begin_is_fresh() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        complete(d.path(), &mut s, "ACCEPT").unwrap();
        std::fs::write(d.path().join("manifest.json"), "{\"m\":1}").unwrap();

        reset_night(d.path()).unwrap();

        assert!(load(d.path()).is_none());
        assert!(!d.path().join("manifest.json").exists());
        let names: Vec<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().any(|n| n.starts_with("run-state-superseded-")),
            "{names:?}"
        );
        let m = names
            .iter()
            .find(|n| n.starts_with("manifest-superseded-"))
            .expect("manifest archived");
        assert_eq!(
            std::fs::read_to_string(d.path().join(m)).unwrap(),
            "{\"m\":1}"
        );
        let r = begin_as(d.path(), OTHER, "2026-09-05-alpha", "2026-09-05");
        assert!(matches!(r, Resume::Fresh(_)), "got {r:?}");
        // Resetting an empty directory is a no-op, not an error.
        reset_night(tempfile::tempdir().unwrap().path()).unwrap();
    }
}
