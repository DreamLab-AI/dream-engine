//! Forward-looking replay: capture a night's gate inputs, re-run the pure gate.
//!
//! Every judgement [`dream_gate::gate::decide`] makes is a pure function of its
//! inputs. This module persists those inputs for each night in a stable on-disk
//! schema, and [`verify`] recomputes the verdict from them and diffs it against
//! the verdict that was recorded. Nothing is trusted from the recorded
//! `decision.json`: a tampered receipt, report, config or decision shows up as a
//! mismatch or a refusal.
//!
//! **No historical corpus exists.** Replay covers nights captured from the day
//! this module is wired in; earlier nights were never recorded in this shape and
//! this module does not pretend to reconstruct them.
//!
//! # Layout of a run directory (schema 1)
//!
//! | file | content |
//! |---|---|
//! | `manifest.json` | the frozen [`ExperimentManifest`] ([`dream_gate::manifest::freeze`]) |
//! | `receipts/baseline/`, `receipts/candidate/` | typed receipts and raw streams ([`dream_gate::receipts::persist`]) |
//! | `report.md` | the model's report, verbatim; its digest is in `replay.json` |
//! | `config.json` | the baseline config bytes (digest-checked against the manifest) |
//! | `candidate.patch` | the patch the candidate state refers to (the applied text for an applied candidate); absent when there was none |
//! | `replay.json` | candidate state, per-test evidence, options, report digest |
//! | `decision.json` | the recorded [`GateDecision`] |
//!
//! # Example
//!
//! ```
//! use dream_core::replay::{capture, verify, NightInputs};
//! # use dream_gate::{gate::CandidateState, manifest::{digest, EvaluatorIdentity, ExperimentManifest, ModelIdentity}, pertest::PairwiseOptions};
//! # let config = br#"{"repo":"o/r","slots":[{"deep":"d","scan":[]}]}"#.to_vec();
//! # let manifest = ExperimentManifest { schema: 1, run_id: "abcdef0123456789".into(), night_id: "n".into(), repo: "r".into(),
//! #   repo_slug: "o/r".into(), date: "2026-10-01".into(), day_int: 20261001, deep: "d".into(), scan: vec![],
//! #   baseline_revision: "0".repeat(40), baseline_tree_hash: "1".repeat(40), config_digest: digest(&config),
//! #   evaluators: vec![], model: ModelIdentity { provider: "p".into(), model: "m".into(), max_tokens: 1, fallback: None },
//! #   engine_version: "0".into(), created_at: "2026-10-01T00:00:00Z".into() };
//! let night = NightInputs {
//!     manifest, report: "nothing to change\nVERDICT: REJECT\n".into(), candidate: CandidateState::NotAttempted,
//!     extracted_patch: None, baseline_receipts: vec![], candidate_receipts: vec![], config_bytes: config,
//!     fallback_ledger_path: String::new(), candidate_config_digest: None, tests: Default::default(),
//!     options: PairwiseOptions::default(),
//! };
//! let dir = tempfile::tempdir()?;
//! let decision = capture(dir.path(), &night)?;
//! assert_eq!(decision.verdict, "REJECT");
//! assert!(verify(dir.path())?.matches);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use dream_gate::gate::{self, CandidateState, GateContext, GateDecision};
use dream_gate::guard::{self, GuardViolation, PolicyError, ProtectedPolicy};
use dream_gate::manifest::{self, digest, write_atomic, ExperimentManifest};
use dream_gate::pertest::{PairwiseOptions, TestOutcomes};
use dream_gate::outcome::ExecOutcome;
use dream_gate::receipts::{self, classify, EvaluatorOutcome, EvaluatorReceipt, Phase};
use dream_gate::verdict::parse_verdict_strict;
use serde::{Deserialize, Serialize};

/// Version of the on-disk layout written by [`capture`].
pub const REPLAY_SCHEMA: u32 = 1;

/// Why capturing or verifying a night failed.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// Filesystem trouble.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A record is not valid JSON for its schema.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// A required file is absent from the run directory.
    #[error("missing {0} in the run directory")]
    Missing(String),
    /// The record was written by a layout this crate does not read.
    #[error("unsupported replay schema {0}")]
    UnsupportedSchema(u32),
    /// `report.md` no longer hashes to the digest recorded in `replay.json`.
    #[error("report.md does not match the recorded digest")]
    ReportDigestMismatch,
    /// The config bytes are not the document the manifest froze, or are invalid.
    #[error("protected policy: {0}")]
    Policy(#[from] PolicyError),
    /// The candidate state is not one this schema can record.
    #[error("unsupported candidate state: {0}")]
    UnsupportedState(String),
}

/// Everything the gate judged a night on.
#[derive(Debug, Clone)]
pub struct NightInputs {
    /// The frozen experiment.
    pub manifest: ExperimentManifest,
    /// The model's full report.
    pub report: String,
    /// The structural state of the candidate.
    pub candidate: CandidateState,
    /// For a candidate that was *not* applied (for example a protected
    /// patch), the patch as extracted, kept for the guard cross-check. Ignored
    /// for an applied candidate, whose applied text is in the state.
    pub extracted_patch: Option<String>,
    /// Receipts of the baseline run.
    pub baseline_receipts: Vec<EvaluatorReceipt>,
    /// Receipts of the candidate re-run.
    pub candidate_receipts: Vec<EvaluatorReceipt>,
    /// The baseline config bytes (the document `manifest.config_digest` froze).
    pub config_bytes: Vec<u8>,
    /// Ledger path protected when the config names none.
    pub fallback_ledger_path: String,
    /// Digest of the config the candidate tree was evaluated with.
    pub candidate_config_digest: Option<String>,
    /// Per-test evidence by evaluator name.
    pub tests: BTreeMap<String, TestOutcomes>,
    /// Pairwise policies applied to that evidence.
    pub options: PairwiseOptions,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum RecordedCandidate {
    #[serde(rename_all = "camelCase")]
    Applied { tree_hash: String },
    NoPatch,
    DidNotApply { detail: String },
    Refused { detail: String },
    Protected { paths: Vec<String> },
    NotAttempted,
}

impl RecordedCandidate {
    fn from_state(s: &CandidateState) -> Result<Self, ReplayError> {
        Ok(match s {
            CandidateState::Applied { tree_hash, .. } => Self::Applied {
                tree_hash: tree_hash.clone(),
            },
            CandidateState::NoPatch => Self::NoPatch,
            CandidateState::DidNotApply { detail } => Self::DidNotApply {
                detail: detail.clone(),
            },
            CandidateState::Refused { detail } => Self::Refused {
                detail: detail.clone(),
            },
            CandidateState::Protected { paths } => Self::Protected {
                paths: paths.clone(),
            },
            CandidateState::NotAttempted => Self::NotAttempted,
            other => return Err(ReplayError::UnsupportedState(format!("{other:?}"))),
        })
    }

    fn into_state(self, patch: Option<String>) -> Result<CandidateState, ReplayError> {
        Ok(match self {
            Self::Applied { tree_hash } => CandidateState::Applied {
                tree_hash,
                patch: patch.ok_or_else(|| ReplayError::Missing("candidate.patch".into()))?,
            },
            Self::NoPatch => CandidateState::NoPatch,
            Self::DidNotApply { detail } => CandidateState::DidNotApply { detail },
            Self::Refused { detail } => CandidateState::Refused { detail },
            Self::Protected { paths } => CandidateState::Protected { paths },
            Self::NotAttempted => CandidateState::NotAttempted,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplayRecord {
    schema: u32,
    report_digest: String,
    fallback_ledger_path: String,
    candidate: RecordedCandidate,
    candidate_config_digest: Option<String>,
    tests: BTreeMap<String, TestOutcomes>,
    options: PairwiseOptions,
}

fn policy_for(inputs: &NightInputs) -> Result<ProtectedPolicy, ReplayError> {
    Ok(ProtectedPolicy::for_manifest(
        &inputs.manifest,
        &inputs.config_bytes,
        &inputs.fallback_ledger_path,
    )?)
}

/// Run the gate on recorded inputs. This is the one function both
/// [`capture`] and [`verify`] judge with.
pub fn decide(inputs: &NightInputs) -> Result<GateDecision, ReplayError> {
    let policy = policy_for(inputs)?;
    let strict = parse_verdict_strict(&inputs.report);
    let mut ctx = GateContext::new(&policy).with_options(inputs.options.clone());
    if let Some(d) = &inputs.candidate_config_digest {
        ctx = ctx.with_config_digest(d.clone());
    }
    for (name, t) in &inputs.tests {
        ctx = ctx.with_tests(name.clone(), t.clone());
    }
    Ok(gate::decide(
        &inputs.manifest,
        &strict,
        &inputs.candidate,
        &inputs.candidate_receipts,
        &ctx,
    ))
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), ReplayError> {
    write_atomic(path, &serde_json::to_vec_pretty(v)?)?;
    Ok(())
}

/// Judge the night with [`decide`] and persist its inputs and decision under
/// `run_dir` (see the [module layout](self)). Returns the decision; nothing is
/// written if judging fails. Idempotent for a restarted night.
pub fn capture(run_dir: &Path, inputs: &NightInputs) -> Result<GateDecision, ReplayError> {
    let decision = decide(inputs)?;
    let recorded = RecordedCandidate::from_state(&inputs.candidate)?;
    std::fs::create_dir_all(run_dir)?;
    manifest::freeze(run_dir, &inputs.manifest).map_err(|e| match e {
        manifest::ManifestError::Io(e) => ReplayError::Io(e),
        other => ReplayError::Io(std::io::Error::other(other.to_string())),
    })?;
    receipts::persist(run_dir, Phase::Baseline, &inputs.baseline_receipts)?;
    receipts::persist(run_dir, Phase::Candidate, &inputs.candidate_receipts)?;
    write_atomic(&run_dir.join("report.md"), inputs.report.as_bytes())?;
    write_atomic(&run_dir.join("config.json"), &inputs.config_bytes)?;
    let patch = match &inputs.candidate {
        CandidateState::Applied { patch, .. } => Some(patch.as_str()),
        _ => inputs.extracted_patch.as_deref(),
    };
    match patch {
        Some(p) => write_atomic(&run_dir.join("candidate.patch"), p.as_bytes())?,
        None => {
            let _ = std::fs::remove_file(run_dir.join("candidate.patch"));
        }
    }
    write_json(
        &run_dir.join("replay.json"),
        &ReplayRecord {
            schema: REPLAY_SCHEMA,
            report_digest: digest(inputs.report.as_bytes()),
            fallback_ledger_path: inputs.fallback_ledger_path.clone(),
            candidate: recorded,
            candidate_config_digest: inputs.candidate_config_digest.clone(),
            tests: inputs.tests.clone(),
            options: inputs.options.clone(),
        },
    )?;
    write_json(&run_dir.join("decision.json"), &decision)?;
    Ok(decision)
}

fn read(run_dir: &Path, name: &str) -> Result<Vec<u8>, ReplayError> {
    std::fs::read(run_dir.join(name)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ReplayError::Missing(name.to_string())
        } else {
            ReplayError::Io(e)
        }
    })
}

/// Read a captured night back: its inputs and the decision recorded for it.
pub fn load(run_dir: &Path) -> Result<(NightInputs, GateDecision), ReplayError> {
    let rec: ReplayRecord = serde_json::from_slice(&read(run_dir, "replay.json")?)?;
    if rec.schema != REPLAY_SCHEMA {
        return Err(ReplayError::UnsupportedSchema(rec.schema));
    }
    let manifest =
        manifest::load(run_dir).ok_or_else(|| ReplayError::Missing("manifest.json".into()))?;
    let report = String::from_utf8_lossy(&read(run_dir, "report.md")?).into_owned();
    if digest(report.as_bytes()) != rec.report_digest {
        return Err(ReplayError::ReportDigestMismatch);
    }
    let config_bytes = read(run_dir, "config.json")?;
    let patch = std::fs::read_to_string(run_dir.join("candidate.patch")).ok();
    let is_applied = matches!(rec.candidate, RecordedCandidate::Applied { .. });
    let candidate = rec.candidate.into_state(patch.clone())?;
    let decision: GateDecision = serde_json::from_slice(&read(run_dir, "decision.json")?)?;
    Ok((
        NightInputs {
            manifest,
            report,
            candidate,
            extracted_patch: if is_applied { None } else { patch },
            baseline_receipts: receipts::load(run_dir, Phase::Baseline),
            candidate_receipts: receipts::load(run_dir, Phase::Candidate),
            config_bytes,
            fallback_ledger_path: rec.fallback_ledger_path,
            candidate_config_digest: rec.candidate_config_digest,
            tests: rec.tests,
            options: rec.options,
        },
        decision,
    ))
}

/// The result of re-judging a captured night.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// The decision found in `decision.json`.
    pub recorded: GateDecision,
    /// The decision [`decide`] reaches from the stored inputs.
    pub recomputed: GateDecision,
    /// Whether they agree on every field **and** the guard cross-check holds.
    pub matches: bool,
    /// One line per disagreeing field (`field: recorded X, recomputed Y`) and
    /// per receipt whose typed outcome contradicts its own exit status and
    /// streams.
    pub diffs: Vec<String>,
    /// For a night with a stored patch: whether re-running
    /// [`dream_gate::guard::check`] on it agrees with the recorded candidate
    /// state (an applied candidate's patch must pass; a protected one must be
    /// refused for the same paths). `None` when there is no patch to check.
    pub guard_agrees: Option<bool>,
}

fn field_diff<T: Serialize>(name: &str, a: &T, b: &T, out: &mut Vec<String>) {
    let (sa, sb) = (
        serde_json::to_string(a).unwrap_or_default(),
        serde_json::to_string(b).unwrap_or_default(),
    );
    if sa != sb {
        out.push(format!("{name}: recorded {sa}, recomputed {sb}"));
    }
}

/// Re-derive a receipt's typed outcome from the raw execution it records and
/// report a disagreement. The gate reads the typed outcome, so a receipt whose
/// exit status or streams were edited after the fact would otherwise pass
/// unnoticed.
fn receipt_inconsistency(r: &EvaluatorReceipt) -> Option<String> {
    if r.outcome == EvaluatorOutcome::Missing {
        return None;
    }
    let no_status = "process produced no exit status";
    let exec = ExecOutcome {
        exit_code: r.exit_code,
        stdout: r.stdout.clone(),
        stderr: r.stderr.clone(),
        duration_ms: r.duration_ms,
        transport_error: match &r.outcome {
            EvaluatorOutcome::Blocked { detail } if detail != no_status => Some(detail.clone()),
            _ => None,
        },
        timed_out: matches!(r.outcome, EvaluatorOutcome::TimedOut { .. }),
        started_at: r.started_at.clone(),
    };
    let again = classify(&exec, r.timeout_secs);
    (again != r.outcome).then(|| {
        format!(
            "receipt {}/{}: recorded outcome {} but its exit status and streams give {}",
            r.phase.as_str(),
            r.name,
            r.outcome.label(),
            again.label()
        )
    })
}

/// Re-judge the night in `run_dir` and compare with what was recorded.
pub fn verify(run_dir: &Path) -> Result<Verification, ReplayError> {
    let (inputs, recorded) = load(run_dir)?;
    let recomputed = decide(&inputs)?;
    let mut diffs = Vec::new();
    field_diff("verdict", &recorded.verdict, &recomputed.verdict, &mut diffs);
    field_diff("accepted", &recorded.accepted, &recomputed.accepted, &mut diffs);
    field_diff("modelVerdict", &recorded.model_verdict, &recomputed.model_verdict, &mut diffs);
    field_diff("vetoes", &recorded.vetoes, &recomputed.vetoes, &mut diffs);
    field_diff(
        "requiredOutcomes",
        &recorded.required_outcomes,
        &recomputed.required_outcomes,
        &mut diffs,
    );
    field_diff("summary", &recorded.summary, &recomputed.summary, &mut diffs);
    for r in inputs
        .baseline_receipts
        .iter()
        .chain(inputs.candidate_receipts.iter())
    {
        diffs.extend(receipt_inconsistency(r));
    }

    let policy = policy_for(&inputs)?;
    let stored = match &inputs.candidate {
        CandidateState::Applied { patch, .. } => Some(patch.as_str()),
        _ => inputs.extracted_patch.as_deref(),
    };
    let guard_agrees = stored.map(|p| match (&inputs.candidate, guard::check(p, &policy)) {
        (CandidateState::Applied { .. }, Ok(_)) => true,
        (CandidateState::Protected { paths }, Err(GuardViolation::Protected { paths: now })) => {
            let (mut a, mut b) = (paths.clone(), now);
            a.sort();
            b.sort();
            a == b
        }
        // Other states carry no claim about the guard; only a contradiction
        // of an Applied or Protected claim is disagreement.
        (CandidateState::Applied { .. } | CandidateState::Protected { .. }, _) => false,
        _ => true,
    });
    let matches = diffs.is_empty() && guard_agrees != Some(false);
    Ok(Verification {
        recorded,
        recomputed,
        matches,
        diffs,
        guard_agrees,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dream_gate::gate::CandidateState;
    use dream_gate::manifest::{digest, EvaluatorIdentity, ExperimentManifest, ModelIdentity};
    use dream_gate::outcome::ExecOutcome;
    use dream_gate::pertest::PairwiseOptions;
    use dream_gate::receipts::{EvaluatorReceipt, Phase};

    const CONFIG: &str = r#"{"repo":"o/r","slots":[{"deep":"d","scan":[]}],"ledgerPath":"docs/LEDGER.md","evaluatorEntrypoints":{"check":"bash scripts/check.sh"}}"#;
    const BENIGN: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n";
    const PROTECTED: &str = "diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml\n--- a/.github/workflows/ci.yml\n+++ b/.github/workflows/ci.yml\n@@ -1 +1 @@\n-a\n+b\n";

    fn manifest() -> ExperimentManifest {
        ExperimentManifest {
            schema: 1,
            run_id: "abcdef0123456789".into(),
            night_id: "2026-10-01-alpha".into(),
            repo: "alpha".into(),
            repo_slug: "o/r".into(),
            date: "2026-10-01".into(),
            day_int: 20261001,
            deep: "d".into(),
            scan: vec![],
            baseline_revision: "0".repeat(40),
            baseline_tree_hash: "1".repeat(40),
            config_digest: digest(CONFIG.as_bytes()),
            evaluators: vec![EvaluatorIdentity {
                name: "check".into(),
                command: "bash scripts/check.sh".into(),
                command_digest: digest(b"bash scripts/check.sh"),
                required: true,
                deeps: vec![],
                timeout_secs: 60,
                test_format: None,
            }],
            model: ModelIdentity {
                provider: "test".into(),
                model: "m".into(),
                max_tokens: 1024,
                fallback: None,
            },
            engine_version: "0.1.0".into(),
            created_at: "2026-10-01T01:00:00Z".into(),
        }
    }

    fn exec(code: Option<i32>, out: &str, transport: Option<&str>) -> ExecOutcome {
        ExecOutcome {
            exit_code: code,
            stdout: out.into(),
            stderr: String::new(),
            duration_ms: 5,
            transport_error: transport.map(str::to_string),
            timed_out: false,
            started_at: "2026-10-01T01:00:00Z".into(),
        }
    }
    fn receipt(phase: Phase, e: ExecOutcome) -> EvaluatorReceipt {
        EvaluatorReceipt::from_exec("check", "bash scripts/check.sh", phase, true, 60, e)
    }
    fn pass(phase: Phase) -> EvaluatorReceipt {
        receipt(phase, exec(Some(0), "check ok\n", None))
    }

    fn inputs(report: &str, candidate: CandidateState, cand: Vec<EvaluatorReceipt>) -> NightInputs {
        let extracted = match &candidate {
            CandidateState::Applied { patch, .. } => Some(patch.clone()),
            CandidateState::Protected { .. } => Some(PROTECTED.to_string()),
            _ => None,
        };
        NightInputs {
            manifest: manifest(),
            report: report.into(),
            candidate,
            extracted_patch: extracted,
            baseline_receipts: vec![pass(Phase::Baseline)],
            candidate_receipts: cand,
            config_bytes: CONFIG.as_bytes().to_vec(),
            fallback_ledger_path: "docs/LEDGER.md".into(),
            candidate_config_digest: None,
            tests: Default::default(),
            options: PairwiseOptions::default(),
        }
    }

    fn applied() -> CandidateState {
        CandidateState::Applied {
            tree_hash: "2".repeat(40),
            patch: BENIGN.into(),
        }
    }

    /// The five synthetic nights, one per verdict class.
    fn nights() -> Vec<(&'static str, NightInputs)> {
        let accept_report = "analysis\nVERDICT: ACCEPT\n";
        vec![
            ("ACCEPT", inputs(accept_report, applied(), vec![pass(Phase::Candidate)])),
            (
                "REJECT",
                inputs(
                    accept_report,
                    applied(),
                    vec![receipt(Phase::Candidate, exec(Some(1), "FAIL\n", None))],
                ),
            ),
            ("INCONCLUSIVE", inputs(accept_report, CandidateState::NoPatch, vec![])),
            (
                "BLOCKED-ENV",
                inputs(
                    accept_report,
                    applied(),
                    vec![receipt(Phase::Candidate, exec(None, "", Some("runner unreachable")))],
                ),
            ),
            (
                "REJECT",
                inputs(
                    accept_report,
                    CandidateState::Protected {
                        paths: vec![".github/workflows/ci.yml".into()],
                    },
                    vec![],
                ),
            ),
        ]
    }

    #[test]
    fn every_verdict_class_replays_to_the_recorded_verdict() {
        for (i, (want, n)) in nights().into_iter().enumerate() {
            let dir = tempfile::tempdir().unwrap();
            let decision = capture(dir.path(), &n).unwrap();
            assert_eq!(decision.verdict, want, "night {i}: {decision:?}");
            let v = verify(dir.path()).unwrap();
            assert!(v.matches, "night {i}: {:?}", v.diffs);
            assert_eq!(v.recorded, decision);
            assert_eq!(v.recomputed, decision);
            assert!(v.diffs.is_empty());
        }
    }

    #[test]
    fn the_protected_night_is_cross_checked_against_the_guard() {
        let (_, n) = nights().remove(4);
        let dir = tempfile::tempdir().unwrap();
        capture(dir.path(), &n).unwrap();
        let v = verify(dir.path()).unwrap();
        assert_eq!(v.guard_agrees, Some(true));
        // Doctor the record so the state says Applied while the patch is protected.
        let mut rec: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("replay.json")).unwrap()).unwrap();
        rec["candidate"] = serde_json::json!({"state": "applied", "treeHash": "2".repeat(40)});
        std::fs::write(dir.path().join("replay.json"), serde_json::to_vec_pretty(&rec).unwrap())
            .unwrap();
        std::fs::write(dir.path().join("candidate.patch"), PROTECTED).unwrap();
        let v = verify(dir.path()).unwrap();
        assert!(!v.matches);
        assert_eq!(v.guard_agrees, Some(false));
    }

    #[test]
    fn a_tampered_decision_is_reported_field_by_field() {
        let (_, n) = nights().remove(1); // REJECT
        let dir = tempfile::tempdir().unwrap();
        capture(dir.path(), &n).unwrap();
        let p = dir.path().join("decision.json");
        let mut d: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        d["verdict"] = "ACCEPT".into();
        d["accepted"] = true.into();
        std::fs::write(&p, serde_json::to_vec_pretty(&d).unwrap()).unwrap();
        let v = verify(dir.path()).unwrap();
        assert!(!v.matches);
        assert!(v.diffs.iter().any(|d| d.starts_with("verdict:")), "{:?}", v.diffs);
        assert!(v.diffs.iter().any(|d| d.starts_with("accepted:")), "{:?}", v.diffs);
    }

    #[test]
    fn a_tampered_receipt_changes_the_recomputed_verdict() {
        let (_, n) = nights().remove(0); // ACCEPT
        let dir = tempfile::tempdir().unwrap();
        capture(dir.path(), &n).unwrap();
        let p = dir.path().join("receipts/candidate/check.json");
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"exitCode\": 0"), "{text}");
        std::fs::write(&p, text.replace("\"exitCode\": 0", "\"exitCode\": 1")).unwrap();
        let v = verify(dir.path()).unwrap();
        assert!(!v.matches, "recomputing must not trust the recorded verdict");
        assert_eq!(v.recorded.verdict, "ACCEPT");
    }

    #[test]
    fn a_tampered_report_or_config_is_refused() {
        let (_, n) = nights().remove(0);
        let dir = tempfile::tempdir().unwrap();
        capture(dir.path(), &n).unwrap();
        std::fs::write(dir.path().join("report.md"), "VERDICT: REJECT\n").unwrap();
        assert!(matches!(verify(dir.path()), Err(ReplayError::ReportDigestMismatch)));

        let dir = tempfile::tempdir().unwrap();
        capture(dir.path(), &n).unwrap();
        std::fs::write(dir.path().join("config.json"), CONFIG.replace("docs/LEDGER.md", "x.md")).unwrap();
        assert!(matches!(verify(dir.path()), Err(ReplayError::Policy(_))));
    }

    #[test]
    fn missing_files_and_unknown_schema_are_errors_not_silent_passes() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(verify(dir.path()), Err(ReplayError::Missing(_))));
        let (_, n) = nights().remove(0);
        capture(dir.path(), &n).unwrap();
        let p = dir.path().join("replay.json");
        let mut rec: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        rec["schema"] = 99.into();
        std::fs::write(&p, serde_json::to_vec(&rec).unwrap()).unwrap();
        assert!(matches!(verify(dir.path()), Err(ReplayError::UnsupportedSchema(99))));
    }

    #[test]
    fn the_on_disk_schema_is_stable() {
        let (_, n) = nights().remove(0);
        let dir = tempfile::tempdir().unwrap();
        capture(dir.path(), &n).unwrap();
        let mut files: Vec<String> = ["manifest.json", "replay.json", "decision.json", "report.md",
            "candidate.patch", "config.json", "receipts/baseline/check.json", "receipts/candidate/check.json"]
            .iter()
            .filter(|f| !dir.path().join(f).exists())
            .map(|f| f.to_string())
            .collect();
        files.sort();
        assert!(files.is_empty(), "missing: {files:?}");
        let rec: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("replay.json")).unwrap()).unwrap();
        let mut keys: Vec<&str> = rec.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["candidate", "candidateConfigDigest", "fallbackLedgerPath", "options", "reportDigest", "schema", "tests"]
        );
        assert_eq!(rec["schema"], 1);
        assert_eq!(rec["candidate"]["state"], "applied");
    }

    #[test]
    fn capture_is_idempotent_for_a_restarted_night() {
        let (_, n) = nights().remove(0);
        let dir = tempfile::tempdir().unwrap();
        let a = capture(dir.path(), &n).unwrap();
        let b = capture(dir.path(), &n).unwrap();
        assert_eq!(a, b);
        assert!(verify(dir.path()).unwrap().matches);
    }

    #[test]
    fn per_test_evidence_and_options_survive_the_round_trip() {
        use dream_gate::pertest::{TestCase, TestOutcomes, TestStatus};
        let (_, mut n) = nights().remove(0);
        n.tests.insert(
            "check".into(),
            TestOutcomes {
                baseline: vec![TestCase::new("t1", TestStatus::Passed)],
                candidate: vec![TestCase::new("t1", TestStatus::failed("boom"))],
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let d = capture(dir.path(), &n).unwrap();
        assert!(!d.accepted, "a per-test regression vetoes: {d:?}");
        let v = verify(dir.path()).unwrap();
        assert!(v.matches, "{:?}", v.diffs);
    }
}
