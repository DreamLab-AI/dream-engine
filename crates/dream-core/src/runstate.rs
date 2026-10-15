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
//!
//! # Delivery guarantee: at-most-once
//!
//! [`persist`] journals [`Phase::Persisted`] *before* the forge request and the
//! ledger commit, and nothing records their outcomes. A crash after `Persisted`
//! can therefore **lose** a publish (before the pull request exists, or after
//! it exists but before its URL or ledger row is recorded) and never repeats
//! one; the restart finalises the night to `Complete` without reconciling
//! against the forge. This is a deliberate trade-off, not an oversight.
//! Recovery is [`reset_night`]; the idempotency key for any future
//! reconciliation is the night key `(repo, date, night_id)`.
//!
//! # Exclusion and damage
//!
//! Hold a [`NightLock`] across the whole night, taken before the first
//! journal read. A journal that exists but cannot be trusted is a
//! [`JournalError`], never a fresh start.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

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

/// Why the run journal could not be trusted.
///
/// Absence is **not** an error ([`load_checked`] returns `Ok(None)`): only a
/// journal that exists but cannot be read, parsed or matched to this night is.
/// The engine treats every variant as `BLOCKED-ENV` for the night, leaves the
/// file untouched as evidence, and points the operator at `reset-night`.
#[derive(Debug, Error)]
pub enum JournalError {
    /// An I/O failure that is not "file not found" (permissions, a directory
    /// where the file should be, a failed write).
    #[error("run journal I/O error at {path}: {source}")]
    Io {
        /// The journal path.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The file exists but is not a valid journal (truncated or garbled JSON).
    #[error("run journal at {path} is corrupt: {detail}")]
    Corrupt {
        /// The journal path.
        path: PathBuf,
        /// The parse error.
        detail: String,
    },
    /// The journal was written by an engine speaking a schema this one does
    /// not know.
    #[error("run journal at {path} has schema {found}, this engine reads schema {supported}")]
    UnknownSchema {
        /// The journal path.
        path: PathBuf,
        /// The schema the file declares.
        found: u32,
        /// The schema this build understands ([`RUNSTATE_SCHEMA`]).
        supported: u32,
    },
    /// The journal in this night's directory records a different repo, date
    /// or night id, so it cannot be this night's.
    #[error(
        "run journal at {path} belongs to another night \
         (repo {repo:?}, date {date:?}, night {night_id:?})"
    )]
    WrongNight {
        /// The journal path.
        path: PathBuf,
        /// Repo recorded in the journal.
        repo: String,
        /// Date recorded in the journal.
        date: String,
        /// Night id recorded in the journal.
        night_id: String,
    },
}

fn path(dir: &Path) -> PathBuf {
    dir.join("run-state.json")
}

/// Why a [`NightLock`] could not be taken.
#[derive(Debug, Error)]
pub enum LockError {
    /// Another engine (or another holder in this process) has the night.
    #[error("night is locked by another engine: {path}")]
    Held {
        /// The lock file.
        path: PathBuf,
    },
    /// The lock file could not be created or locked.
    #[error("night lock {path}: {source}")]
    Io {
        /// The lock file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// An exclusive advisory lock on one night directory.
///
/// Two engines that both read an absent journal would otherwise both run the
/// night and both publish. Take the lock **before** the first journal read and
/// hold it across check → freeze → execute → persist; dropping it (or the
/// process dying, however abruptly) releases it. The lock is `flock`-style on
/// `<night_dir>/.lock` ([`File::try_lock`]), so it is per open file
/// description: a second [`NightLock::acquire`] in the same process conflicts
/// exactly like one from another process.
#[derive(Debug)]
pub struct NightLock {
    // Held for its Drop: closing the descriptor releases the lock.
    _file: File,
    path: PathBuf,
}

impl NightLock {
    /// Take the lock for `dir` (created if absent) without blocking.
    ///
    /// # Errors
    ///
    /// [`LockError::Held`] when someone else holds it; [`LockError::Io`]
    /// otherwise.
    pub fn acquire(dir: &Path) -> Result<Self, LockError> {
        let lock_path = dir.join(".lock");
        let io = |source| LockError::Io {
            path: lock_path.clone(),
            source,
        };
        std::fs::create_dir_all(dir).map_err(io)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(io)?;
        match file.try_lock() {
            Ok(()) => Ok(Self {
                _file: file,
                path: lock_path,
            }),
            Err(std::fs::TryLockError::WouldBlock) => Err(LockError::Held { path: lock_path }),
            Err(std::fs::TryLockError::Error(e)) => Err(io(e)),
        }
    }

    /// Like [`NightLock::acquire`], but keeps retrying a held lock for up to
    /// `wait`.
    ///
    /// A descriptor is briefly duplicated into any child forked by another
    /// thread before it execs, so a lock that was *just* released can look held
    /// for a few milliseconds. A real holder keeps the night for the whole run,
    /// so a short wait costs nothing and removes the false "held".
    ///
    /// # Errors
    ///
    /// As [`NightLock::acquire`], after `wait` has elapsed.
    pub fn acquire_within(dir: &Path, wait: std::time::Duration) -> Result<Self, LockError> {
        let deadline = std::time::Instant::now() + wait;
        loop {
            match Self::acquire(dir) {
                Err(LockError::Held { .. }) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other,
            }
        }
    }

    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn save(dir: &Path, state: &RunState) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(&path(dir), &bytes)
}

fn save_checked(dir: &Path, state: &RunState) -> Result<(), JournalError> {
    save(dir, state).map_err(|source| JournalError::Io {
        path: path(dir),
        source,
    })
}

/// Read the journal for `dir`, distinguishing **absent** from **unusable**.
///
/// `Ok(None)` means there is no journal. A file that cannot be read, does not
/// parse, or declares a schema other than [`RUNSTATE_SCHEMA`] is an error: a
/// damaged record must never read as a fresh night, because that reopens a
/// finished one and resets its attempt budget. Unknown JSON fields are
/// tolerated, so every journal an earlier build wrote still loads.
pub fn load_checked(dir: &Path) -> Result<Option<RunState>, JournalError> {
    let p = path(dir);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(JournalError::Io { path: p, source }),
    };
    // Read the version first so a future schema reports as such rather than
    // as a parse failure of a shape this build does not know.
    #[derive(Deserialize)]
    struct Probe {
        schema: Option<u32>,
    }
    let corrupt = |e: serde_json::Error| JournalError::Corrupt {
        path: p.clone(),
        detail: e.to_string(),
    };
    let probe: Probe = serde_json::from_str(&text).map_err(corrupt)?;
    match probe.schema {
        Some(RUNSTATE_SCHEMA) => {}
        Some(found) => {
            return Err(JournalError::UnknownSchema {
                path: p,
                found,
                supported: RUNSTATE_SCHEMA,
            })
        }
        None => {
            return Err(JournalError::Corrupt {
                path: p,
                detail: "no schema field".into(),
            })
        }
    }
    serde_json::from_str(&text).map(Some).map_err(corrupt)
}

/// Read the journal for `dir`, if one exists and is valid.
///
/// A thin compatibility wrapper over [`load_checked`] that folds every error
/// into `None`. Do not use it to decide whether a night may run: it cannot
/// tell a missing journal from a damaged one.
pub fn load(dir: &Path) -> Option<RunState> {
    load_checked(dir).ok().flatten()
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
/// manifest should ask this first. `Ok(None)` for any other state, including
/// an absent journal or one for a different night.
///
/// # Errors
///
/// A journal that exists but is unreadable, corrupt or of an unknown schema
/// ([`JournalError`]): "not finished" cannot be concluded from a record that
/// cannot be read.
pub fn finished_night(
    dir: &Path,
    repo: &str,
    date: &str,
    night_id: &str,
) -> Result<Option<RunState>, JournalError> {
    let Some(mut prior) = load_checked(dir)? else {
        return Ok(None);
    };
    if prior.repo != repo || prior.date != date || prior.night_id != night_id {
        return Ok(None);
    }
    match prior.phase {
        Phase::Complete => Ok(Some(prior)),
        Phase::Persisted => {
            prior.phase = Phase::Complete;
            prior.updated_at = now();
            // Best effort: the answer does not depend on the write landing.
            let _ = save(dir, &prior);
            Ok(Some(prior))
        }
        _ => Ok(None),
    }
}

static ARCHIVE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Archive (never delete, never overwrite) the night's run-state and manifest
/// as `run-state-superseded-<stamp>.json` / `manifest-superseded-<stamp>.json`,
/// so the next [`begin`] is fresh. This is the operator's explicit way to run a
/// different experiment on a night that already finished, and to clear a
/// journal [`begin`] refused as corrupt. Absent files are skipped, so
/// resetting an untouched night is a no-op.
///
/// The stamp is UTC nanoseconds plus the process id and a counter, and each
/// archive is created by hard link, which fails rather than replacing an
/// existing name. Both files are archived or neither is: if the second cannot
/// be, the first archive is removed and the night is left as it was. The night
/// lock ([`NightLock`]) is taken for the duration; a running engine makes this
/// fail with [`std::io::ErrorKind::WouldBlock`].
pub fn reset_night(dir: &Path) -> std::io::Result<()> {
    let _lock = match NightLock::acquire_within(dir, std::time::Duration::from_millis(500)) {
        Ok(l) => l,
        Err(LockError::Held { path }) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                format!("night is in use by a running engine ({})", path.display()),
            ))
        }
        Err(LockError::Io { source, .. }) => return Err(source),
    };
    let stamp = format!(
        "{}-{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%9fZ"),
        std::process::id(),
        ARCHIVE_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new(); // (source, archive)
    for stem in ["run-state", "manifest"] {
        let from = dir.join(format!("{stem}.json"));
        if std::fs::symlink_metadata(&from).is_err() {
            continue;
        }
        let to = dir.join(format!("{stem}-superseded-{stamp}.json"));
        // `hard_link` fails if `to` exists; unlike `rename` it never replaces.
        if let Err(e) = std::fs::hard_link(&from, &to) {
            for (_, archive) in &done {
                let _ = std::fs::remove_file(archive);
            }
            return Err(e);
        }
        done.push((from, to));
    }
    // Every archive exists; now remove the sources. If one cannot be removed,
    // put back what was removed and drop the archives, leaving the night as it was.
    for (i, (from, _)) in done.iter().enumerate() {
        if let Err(e) = std::fs::remove_file(from) {
            for (src, archive) in &done[..i] {
                let _ = std::fs::hard_link(archive, src);
            }
            for (_, archive) in &done {
                let _ = std::fs::remove_file(archive);
            }
            return Err(e);
        }
    }
    Ok(())
}

/// Open (or reopen) the journal for a run, deciding what this attempt is.
///
/// A night that already reached `Persisted`/`Complete` is
/// [`Resume::AlreadyComplete`] whatever `run_id` says (see [`finished_night`]).
///
/// The attempt budget belongs to the **night**, not the run id: a journal for
/// this night with a different run id (the baseline moved between attempts)
/// starts a fresh experiment in phase `Initialised` but carries the attempt
/// count forward, and an exhausted or `Abandoned` night stays abandoned. A
/// run id that changes on every failure therefore cannot loop forever.
///
/// Writes the updated record before returning, so a crash immediately after
/// this call still leaves the attempt counted — an interrupted run can never
/// masquerade as a fresh one.
///
/// # Errors
///
/// Any [`JournalError`]: a journal that is unreadable, corrupt, of an unknown
/// schema, or that records a different repo/date/night id than the one asked
/// for. The file is left untouched. Callers should report `BLOCKED-ENV` and
/// point at `reset-night`; never fall back to a fresh run.
pub fn begin(
    dir: &Path,
    run_id: &str,
    night_id: &str,
    repo: &str,
    date: &str,
    max_attempts: u32,
) -> Result<Resume, JournalError> {
    std::fs::create_dir_all(dir).map_err(|source| JournalError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let prior = load_checked(dir)?;
    if let Some(p) = &prior {
        if p.repo != repo || p.date != date || p.night_id != night_id {
            return Err(JournalError::WrongNight {
                path: path(dir),
                repo: p.repo.clone(),
                date: p.date.clone(),
                night_id: p.night_id.clone(),
            });
        }
    }
    if let Some(done) = finished_night(dir, repo, date, night_id)? {
        return Ok(Resume::AlreadyComplete(done));
    }
    let Some(prior) = prior else {
        let s = new_state(run_id, night_id, repo, date, 1, max_attempts, None);
        save_checked(dir, &s)?;
        return Ok(Resume::Fresh(s));
    };
    if prior.phase == Phase::Abandoned || prior.attempts >= max_attempts {
        let mut s = prior;
        s.phase = Phase::Abandoned;
        s.updated_at = now();
        save_checked(dir, &s)?;
        return Ok(Resume::Abandoned(s));
    }
    if prior.run_id == run_id {
        let mut s = prior;
        s.resumed_from = Some(s.phase);
        s.attempts += 1;
        s.max_attempts = max_attempts;
        s.updated_at = now();
        s.last_error = None;
        save_checked(dir, &s)?;
        Ok(Resume::Resumed(s))
    } else {
        // A different experiment on the same night (the baseline or the
        // config moved); the manifest freeze has already archived the
        // superseded document. The attempt budget is the night's.
        let s = new_state(
            run_id,
            night_id,
            repo,
            date,
            prior.attempts + 1,
            max_attempts,
            Some(prior.first_seen),
        );
        save_checked(dir, &s)?;
        Ok(Resume::Fresh(s))
    }
}

fn new_state(
    run_id: &str,
    night_id: &str,
    repo: &str,
    date: &str,
    attempts: u32,
    max_attempts: u32,
    first_seen: Option<String>,
) -> RunState {
    RunState {
        schema: RUNSTATE_SCHEMA,
        run_id: run_id.into(),
        night_id: night_id.into(),
        repo: repo.into(),
        date: date.into(),
        phase: Phase::Initialised,
        attempts,
        max_attempts,
        resumed_from: None,
        first_seen: first_seen.unwrap_or_else(now),
        updated_at: now(),
        last_error: None,
        verdict: None,
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
    fn a_different_run_id_starts_a_fresh_experiment_but_keeps_the_attempt_count() {
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
        assert_eq!(r.state().phase, Phase::Initialised);
        assert_eq!(r.state().attempts, 2, "the attempt budget is the night's");
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
    fn a_corrupt_journal_blocks_the_night_instead_of_reopening_it() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(path(d.path()), "{ this is not json").unwrap();
        let e = begin(d.path(), RUN, "2026-09-05-alpha", "alpha", "2026-09-05", 2).unwrap_err();
        assert!(matches!(e, JournalError::Corrupt { .. }), "{e:?}");
        assert!(
            load(d.path()).is_none(),
            "the compatibility wrapper reads it as absent"
        );
        assert_eq!(
            std::fs::read_to_string(path(d.path())).unwrap(),
            "{ this is not json",
            "never overwritten"
        );
    }

    // ---- audit F5 / F8 (written against the old API shape: `.is_err()` and
    // `.unwrap()` compile either way) ----

    #[test]
    fn f5_a_damaged_journal_is_an_error_not_a_fresh_night() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        complete(d.path(), &mut s, "ACCEPT").unwrap();
        let good = std::fs::read(path(d.path())).unwrap();
        std::fs::write(path(d.path()), &good[..good.len() / 2]).unwrap(); // truncated
        let e = begin(d.path(), RUN, "2026-09-05-alpha", "alpha", "2026-09-05", 2).unwrap_err();
        assert!(matches!(e, JournalError::Corrupt { .. }), "{e:?}");
        assert!(
            finished_night(d.path(), "alpha", "2026-09-05", "2026-09-05-alpha").is_err(),
            "a damaged journal must not read as 'not finished'"
        );
        assert_eq!(
            std::fs::read(path(d.path())).unwrap(),
            &good[..good.len() / 2],
            "the damaged file is evidence and is never overwritten"
        );
    }

    #[test]
    fn f5_an_unknown_schema_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let _ = begin_default(d.path());
        let text = std::fs::read_to_string(path(d.path())).unwrap();
        std::fs::write(
            path(d.path()),
            text.replace("\"schema\": 1", "\"schema\": 99"),
        )
        .unwrap();
        let e = begin(d.path(), RUN, "2026-09-05-alpha", "alpha", "2026-09-05", 2).unwrap_err();
        assert!(
            matches!(
                e,
                JournalError::UnknownSchema {
                    found: 99,
                    supported: 1,
                    ..
                }
            ),
            "{e:?}"
        );
        assert!(matches!(
            load_checked(d.path()),
            Err(JournalError::UnknownSchema { .. })
        ));
    }

    #[test]
    fn f5_changing_the_run_id_does_not_reset_the_attempt_budget() {
        let d = tempfile::tempdir().unwrap();
        let mut seen = vec![];
        for run in ["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", "cccccccccccccccc"] {
            let r = begin(d.path(), run, "2026-09-05-alpha", "alpha", "2026-09-05", 2).unwrap();
            seen.push((r.should_run(), r.state().attempts));
        }
        assert_eq!(seen, vec![(true, 1), (true, 2), (false, 2)], "{seen:?}");
        assert_eq!(load(d.path()).unwrap().phase, Phase::Abandoned);
    }

    #[test]
    fn f5_a_journal_for_another_repo_date_or_night_is_never_resumed() {
        for (repo, night, date) in [
            ("beta", "2026-09-05-alpha", "2026-09-05"),
            ("alpha", "2026-09-05-alpha-x", "2026-09-05"),
            ("alpha", "2026-09-05-alpha", "2026-09-06"),
        ] {
            let d = tempfile::tempdir().unwrap();
            let _ = begin_default(d.path());
            assert!(
                begin(d.path(), RUN, night, repo, date, 2).is_err(),
                "{repo} {night} {date}"
            );
            assert_eq!(load(d.path()).unwrap().attempts, 1, "journal untouched");
        }
    }

    #[test]
    fn f8_rapid_resets_never_overwrite_an_earlier_archive() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..40 {
            std::fs::write(d.path().join("run-state.json"), format!("rs{i}")).unwrap();
            std::fs::write(d.path().join("manifest.json"), format!("m{i}")).unwrap();
            reset_night(d.path()).unwrap();
        }
        let mut bodies: Vec<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains("superseded"))
            .map(|p| std::fs::read_to_string(p).unwrap())
            .collect();
        bodies.sort();
        bodies.dedup();
        assert_eq!(bodies.len(), 80, "an archive was overwritten");
    }

    #[test]
    fn f8_a_reset_that_cannot_archive_both_files_archives_neither() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("run-state.json"), "rs").unwrap();
        // A directory cannot be archived by link; the first file must be rolled back.
        std::fs::create_dir(d.path().join("manifest.json")).unwrap();
        assert!(reset_night(d.path()).is_err());
        assert!(
            d.path().join("run-state.json").exists(),
            "first rename not rolled back"
        );
        let archives = std::fs::read_dir(d.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("superseded")
            })
            .count();
        assert_eq!(archives, 0);
    }

    // ---- audit F3: the per-night lock ----

    #[test]
    fn f3_a_second_acquirer_gets_held_and_the_lock_releases_on_drop() {
        let d = tempfile::tempdir().unwrap();
        let first = NightLock::acquire(d.path()).unwrap();
        assert_eq!(first.path(), d.path().join(".lock"));
        let second = NightLock::acquire(d.path());
        assert!(matches!(second, Err(LockError::Held { .. })), "{second:?}");
        // reset_night refuses while an engine holds the night.
        let e = reset_night(d.path()).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock);
        drop(first);
        let again = NightLock::acquire_within(d.path(), std::time::Duration::from_secs(5));
        assert!(again.is_ok(), "{again:?}");
    }

    #[test]
    fn f3_nights_lock_independently() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let _la = NightLock::acquire(a.path()).unwrap();
        assert!(NightLock::acquire(b.path()).is_ok());
    }

    /// A real second process: the child re-executes this test binary and tries
    /// the lock the parent holds.
    #[test]
    fn f3_a_child_process_is_refused_while_the_parent_holds_the_night() {
        if let Ok(dir) = std::env::var("DREAM_LOCK_CHILD_DIR") {
            // Child: report what happened through the exit code.
            let held = matches!(
                NightLock::acquire(Path::new(&dir)),
                Err(LockError::Held { .. })
            );
            std::process::exit(if held { 42 } else { 1 });
        }
        let d = tempfile::tempdir().unwrap();
        let me = std::env::current_exe().unwrap();
        let run_child = || {
            std::process::Command::new(&me)
                .args([
                    "--exact",
                    "runstate::tests::f3_a_child_process_is_refused_while_the_parent_holds_the_night",
                    "--nocapture",
                ])
                .env("DREAM_LOCK_CHILD_DIR", d.path())
                .status()
                .unwrap()
                .code()
        };
        let lock = NightLock::acquire(d.path()).unwrap();
        assert_eq!(run_child(), Some(42), "child must see the lock as held");
        drop(lock);
        // A sibling test thread forking at this instant can briefly duplicate
        // the released descriptor; a few retries rule that out.
        let mut code = run_child();
        for _ in 0..50 {
            if code == Some(1) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            code = run_child();
        }
        assert_eq!(code, Some(1), "child acquires once the parent lets go");
    }

    // ---- audit F5: journals the previous binary wrote must still load ----

    #[test]
    fn f5_every_journal_shape_the_old_binary_wrote_still_loads_and_behaves() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/journals");
        let mut seen = 0;
        for entry in std::fs::read_dir(&fixtures).unwrap() {
            let f = entry.unwrap().path();
            let d = tempfile::tempdir().unwrap();
            std::fs::copy(&f, path(d.path())).unwrap();
            let loaded = load_checked(d.path())
                .unwrap()
                .unwrap_or_else(|| panic!("{f:?}"));
            assert_eq!(loaded.schema, RUNSTATE_SCHEMA);
            // begin() for the very night the fixture records must not error.
            let r = begin(
                d.path(),
                &loaded.run_id,
                &loaded.night_id,
                &loaded.repo,
                &loaded.date,
                loaded.max_attempts,
            )
            .unwrap_or_else(|e| panic!("{f:?}: {e}"));
            match loaded.phase {
                Phase::Complete | Phase::Persisted => {
                    assert!(matches!(r, Resume::AlreadyComplete(_)), "{f:?}: {r:?}")
                }
                _ => assert!(matches!(r, Resume::Resumed(_)), "{f:?}: {r:?}"),
            }
            seen += 1;
        }
        assert!(seen >= 6, "fixtures missing ({seen})");
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
    fn a_journal_that_records_another_night_is_refused_not_overwritten() {
        let d = tempfile::tempdir().unwrap();
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        complete(d.path(), &mut s, "ACCEPT").unwrap();
        // finished_night alone still just says "not this night".
        assert!(
            finished_night(d.path(), "alpha", "2026-09-06", "2026-09-06-alpha")
                .unwrap()
                .is_none()
        );
        for (night, date) in [
            ("2026-09-06-alpha", "2026-09-06"),
            ("2026-09-05-alpha-deep2", "2026-09-05"),
        ] {
            let e = begin(d.path(), OTHER, night, "alpha", date, 2).unwrap_err();
            assert!(matches!(e, JournalError::WrongNight { .. }), "{e:?}");
        }
        assert_eq!(load(d.path()).unwrap().phase, Phase::Complete);
    }

    #[test]
    fn an_abandoned_night_stays_abandoned_when_the_baseline_moves() {
        let d = tempfile::tempdir().unwrap();
        let _ = begin_default(d.path());
        let _ = begin_default(d.path());
        assert!(matches!(begin_default(d.path()), Resume::Abandoned(_)));
        // The budget is the night's: a new run id does not revive it.
        let r = begin_as(d.path(), OTHER, "2026-09-05-alpha", "2026-09-05");
        assert!(matches!(r, Resume::Abandoned(_)), "got {r:?}");
    }

    #[test]
    fn finished_night_reports_without_touching_an_unfinished_one() {
        let d = tempfile::tempdir().unwrap();
        let fin = |d: &Path| finished_night(d, "alpha", "2026-09-05", "2026-09-05-alpha").unwrap();
        assert!(fin(d.path()).is_none());
        let Resume::Fresh(mut s) = begin_default(d.path()) else {
            panic!()
        };
        advance(d.path(), &mut s, Phase::Gated).unwrap();
        assert!(fin(d.path()).is_none());
        complete(d.path(), &mut s, "REJECT").unwrap();
        let f = fin(d.path()).unwrap();
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
