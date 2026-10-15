//! The deterministic required-check gate.
//!
//! The design review named the defect precisely: evaluation ran *before*
//! patch emission, the emitted candidate was never re-evaluated, and evaluator
//! failure did not veto — so failure text and an ACCEPT label could coexist in
//! one report. This module is the veto that closes it.
//!
//! The gate is a **pure function** of three inputs — the frozen manifest, the
//! typed receipts, and the strict verdict parse — and it is consulted *after*
//! the candidate has been applied and the required evaluators re-run. The model
//! cannot argue with it: a required evaluator that is missing, silent, blocked,
//! timed out, explicitly failing or non-zero vetoes acceptance regardless of
//! what the report says.
//!
//! Vetoes carry a class, and the class picks the substituted verdict:
//!
//! | class | cause | verdict |
//! |---|---|---|
//! | harness | missing / silent / blocked / timed out / patch would not apply | `BLOCKED-ENV` |
//! | evidence | non-zero exit / explicit FAIL | `REJECT` |
//! | evidence | patch touches a protected path / new per-test failure | `REJECT` |
//! | harness | candidate config digest drifted / per-test fault | `BLOCKED-ENV` |
//! | unproven | no candidate patch / unreadable verdict line | `INCONCLUSIVE` |
//!
//! `BLOCKED-ENV` is deliberate for harness faults: a broken runner is not
//! evidence against the repository and must not park a healthy repo on the dry
//! streak.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::guard::{self, GuardViolation, ProtectedPolicy};
use crate::manifest::{EvaluatorIdentity, ExperimentManifest};
use crate::pertest::{self, PairwiseOptions, TestOutcomes};
use crate::receipts::{EvaluatorOutcome, EvaluatorReceipt, Phase};
use crate::verdict::{Verdict, VerdictParseError};

/// What kind of thing went wrong, which decides the substituted verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VetoClass {
    /// The evidence could not be gathered — an operational fault.
    Harness,
    /// The evidence was gathered and it is against the candidate.
    Evidence,
    /// There was nothing to test, or nothing readable to act on.
    Unproven,
}

/// One reason acceptance was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Veto {
    /// What kind of thing went wrong.
    pub class: VetoClass,
    /// Evaluator name, or a pseudo-name for structural vetoes.
    pub subject: String,
    /// Human-readable cause.
    pub reason: String,
}

impl Veto {
    pub(crate) fn harness(subject: &str, reason: impl Into<String>) -> Self {
        Self {
            class: VetoClass::Harness,
            subject: subject.into(),
            reason: reason.into(),
        }
    }
    pub(crate) fn evidence(subject: &str, reason: impl Into<String>) -> Self {
        Self {
            class: VetoClass::Evidence,
            subject: subject.into(),
            reason: reason.into(),
        }
    }
    pub(crate) fn unproven(subject: &str, reason: impl Into<String>) -> Self {
        Self {
            class: VetoClass::Unproven,
            subject: subject.into(),
            reason: reason.into(),
        }
    }
}

/// The gate's ruling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateDecision {
    /// True only when the candidate earned ACCEPT: a clean strict parse of
    /// `ACCEPT`, an applied candidate, and every required evaluator passing on
    /// the candidate tree.
    pub accepted: bool,
    /// The verdict that goes in the ledger.
    pub verdict: String,
    /// The model's own strict reading, or `UNPARSED (…)`.
    pub model_verdict: String,
    /// Every reason acceptance was refused; empty when accepted.
    pub vetoes: Vec<Veto>,
    /// Every required evaluator and how it landed on the candidate tree.
    pub required_outcomes: Vec<(String, String)>,
    /// One-line account of the ruling.
    pub summary: String,
}

impl GateDecision {
    /// The ledger verdict as a [`Verdict`] (unknown text reads as inconclusive).
    pub fn verdict_enum(&self) -> Verdict {
        match self.verdict.as_str() {
            "ACCEPT" => Verdict::Accept,
            "REJECT" => Verdict::Reject,
            "BLOCKED-ENV" => Verdict::BlockedEnv,
            "HANDOFF" => Verdict::Handoff,
            _ => Verdict::Inconclusive,
        }
    }
}

/// Turn one required evaluator's outcome into a veto, or `None` if it passed.
fn veto_for(name: &str, outcome: &EvaluatorOutcome) -> Option<Veto> {
    match outcome {
        EvaluatorOutcome::Passed => None,
        EvaluatorOutcome::Missing => Some(Veto::harness(
            name,
            "required evaluator produced no receipt — it never ran",
        )),
        EvaluatorOutcome::Silent => Some(Veto::harness(
            name,
            "required evaluator exited 0 with no output on either stream — \
             surface-independent, so it proves nothing",
        )),
        EvaluatorOutcome::Blocked { detail } => Some(Veto::harness(
            name,
            format!("required evaluator was blocked: {detail}"),
        )),
        EvaluatorOutcome::TimedOut { after_secs } => Some(Veto::harness(
            name,
            format!("required evaluator exceeded its {after_secs}s budget and was killed"),
        )),
        EvaluatorOutcome::Failed { exit_code } => Some(Veto::evidence(
            name,
            format!("required evaluator exited {exit_code}"),
        )),
        EvaluatorOutcome::ExplicitFail { marker } => Some(Veto::evidence(
            name,
            format!("required evaluator declared failure: {marker}"),
        )),
    }
}

/// Complete the receipt set: every required evaluator in the manifest that has
/// no receipt for `phase` gets a [`EvaluatorOutcome::Missing`] one.
///
/// This is what makes "missing" a veto rather than a silent pass — an evaluator
/// the dispatcher skipped is otherwise simply absent from the evidence.
pub fn complete_receipts(
    required: &[&EvaluatorIdentity],
    receipts: &[EvaluatorReceipt],
    phase: Phase,
) -> Vec<EvaluatorReceipt> {
    let mut out: Vec<EvaluatorReceipt> = receipts
        .iter()
        .filter(|r| r.phase == phase)
        .cloned()
        .collect();
    for id in required {
        if !out.iter().any(|r| r.name == id.name) {
            out.push(EvaluatorReceipt::missing(
                &id.name,
                &id.command,
                phase,
                true,
            ));
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The environment pre-check on the **baseline** receipts.
///
/// A baseline evaluator is allowed to fail — that failure is often the very
/// finding the night is about. What is not allowed is a baseline evaluator that
/// could not run: if the harness is broken there is nothing to reason over, and
/// the night is `BLOCKED-ENV` before a token is spent on the model.
pub fn environment_vetoes(
    manifest: &ExperimentManifest,
    baseline: &[EvaluatorReceipt],
) -> Vec<Veto> {
    let required = manifest.required();
    complete_receipts(&required, baseline, Phase::Baseline)
        .iter()
        .filter(|r| r.required && r.outcome.is_harness_fault())
        .filter_map(|r| veto_for(&r.name, &r.outcome))
        .collect()
}

/// Structural facts about the candidate, established before the gate runs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CandidateState {
    /// A patch was extracted, applied cleanly and re-evaluated. The patch text
    /// is carried so [`decide`] can re-derive its touched paths itself.
    Applied {
        /// Tree hash of the patched candidate.
        tree_hash: String,
        /// The unified diff that was applied.
        patch: String,
    },
    /// The report claimed a result but emitted no `dream-patch` block.
    NoPatch,
    /// A patch was emitted but would not apply to the baseline tree.
    DidNotApply {
        /// Why it did not apply.
        detail: String,
    },
    /// A patch was emitted but the engine refused it before applying (for
    /// example it deletes a binary file). A model fault, not a harness one.
    Refused {
        /// Why it was refused.
        detail: String,
    },
    /// The guard refused the patch because it touches protected paths
    /// ([`guard::check`] returned [`GuardViolation::Protected`]). Maps to an
    /// evidence veto (`REJECT`) with subject `protected-path`.
    Protected {
        /// The protected paths touched.
        paths: Vec<String>,
    },
    /// The model did not claim ACCEPT, so no candidate was built. The gate
    /// still records the required outcomes, but there is nothing to veto.
    NotAttempted,
}

/// Everything [`decide`] needs beyond the manifest, verdict, candidate state
/// and receipts.
///
/// The policy is mandatory, so the gate cannot be run with the protected-path
/// re-check silently absent.
#[derive(Debug, Clone)]
pub struct GateContext<'a> {
    /// The protected-path policy, built from the **baseline** config (see
    /// [`ProtectedPolicy::for_manifest`]).
    pub policy: &'a ProtectedPolicy,
    /// Digest of the configuration bytes the candidate tree was evaluated
    /// with. When `Some` and different from the manifest's frozen
    /// `config_digest` while the patch did *not* touch the config, the
    /// evaluation environment drifted: a harness veto (`BLOCKED-ENV`). `None`
    /// skips the check.
    pub candidate_config_digest: Option<String>,
    /// Optional per-test evidence, keyed by evaluator name (see [`pertest`]).
    pub tests: BTreeMap<String, TestOutcomes>,
    /// Which pairwise policies apply to that evidence.
    pub options: PairwiseOptions,
}

impl<'a> GateContext<'a> {
    /// A context with no config-digest check and no per-test evidence.
    pub fn new(policy: &'a ProtectedPolicy) -> Self {
        Self {
            policy,
            candidate_config_digest: None,
            tests: BTreeMap::new(),
            options: PairwiseOptions::default(),
        }
    }

    /// Set the candidate's config digest.
    #[must_use]
    pub fn with_config_digest(mut self, digest: impl Into<String>) -> Self {
        self.candidate_config_digest = Some(digest.into());
        self
    }

    /// Attach per-test evidence for one evaluator.
    #[must_use]
    pub fn with_tests(mut self, evaluator: impl Into<String>, t: TestOutcomes) -> Self {
        self.tests.insert(evaluator.into(), t);
        self
    }

    /// Replace the pairwise options.
    #[must_use]
    pub fn with_options(mut self, options: PairwiseOptions) -> Self {
        self.options = options;
        self
    }
}

fn guard_veto(v: &GuardViolation) -> Veto {
    match v {
        GuardViolation::Protected { paths } => Veto::evidence(
            "protected-path",
            format!(
                "candidate patch touches protected path(s): {}",
                paths.join(", ")
            ),
        ),
        other => Veto::evidence(
            "patch-guard",
            format!("candidate patch failed the guard re-check: {other}"),
        ),
    }
}

/// The deterministic gate.
///
/// `strict` is [`crate::verdict::parse_verdict_strict`]'s result — the only
/// reading of the model's verdict that acceptance consults.
///
/// # Defence in depth
///
/// For an [`CandidateState::Applied`] candidate this function **re-derives the
/// touched paths from the patch text** with [`guard::check`] and vetoes (as
/// evidence, `REJECT`) if any is protected or the patch cannot be parsed with
/// certainty. That is a second line: callers must *also* run [`guard::check`]
/// **before applying** the patch. When the re-check fails the candidate's
/// receipts are not graded — they come from a tree that should never have been
/// built.
///
/// The protected-path re-check runs when the model claims ACCEPT (as every
/// candidate-state check does); when no acceptance is claimed there is nothing
/// to veto and the model's own verdict passes through.
///
/// Per-test evidence in `ctx` can only add vetoes; an evaluator without it is
/// judged exactly as by the receipts alone.
pub fn decide(
    manifest: &ExperimentManifest,
    strict: &Result<Verdict, VerdictParseError>,
    candidate: &CandidateState,
    candidate_receipts: &[EvaluatorReceipt],
    ctx: &GateContext<'_>,
) -> GateDecision {
    let required = manifest.required();
    let completed = complete_receipts(&required, candidate_receipts, Phase::Candidate);
    let required_outcomes: Vec<(String, String)> = completed
        .iter()
        .filter(|r| r.required)
        .map(|r| (r.name.clone(), r.outcome.label().to_string()))
        .collect();

    let model_verdict = match strict {
        Ok(v) => v.as_str().to_string(),
        Err(e) => format!("UNPARSED ({e})"),
    };

    let mut vetoes: Vec<Veto> = Vec::new();

    // 1. The model must have declared ACCEPT, readably. Anything else is not a
    //    veto so much as an absence of a claim to accept.
    let claimed_accept = matches!(strict, Ok(Verdict::Accept));
    if let Err(e) = strict {
        vetoes.push(Veto::unproven(
            "verdict",
            format!("strict verdict parse failed: {e}"),
        ));
    }

    // 2. A claim of ACCEPT must come with a candidate that was applied and
    //    re-evaluated. "Accepted" with nothing to test is unearned. An applied
    //    candidate's patch is re-checked against the protected policy.
    let mut tree_trusted = matches!(candidate, CandidateState::Applied { .. });
    if claimed_accept {
        match candidate {
            CandidateState::Applied { patch, .. } => {
                if let Err(v) = guard::check(patch, ctx.policy) {
                    tree_trusted = false;
                    vetoes.push(guard_veto(&v));
                }
            }
            CandidateState::Protected { paths } => vetoes.push(Veto::evidence(
                "protected-path",
                format!(
                    "candidate patch touches protected path(s): {}",
                    paths.join(", ")
                ),
            )),
            CandidateState::NoPatch => vetoes.push(Veto::unproven(
                "candidate",
                "report declared ACCEPT but emitted no ```dream-patch block, \
                 so no candidate tree could be built or re-evaluated",
            )),
            CandidateState::Refused { detail } => vetoes.push(Veto::unproven(
                "candidate",
                format!("candidate patch refused by the engine: {detail}"),
            )),
            CandidateState::DidNotApply { detail } => vetoes.push(Veto::harness(
                "candidate",
                format!("candidate patch did not apply to the baseline tree: {detail}"),
            )),
            CandidateState::NotAttempted => vetoes.push(Veto::unproven(
                "candidate",
                "report declared ACCEPT but the candidate rerun was never attempted",
            )),
        }
    }

    // 3. Required evaluators on the candidate tree. This is the veto proper,
    //    and it applies whatever the report's prose says — but only when a
    //    candidate tree exists to have been evaluated. An ACCEPT that shipped
    //    no patch has nothing to run: grading its absent receipts as "never
    //    ran" turned a model failing into false harness vetoes and a
    //    BLOCKED-ENV verdict about a harness that had in fact passed every
    //    baseline evaluator. That case is `unproven` → INCONCLUSIVE, which is
    //    what step 2 records. Likewise a tree built from a patch the guard
    //    refuses proves nothing, so its receipts are not graded.
    if tree_trusted {
        for r in completed.iter().filter(|r| r.required) {
            if let Some(v) = veto_for(&r.name, &r.outcome) {
                vetoes.push(v);
            }
        }
        // 3b. The environment must be the one that was frozen. The patch did
        //     not touch the config (the guard just said so), so any digest
        //     drift is the harness, not the candidate.
        if claimed_accept {
            if let Some(d) = &ctx.candidate_config_digest {
                if d != &manifest.config_digest {
                    vetoes.push(Veto::harness(
                        "config-digest",
                        format!(
                            "candidate was evaluated with config digest {d}, but the manifest \
                             froze {} and the patch did not touch the config",
                            manifest.config_digest
                        ),
                    ));
                }
            }
        }
        // 3c. Optional per-test evidence: additive, never relaxes anything.
        let names: Vec<&str> = required.iter().map(|e| e.name.as_str()).collect();
        vetoes.extend(pertest::vetoes(&names, &ctx.tests, &ctx.options));
    }

    let accepted = claimed_accept && vetoes.is_empty();
    let verdict = if accepted {
        Verdict::Accept
    } else if !claimed_accept {
        // No acceptance was claimed: keep the model's own reading, and fall
        // back to INCONCLUSIVE when it could not be read at all.
        match strict {
            Ok(v) => *v,
            Err(_) => Verdict::Inconclusive,
        }
    } else if vetoes.iter().any(|v| v.class == VetoClass::Harness) {
        // Harness faults outrank evidence — but a refused patch skips step 3
        // entirely, so a guard veto can never be diluted into BLOCKED-ENV.
        Verdict::BlockedEnv
    } else if vetoes.iter().any(|v| v.class == VetoClass::Evidence) {
        Verdict::Reject
    } else {
        Verdict::Inconclusive
    };

    let summary = if accepted {
        format!(
            "ACCEPT upheld: {} required evaluator(s) passed on candidate tree {}",
            required_outcomes.len(),
            match candidate {
                CandidateState::Applied { tree_hash, .. } => &tree_hash[..tree_hash.len().min(12)],
                _ => "-",
            }
        )
    } else if claimed_accept {
        format!(
            "ACCEPT vetoed → {}: {}",
            verdict.as_str(),
            vetoes
                .iter()
                .map(|v| format!("{}: {}", v.subject, v.reason))
                .collect::<Vec<_>>()
                .join("; ")
        )
    } else {
        format!("no acceptance claimed (model verdict {model_verdict})")
    };

    GateDecision {
        accepted,
        verdict: verdict.as_str().to_string(),
        model_verdict,
        vetoes,
        required_outcomes,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{digest, ModelIdentity};
    use crate::outcome::ExecOutcome;
    use crate::pertest::{FaultKind, TestCase, TestStatus};

    fn ident(name: &str, required: bool) -> EvaluatorIdentity {
        let cmd = format!("run-{name}");
        EvaluatorIdentity {
            name: name.into(),
            command_digest: digest(cmd.as_bytes()),
            command: cmd,
            required,
            deeps: vec![],
            timeout_secs: 900,
            test_format: None,
        }
    }

    fn manifest(evaluators: Vec<EvaluatorIdentity>) -> ExperimentManifest {
        ExperimentManifest {
            schema: 1,
            run_id: "0123456789abcdef".into(),
            night_id: "2026-09-05-example".into(),
            repo: "example".into(),
            repo_slug: "owner/example".into(),
            date: "2026-09-05".into(),
            day_int: 20260905,
            deep: "core".into(),
            scan: vec![],
            baseline_revision: "a".repeat(40),
            baseline_tree_hash: "b".repeat(40),
            config_digest: digest(b"cfg"),
            evaluators,
            model: ModelIdentity {
                provider: "provider-x".into(),
                model: "model-y".into(),
                max_tokens: 16384,
                fallback: None,
            },
            engine_version: "0.1.0".into(),
            created_at: "2026-09-05T01:00:00Z".into(),
        }
    }

    fn receipt(name: &str, phase: Phase, code: i32, stdout: &str) -> EvaluatorReceipt {
        EvaluatorReceipt::from_exec(
            name,
            &format!("run-{name}"),
            phase,
            true,
            900,
            ExecOutcome {
                exit_code: Some(code),
                stdout: stdout.into(),
                stderr: String::new(),
                duration_ms: 10,
                transport_error: None,
                timed_out: false,
                started_at: "2026-09-05T01:00:00Z".into(),
            },
        )
    }

    const BENIGN_PATCH: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n";

    fn applied_with(patch: &str) -> CandidateState {
        CandidateState::Applied {
            tree_hash: "c".repeat(40),
            patch: patch.into(),
        }
    }

    fn applied() -> CandidateState {
        applied_with(BENIGN_PATCH)
    }

    /// The default context: built-in protection only, no digest, no per-test data.
    fn decide(
        m: &ExperimentManifest,
        strict: &Result<Verdict, VerdictParseError>,
        cand: &CandidateState,
        receipts: &[EvaluatorReceipt],
    ) -> GateDecision {
        let policy = ProtectedPolicy::default();
        super::decide(m, strict, cand, receipts, &GateContext::new(&policy))
    }

    #[test]
    fn clean_accept_survives_the_gate() {
        let m = manifest(vec![ident("tests", true), ident("lint", false)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt("tests", Phase::Candidate, 0, "42 passed")],
        );
        assert!(d.accepted, "{}", d.summary);
        assert_eq!(d.verdict, "ACCEPT");
        assert!(d.vetoes.is_empty());
        assert_eq!(
            d.required_outcomes,
            vec![("tests".to_string(), "PASSED".to_string())]
        );
    }

    /// The headline case from the ADR closeout: a deliberately broken candidate
    /// cannot receive ACCEPT, however confidently the report asserts it.
    #[test]
    fn a_broken_candidate_cannot_receive_accept() {
        let m = manifest(vec![ident("tests", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt(
                "tests",
                Phase::Candidate,
                101,
                "error[E0308]: mismatched types",
            )],
        );
        assert!(!d.accepted);
        assert_eq!(
            d.verdict, "REJECT",
            "a failing required evaluator is evidence, not a harness fault"
        );
        assert_eq!(
            d.model_verdict, "ACCEPT",
            "the model's own claim is still recorded"
        );
        assert_eq!(d.vetoes.len(), 1);
        assert_eq!(d.vetoes[0].class, VetoClass::Evidence);
    }

    #[test]
    fn failure_text_with_a_zero_exit_still_vetoes() {
        // "Failure text can coexist with ACCEPT" — the exact reproduced defect.
        let m = manifest(vec![ident("recall", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt(
                "recall",
                Phase::Candidate,
                0,
                "band check\nFAIL: true 91/120\n",
            )],
        );
        assert!(!d.accepted);
        assert_eq!(d.verdict, "REJECT");
        assert!(
            d.vetoes[0].reason.contains("declared failure"),
            "{:?}",
            d.vetoes
        );
    }

    #[test]
    fn a_missing_required_evaluator_vetoes_as_a_harness_fault() {
        let m = manifest(vec![ident("tests", true), ident("hooks", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt("tests", Phase::Candidate, 0, "ok")],
        );
        assert!(!d.accepted);
        assert_eq!(d.verdict, "BLOCKED-ENV");
        assert_eq!(d.vetoes.len(), 1);
        assert_eq!(d.vetoes[0].subject, "hooks");
        assert_eq!(d.vetoes[0].class, VetoClass::Harness);
    }

    #[test]
    fn a_silent_required_evaluator_vetoes() {
        let m = manifest(vec![ident("darwin", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt("darwin", Phase::Candidate, 0, "")],
        );
        assert!(!d.accepted);
        assert_eq!(d.verdict, "BLOCKED-ENV");
        assert!(d.vetoes[0].reason.contains("no output"), "{:?}", d.vetoes);
    }

    #[test]
    fn a_timed_out_required_evaluator_vetoes() {
        let m = manifest(vec![ident("tests", true)]);
        let mut r = receipt("tests", Phase::Candidate, 0, "partial");
        r.outcome = EvaluatorOutcome::TimedOut { after_secs: 900 };
        let d = decide(&m, &Ok(Verdict::Accept), &applied(), &[r]);
        assert!(!d.accepted);
        assert_eq!(d.verdict, "BLOCKED-ENV");
    }

    #[test]
    fn a_blocked_required_evaluator_vetoes() {
        let m = manifest(vec![ident("tests", true)]);
        let mut r = receipt("tests", Phase::Candidate, 0, "x");
        r.outcome = EvaluatorOutcome::Blocked {
            detail: "runner: no route to host".into(),
        };
        let d = decide(&m, &Ok(Verdict::Accept), &applied(), &[r]);
        assert!(!d.accepted);
        assert_eq!(d.verdict, "BLOCKED-ENV");
    }

    #[test]
    fn an_advisory_evaluator_never_vetoes() {
        let m = manifest(vec![ident("tests", true), ident("lint", false)]);
        let mut lint = receipt("lint", Phase::Candidate, 1, "style nits");
        lint.required = false;
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt("tests", Phase::Candidate, 0, "ok"), lint],
        );
        assert!(d.accepted, "{}", d.summary);
    }

    /// 2026-09-07: bench/lint/pin-parity all PASSED on
    /// baseline, the model claimed ACCEPT without a patch, and the gate
    /// reported "required evaluator produced no receipt — it never ran" three
    /// times → BLOCKED-ENV. No patch means nothing to re-run; that is the
    /// model's failing (unproven), not the harness's.
    #[test]
    fn accept_without_a_candidate_patch_is_unproven_not_a_harness_fault() {
        let m = manifest(vec![ident("tests", true)]);
        let d = decide(&m, &Ok(Verdict::Accept), &CandidateState::NoPatch, &[]);
        assert!(!d.accepted);
        assert_eq!(d.verdict, "INCONCLUSIVE", "{}", d.summary);
        assert!(
            d.vetoes.iter().any(|v| v.subject == "candidate"),
            "{:?}",
            d.vetoes
        );
        assert!(
            d.vetoes.iter().all(|v| v.class != VetoClass::Harness),
            "absent candidate receipts must not read as a broken harness: {:?}",
            d.vetoes
        );
    }

    #[test]
    fn a_patch_that_will_not_apply_is_a_harness_fault() {
        let m = manifest(vec![ident("tests", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &CandidateState::DidNotApply {
                detail: "hunk #2 failed".into(),
            },
            &[receipt("tests", Phase::Candidate, 0, "ok")],
        );
        assert!(!d.accepted);
        assert_eq!(d.verdict, "BLOCKED-ENV");
    }

    #[test]
    fn an_unreadable_verdict_never_reaches_accept() {
        let m = manifest(vec![ident("tests", true)]);
        for err in [
            VerdictParseError::Missing,
            VerdictParseError::Ambiguous(vec!["ACCEPT".into(), "REJECT".into()]),
            VerdictParseError::Noisy("ACCEPT because".into()),
            VerdictParseError::Unknown("MAYBE".into()),
        ] {
            let d = decide(
                &m,
                &Err(err.clone()),
                &applied(),
                &[receipt("tests", Phase::Candidate, 0, "ok")],
            );
            assert!(!d.accepted, "{err:?} must not accept");
            assert_eq!(d.verdict, "INCONCLUSIVE", "{err:?}");
        }
    }

    #[test]
    fn a_reject_verdict_passes_through_untouched() {
        let m = manifest(vec![ident("tests", true)]);
        let d = decide(&m, &Ok(Verdict::Reject), &CandidateState::NotAttempted, &[]);
        assert!(!d.accepted);
        assert_eq!(d.verdict, "REJECT");
        assert!(
            d.vetoes.is_empty(),
            "nothing to veto when nothing was claimed"
        );
    }

    #[test]
    fn environment_vetoes_ignore_a_legitimately_failing_baseline() {
        let m = manifest(vec![ident("tests", true)]);
        // A baseline failure is the finding, not a fault.
        let v = environment_vetoes(
            &m,
            &[receipt("tests", Phase::Baseline, 1, "3 tests failed")],
        );
        assert!(v.is_empty(), "{v:?}");
        // A baseline evaluator that could not run is a fault.
        let mut blocked = receipt("tests", Phase::Baseline, 0, "");
        blocked.outcome = EvaluatorOutcome::Blocked {
            detail: "no such file".into(),
        };
        assert_eq!(environment_vetoes(&m, &[blocked]).len(), 1);
        // As is one that never ran at all.
        assert_eq!(environment_vetoes(&m, &[]).len(), 1);
    }

    #[test]
    fn evaluators_scoped_to_another_deep_are_not_required_tonight() {
        let mut other = ident("hooks", true);
        other.deeps = vec!["hooks-pipeline".into()];
        let m = manifest(vec![ident("tests", true), other]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt("tests", Phase::Candidate, 0, "ok")],
        );
        assert!(d.accepted, "{}", d.summary);
    }

    // ------------------------------------------------------------------
    // Regression oracle: the old decision procedure, byte for byte.
    // ------------------------------------------------------------------

    /// VERBATIM copy of the pre-guard `decide` (original dream-engine gate.rs),
    /// kept as the behavioural oracle for the additive-change regression test.
    fn legacy_decide(
        manifest: &ExperimentManifest,
        strict: &Result<Verdict, VerdictParseError>,
        candidate: &CandidateState,
        candidate_receipts: &[EvaluatorReceipt],
    ) -> GateDecision {
        let required = manifest.required();
        let completed = complete_receipts(&required, candidate_receipts, Phase::Candidate);
        let required_outcomes: Vec<(String, String)> = completed
            .iter()
            .filter(|r| r.required)
            .map(|r| (r.name.clone(), r.outcome.label().to_string()))
            .collect();

        let model_verdict = match strict {
            Ok(v) => v.as_str().to_string(),
            Err(e) => format!("UNPARSED ({e})"),
        };

        let mut vetoes: Vec<Veto> = Vec::new();

        // 1. The model must have declared ACCEPT, readably. Anything else is not a
        //    veto so much as an absence of a claim to accept.
        let claimed_accept = matches!(strict, Ok(Verdict::Accept));
        if let Err(e) = strict {
            vetoes.push(Veto::unproven(
                "verdict",
                format!("strict verdict parse failed: {e}"),
            ));
        }

        // 2. A claim of ACCEPT must come with a candidate that was applied and
        //    re-evaluated. "Accepted" with nothing to test is unearned.
        if claimed_accept {
            match candidate {
                CandidateState::Applied { .. } => {}
                CandidateState::NoPatch => vetoes.push(Veto::unproven(
                    "candidate",
                    "report declared ACCEPT but emitted no ```dream-patch block, \
                     so no candidate tree could be built or re-evaluated",
                )),
                CandidateState::Refused { detail } => vetoes.push(Veto::unproven(
                    "candidate",
                    format!("candidate patch refused by the engine: {detail}"),
                )),
                CandidateState::DidNotApply { detail } => vetoes.push(Veto::harness(
                    "candidate",
                    format!("candidate patch did not apply to the baseline tree: {detail}"),
                )),
                CandidateState::Protected { .. } => unreachable!("not a pre-guard state"),
                CandidateState::NotAttempted => vetoes.push(Veto::unproven(
                    "candidate",
                    "report declared ACCEPT but the candidate rerun was never attempted",
                )),
            }
        }

        // 3. Required evaluators on the candidate tree. This is the veto proper,
        //    and it applies whatever the report's prose says — but only when a
        //    candidate tree exists to have been evaluated. An ACCEPT that shipped
        //    no patch (2026-09-07) has nothing to run: grading
        //    its absent receipts as "never ran" turned a model failing into three
        //    false harness vetoes, a BLOCKED-ENV verdict and an operator alert
        //    about a harness that had in fact passed every baseline evaluator.
        //    That case is `unproven` → INCONCLUSIVE, which is what step 2 records.
        if matches!(candidate, CandidateState::Applied { .. }) {
            for r in completed.iter().filter(|r| r.required) {
                if let Some(v) = veto_for(&r.name, &r.outcome) {
                    vetoes.push(v);
                }
            }
        }

        let accepted = claimed_accept && vetoes.is_empty();
        let verdict = if accepted {
            Verdict::Accept
        } else if !claimed_accept {
            // No acceptance was claimed: keep the model's own reading, and fall
            // back to INCONCLUSIVE when it could not be read at all.
            match strict {
                Ok(v) => *v,
                Err(_) => Verdict::Inconclusive,
            }
        } else if vetoes.iter().any(|v| v.class == VetoClass::Harness) {
            Verdict::BlockedEnv
        } else if vetoes.iter().any(|v| v.class == VetoClass::Evidence) {
            Verdict::Reject
        } else {
            Verdict::Inconclusive
        };

        let summary = if accepted {
            format!(
                "ACCEPT upheld: {} required evaluator(s) passed on candidate tree {}",
                required_outcomes.len(),
                match candidate {
                    CandidateState::Applied { tree_hash, .. } =>
                        &tree_hash[..tree_hash.len().min(12)],
                    _ => "-",
                }
            )
        } else if claimed_accept {
            format!(
                "ACCEPT vetoed → {}: {}",
                verdict.as_str(),
                vetoes
                    .iter()
                    .map(|v| format!("{}: {}", v.subject, v.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        } else {
            format!("no acceptance claimed (model verdict {model_verdict})")
        };

        GateDecision {
            accepted,
            verdict: verdict.as_str().to_string(),
            model_verdict,
            vetoes,
            required_outcomes,
            summary,
        }
    }

    fn outcome_variants() -> Vec<Option<EvaluatorOutcome>> {
        vec![
            None,
            Some(EvaluatorOutcome::Passed),
            Some(EvaluatorOutcome::Failed { exit_code: 1 }),
            Some(EvaluatorOutcome::ExplicitFail {
                marker: "FAIL".into(),
            }),
            Some(EvaluatorOutcome::Blocked { detail: "d".into() }),
            Some(EvaluatorOutcome::TimedOut { after_secs: 9 }),
            Some(EvaluatorOutcome::Silent),
            Some(EvaluatorOutcome::Missing),
        ]
    }

    fn receipt_with(name: &str, o: &EvaluatorOutcome) -> EvaluatorReceipt {
        let mut r = receipt(name, Phase::Candidate, 0, "ok");
        r.outcome = o.clone();
        r
    }

    /// Every pre-existing gate scenario — verdict reading × candidate state ×
    /// outcome of two required evaluators — must produce a byte-identical
    /// decision from the new `decide` (benign patch, no digest, no per-test data).
    #[test]
    fn old_verdicts_are_unchanged_across_the_whole_scenario_space() {
        let m = manifest(vec![
            ident("tests", true),
            ident("hooks", true),
            ident("lint", false),
        ]);
        let stricts: Vec<Result<Verdict, VerdictParseError>> = vec![
            Ok(Verdict::Accept),
            Ok(Verdict::Reject),
            Ok(Verdict::Inconclusive),
            Ok(Verdict::BlockedEnv),
            Ok(Verdict::Handoff),
            Err(VerdictParseError::Missing),
            Err(VerdictParseError::Noisy("ACCEPT because".into())),
        ];
        let cands = [
            applied(),
            CandidateState::NoPatch,
            CandidateState::DidNotApply {
                detail: "hunk".into(),
            },
            CandidateState::Refused {
                detail: "bin".into(),
            },
            CandidateState::NotAttempted,
        ];
        let mut n = 0;
        for strict in &stricts {
            for cand in &cands {
                for a in outcome_variants() {
                    for b in outcome_variants() {
                        let mut rs = Vec::new();
                        if let Some(o) = &a {
                            rs.push(receipt_with("tests", o));
                        }
                        if let Some(o) = &b {
                            rs.push(receipt_with("hooks", o));
                        }
                        let new = decide(&m, strict, cand, &rs);
                        let old = legacy_decide(&m, strict, cand, &rs);
                        assert_eq!(
                            serde_json::to_string(&new).unwrap(),
                            serde_json::to_string(&old).unwrap(),
                            "{strict:?} / {cand:?} / {a:?} / {b:?}"
                        );
                        n += 1;
                    }
                }
            }
        }
        assert_eq!(n, 7 * 5 * 8 * 8);
    }

    // ------------------------------------------------------------------
    // Guard integration
    // ------------------------------------------------------------------

    fn patch_for(path: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-a\n+b\n")
    }

    #[test]
    fn a_protected_candidate_state_is_an_evidence_veto_with_subject_protected_path() {
        let m = manifest(vec![ident("tests", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &CandidateState::Protected {
                paths: vec!["gate.rs".into()],
            },
            &[],
        );
        assert!(!d.accepted);
        assert_eq!(d.verdict, "REJECT", "{}", d.summary);
        assert_eq!(d.vetoes.len(), 1);
        assert_eq!(d.vetoes[0].class, VetoClass::Evidence);
        assert_eq!(d.vetoes[0].subject, "protected-path");
    }

    #[test]
    fn decide_rederives_touched_paths_even_if_the_caller_said_applied() {
        let m = manifest(vec![ident("tests", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied_with(&patch_for("crates/x/gate.rs")),
            &[receipt("tests", Phase::Candidate, 0, "ok")],
        );
        assert!(!d.accepted);
        assert_eq!(d.verdict, "REJECT");
        assert_eq!(d.vetoes[0].subject, "protected-path");
        assert!(d.vetoes[0].reason.contains("crates/x/gate.rs"));
    }

    /// A tampered tree's receipts prove nothing: a missing receipt must not
    /// turn the protected-path REJECT into BLOCKED-ENV.
    #[test]
    fn a_protected_patch_stays_reject_even_with_a_missing_receipt() {
        let m = manifest(vec![ident("tests", true), ident("hooks", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Accept),
            &applied_with(&patch_for("Cargo.toml")),
            &[receipt("tests", Phase::Candidate, 0, "ok")],
        );
        assert_eq!(d.verdict, "REJECT", "{}", d.summary);
        assert!(
            d.vetoes.iter().all(|v| v.class == VetoClass::Evidence),
            "{:?}",
            d.vetoes
        );
    }

    #[test]
    fn an_unparseable_applied_patch_fails_closed() {
        let m = manifest(vec![ident("tests", true)]);
        for bad in ["", "not a diff", "diff --git a/x b/x\n@@ -1 +1 @@\n"] {
            let d = decide(
                &m,
                &Ok(Verdict::Accept),
                &applied_with(bad),
                &[receipt("tests", Phase::Candidate, 0, "ok")],
            );
            assert!(!d.accepted, "{bad:?}");
            assert_eq!(d.verdict, "REJECT");
            assert_eq!(d.vetoes[0].subject, "patch-guard");
        }
    }

    #[test]
    fn a_protected_patch_with_no_acceptance_claimed_passes_the_model_verdict_through() {
        let m = manifest(vec![ident("tests", true)]);
        let d = decide(
            &m,
            &Ok(Verdict::Reject),
            &applied_with(&patch_for("Cargo.toml")),
            &[receipt("tests", Phase::Candidate, 0, "ok")],
        );
        assert_eq!(d.verdict, "REJECT");
        assert!(d.vetoes.is_empty(), "{:?}", d.vetoes);
    }

    #[test]
    fn config_digest_drift_without_a_config_edit_is_a_harness_veto() {
        let m = manifest(vec![ident("tests", true)]);
        let policy = ProtectedPolicy::default();
        let rs = [receipt("tests", Phase::Candidate, 0, "ok")];
        let same = GateContext::new(&policy).with_config_digest(m.config_digest.clone());
        assert!(super::decide(&m, &Ok(Verdict::Accept), &applied(), &rs, &same).accepted);
        let drift = GateContext::new(&policy).with_config_digest(digest(b"other"));
        let d = super::decide(&m, &Ok(Verdict::Accept), &applied(), &rs, &drift);
        assert!(!d.accepted);
        assert_eq!(d.verdict, "BLOCKED-ENV");
        assert_eq!(d.vetoes[0].subject, "config-digest");
        // Not claimed ACCEPT: nothing to veto.
        let d = super::decide(
            &m,
            &Ok(Verdict::Reject),
            &CandidateState::NotAttempted,
            &[],
            &drift,
        );
        assert!(d.vetoes.is_empty());
        // Editing the config is a protected-path REJECT, not a digest matter.
        let edit = super::decide(
            &m,
            &Ok(Verdict::Accept),
            &applied_with(&patch_for("dream.config.json")),
            &rs,
            &drift,
        );
        assert_eq!(edit.verdict, "REJECT");
        assert_eq!(edit.vetoes.len(), 1);
    }

    // ------------------------------------------------------------------
    // Per-test evidence
    // ------------------------------------------------------------------

    fn pass(n: &str) -> TestCase {
        TestCase::new(n, TestStatus::Passed)
    }
    fn fail(n: &str, c: &str) -> TestCase {
        TestCase::new(n, TestStatus::failed(c))
    }

    fn run_with(tests: TestOutcomes, options: PairwiseOptions, code: i32) -> GateDecision {
        let m = manifest(vec![ident("tests", true)]);
        let policy = ProtectedPolicy::default();
        let ctx = GateContext::new(&policy)
            .with_tests("tests", tests)
            .with_options(options);
        super::decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt("tests", Phase::Candidate, code, "out")],
            &ctx,
        )
    }

    #[test]
    fn a_clean_per_test_run_is_accepted() {
        let t = TestOutcomes {
            baseline: vec![pass("a"), pass("b")],
            candidate: vec![pass("a"), pass("b"), pass("c")],
        };
        let d = run_with(t, PairwiseOptions::default(), 0);
        assert!(d.accepted, "{}", d.summary);
    }

    #[test]
    fn a_new_failure_fingerprint_vetoes_as_evidence() {
        let t = TestOutcomes {
            baseline: vec![pass("a"), pass("b")],
            candidate: vec![pass("a"), fail("b", "assertion failed at line 12")],
        };
        let d = run_with(t, PairwiseOptions::default(), 0);
        assert_eq!(d.verdict, "REJECT", "{}", d.summary);
        assert!(
            d.vetoes.iter().any(|v| v.reason.contains("new failure")),
            "{:?}",
            d.vetoes
        );
    }

    #[test]
    fn a_failure_already_present_on_the_baseline_is_not_new() {
        let t = TestOutcomes {
            baseline: vec![pass("a"), fail("b", "boom at line 12")],
            candidate: vec![pass("a"), fail("b", "boom at line 14")],
        };
        let d = run_with(t, PairwiseOptions::default(), 0);
        assert!(
            d.accepted,
            "line numbers are normalised away: {}",
            d.summary
        );
    }

    #[test]
    fn a_new_failure_hidden_behind_a_red_baseline_is_still_caught() {
        let t = TestOutcomes {
            baseline: vec![pass("a"), fail("b", "old failure")],
            candidate: vec![fail("a", "fresh regression"), fail("b", "old failure")],
        };
        let d = run_with(t, PairwiseOptions::default(), 0);
        assert_eq!(d.verdict, "REJECT");
    }

    #[test]
    fn per_test_faults_are_a_harness_veto_never_a_loss() {
        for fk in [
            FaultKind::Missing,
            FaultKind::Silent,
            FaultKind::Blocked,
            FaultKind::TimedOut,
        ] {
            let t = TestOutcomes {
                baseline: vec![pass("a")],
                candidate: vec![TestCase::new("a", TestStatus::fault(fk))],
            };
            let d = run_with(t, PairwiseOptions::default(), 0);
            assert_eq!(d.verdict, "BLOCKED-ENV", "{fk:?}: {}", d.summary);
            assert!(
                d.vetoes.iter().all(|v| v.class == VetoClass::Harness),
                "{:?}",
                d.vetoes
            );
        }
    }

    #[test]
    fn a_baseline_test_dropped_from_the_candidate_run_is_a_fault() {
        let t = TestOutcomes {
            baseline: vec![pass("a"), pass("b")],
            candidate: vec![pass("a")],
        };
        let d = run_with(t, PairwiseOptions::default(), 0);
        assert_eq!(d.verdict, "BLOCKED-ENV");
    }

    #[test]
    fn per_test_evidence_never_relaxes_a_failing_receipt() {
        let t = TestOutcomes {
            baseline: vec![pass("a")],
            candidate: vec![pass("a")],
        };
        let d = run_with(t, PairwiseOptions::default(), 101);
        assert_eq!(d.verdict, "REJECT");
    }

    fn many(n: usize, status: impl Fn(usize) -> TestStatus) -> Vec<TestCase> {
        (0..n)
            .map(|i| TestCase::new(format!("t{i}"), status(i)))
            .collect()
    }

    #[test]
    fn statistical_policies_only_apply_with_enough_cases() {
        let opts = PairwiseOptions {
            min_win_margin: Some(1),
            ..PairwiseOptions::default()
        };
        // 25 ties: Margin(1) demands an improvement and rejects.
        let big = TestOutcomes {
            baseline: many(25, |_| TestStatus::Passed),
            candidate: many(25, |_| TestStatus::Passed),
        };
        let d = run_with(big, opts.clone(), 0);
        assert_eq!(d.verdict, "REJECT", "{}", d.summary);
        assert!(d.vetoes[0].reason.contains("margin"), "{:?}", d.vetoes);
        // 19 ties: below the threshold the evaluator is one case, Margin is skipped.
        let small = TestOutcomes {
            baseline: many(19, |_| TestStatus::Passed),
            candidate: many(19, |_| TestStatus::Passed),
        };
        assert!(run_with(small, opts, 0).accepted);
    }

    #[test]
    fn floor_applies_to_the_aggregate_case_when_small() {
        let opts = PairwiseOptions {
            floor: Some(1.0),
            ..PairwiseOptions::default()
        };
        let t = TestOutcomes {
            baseline: vec![fail("a", "old")],
            candidate: vec![fail("a", "old")],
        };
        let d = run_with(t, opts, 0);
        assert_eq!(d.verdict, "REJECT");
        assert!(d.vetoes[0].reason.contains("floor"), "{:?}", d.vetoes);
    }

    #[test]
    fn invalid_pairwise_options_cannot_accept() {
        let t = TestOutcomes {
            baseline: vec![pass("a")],
            candidate: vec![pass("a")],
        };
        let opts = PairwiseOptions {
            sign_alpha: Some(1.5),
            ..PairwiseOptions::default()
        };
        let d = run_with(t, opts, 0);
        assert!(!d.accepted);
        assert_eq!(d.verdict, "BLOCKED-ENV");
        assert_eq!(d.vetoes[0].subject, "pairwise-options");
    }

    #[test]
    fn advisory_evaluator_evidence_is_ignored() {
        let m = manifest(vec![ident("tests", true), ident("lint", false)]);
        let policy = ProtectedPolicy::default();
        let ctx = GateContext::new(&policy).with_tests(
            "lint",
            TestOutcomes {
                baseline: vec![pass("a")],
                candidate: vec![fail("a", "x")],
            },
        );
        let d = super::decide(
            &m,
            &Ok(Verdict::Accept),
            &applied(),
            &[receipt("tests", Phase::Candidate, 0, "ok")],
            &ctx,
        );
        assert!(d.accepted, "{}", d.summary);
    }
}
