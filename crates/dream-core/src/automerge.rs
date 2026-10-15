//! The automerge policy: a pure decision, default off.
//!
//! An ACCEPT verdict says the repository's evaluators did not object; it does
//! not say the change is wanted. The default path is therefore a draft pull
//! request a human merges. A repository may opt in with `autoMerge: true` in
//! its protected `dream.config.json`, and [`decide`] then still refuses unless
//! every condition below holds:
//!
//! 1. the target is not self-referential (this engine's own repository, a
//!    repository whose protected set covers the gate crates, or a checkout
//!    that contains them), whatever the flag says;
//! 2. the candidate verdict is ACCEPT and the pull request is draft-clean;
//! 3. the target branch has at least one required status check. `gh pr merge
//!    --auto` merges at once when nothing is required, so an empty list, a
//!    404 and any API error are all refusals, never permission.
//!
//! ```
//! use dream_core::automerge::{decide, AutomergeDecision, TargetFacts};
//! use dream_core::seams::ForgeError;
//! use dream_gate::{config::DreamConfig, verdict::Verdict};
//!
//! let cfg: DreamConfig = serde_json::from_str(
//!     r#"{"repo":"o/r","slots":[{"deep":"d","scan":["src"]}]}"#).unwrap();
//! let facts = TargetFacts {
//!     repo_slug: "o/r".into(),
//!     protects_gate_crates: false,
//!     contains_gate_crates: false,
//!     required_checks: Err(ForgeError::NotFound),
//!     verdict: Verdict::Accept,
//!     draft_clean: true,
//! };
//! assert_eq!(decide(&cfg, &facts), AutomergeDecision::Disabled);
//! ```

use dream_gate::config::DreamConfig;
use dream_gate::guard::ProtectedPolicy;
use dream_gate::verdict::Verdict;
use serde::Deserialize;

use crate::seams::ForgeError;

/// The slug of this engine's own repository (compared case-insensitively).
pub const ENGINE_REPO: &str = "DreamLab-AI/dream-engine";

/// Repo-relative directories of the gate crates.
pub const GATE_CRATE_DIRS: [&str; 3] = [
    "crates/dream-gate",
    "crates/pairwise-gate",
    "crates/failure-fingerprint",
];

/// Everything [`decide`] needs to know about the target, gathered by the
/// caller so the decision itself stays pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetFacts {
    /// `owner/name` of the target repository.
    pub repo_slug: String,
    /// The repository's protected set covers the gate crates (see
    /// [`protects_gate_crates`]).
    pub protects_gate_crates: bool,
    /// The checkout contains the gate crates (see [`contains_gate_crates`]).
    pub contains_gate_crates: bool,
    /// Required status checks of the target branch, or why they are unknown.
    pub required_checks: Result<Vec<String>, ForgeError>,
    /// The candidate's verdict.
    pub verdict: Verdict,
    /// The pull request opened cleanly as a draft.
    pub draft_clean: bool,
}

/// Why automerge was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The target is this engine or contains the gate crates.
    SelfReferential,
    /// The verdict is not ACCEPT.
    NotAccepted,
    /// The pull request is not a clean draft.
    PrNotClean,
    /// Branch protection could not be read (404, error or unsupported).
    ChecksUnknown(ForgeError),
    /// The branch requires no status checks, so `--auto` would merge at once.
    NoRequiredChecks,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::SelfReferential => f.write_str("target is self-referential"),
            Refusal::NotAccepted => f.write_str("verdict is not ACCEPT"),
            Refusal::PrNotClean => f.write_str("pull request is not a clean draft"),
            Refusal::ChecksUnknown(e) => write!(f, "required status checks unknown: {e}"),
            Refusal::NoRequiredChecks => f.write_str("target branch has no required status checks"),
        }
    }
}

/// The outcome of [`decide`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomergeDecision {
    /// The config does not ask for automerge (the default).
    Disabled,
    /// Automerge was asked for and refused.
    Refused(Refusal),
    /// Every condition holds; the host may enable automerge.
    Allowed,
}

/// Decide whether the pull request may be set to automerge. Pure.
pub fn decide(cfg: &DreamConfig, target: &TargetFacts) -> AutomergeDecision {
    if !cfg.auto_merge {
        return AutomergeDecision::Disabled;
    }
    if target.repo_slug.eq_ignore_ascii_case(ENGINE_REPO)
        || target.protects_gate_crates
        || target.contains_gate_crates
    {
        return AutomergeDecision::Refused(Refusal::SelfReferential);
    }
    if target.verdict != Verdict::Accept {
        return AutomergeDecision::Refused(Refusal::NotAccepted);
    }
    if !target.draft_clean {
        return AutomergeDecision::Refused(Refusal::PrNotClean);
    }
    match &target.required_checks {
        Err(e) => AutomergeDecision::Refused(Refusal::ChecksUnknown(e.clone())),
        Ok(list) if list.iter().all(|c| c.trim().is_empty()) => {
            AutomergeDecision::Refused(Refusal::NoRequiredChecks)
        }
        Ok(_) => AutomergeDecision::Allowed,
    }
}

/// Whether the config's protected set covers any gate crate.
pub fn protects_gate_crates(cfg: &DreamConfig) -> bool {
    let policy = ProtectedPolicy::from_config(cfg);
    GATE_CRATE_DIRS
        .iter()
        .any(|d| policy.is_protected(&format!("{d}/src/lib.rs")))
}

/// Whether the checkout at `repo_dir` contains a gate crate.
pub fn contains_gate_crates(repo_dir: &std::path::Path) -> bool {
    GATE_CRATE_DIRS
        .iter()
        .any(|d| repo_dir.join(d).join("Cargo.toml").is_file())
}

/// Extract the required status-check names from a
/// `GET /repos/{repo}/branches/{branch}/protection` body.
///
/// A body without `required_status_checks` yields an empty list (which
/// [`decide`] refuses); a body that is not JSON is an error.
pub fn parse_protection(body: &str) -> Result<Vec<String>, ForgeError> {
    #[derive(Deserialize)]
    struct Check {
        context: Option<String>,
    }
    #[derive(Deserialize)]
    struct Rsc {
        #[serde(default)]
        contexts: Vec<String>,
        #[serde(default)]
        checks: Vec<Check>,
    }
    #[derive(Deserialize)]
    struct Protection {
        required_status_checks: Option<Rsc>,
    }
    let p: Protection = serde_json::from_str(body)
        .map_err(|e| ForgeError::Failed(format!("unreadable protection response: {e}")))?;
    let mut out: Vec<String> = Vec::new();
    if let Some(r) = p.required_status_checks {
        for name in r
            .contexts
            .into_iter()
            .chain(r.checks.into_iter().filter_map(|c| c.context))
        {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seams::ForgeError;
    use dream_gate::config::DreamConfig;
    use dream_gate::verdict::Verdict;

    fn cfg(on: bool) -> DreamConfig {
        serde_json::from_value(serde_json::json!({
            "repo": "o/r",
            "slots": [{"deep":"d","scan":["src"]}],
            "automerge": on
        }))
        .unwrap()
    }

    fn facts() -> TargetFacts {
        TargetFacts {
            repo_slug: "someone/app".into(),
            protects_gate_crates: false,
            contains_gate_crates: false,
            required_checks: Ok(vec!["ci / test".into()]),
            verdict: Verdict::Accept,
            draft_clean: true,
        }
    }

    #[test]
    fn default_config_has_automerge_off() {
        let c: DreamConfig = serde_json::from_value(serde_json::json!({
            "repo": "o/r", "slots": [{"deep":"d","scan":["src"]}]
        }))
        .unwrap();
        assert!(!c.auto_merge);
        assert_eq!(decide(&c, &facts()), AutomergeDecision::Disabled);
    }

    #[test]
    fn flag_off_is_disabled_whatever_else_is_true() {
        assert_eq!(decide(&cfg(false), &facts()), AutomergeDecision::Disabled);
    }

    #[test]
    fn flag_on_with_everything_in_order_is_allowed() {
        assert_eq!(decide(&cfg(true), &facts()), AutomergeDecision::Allowed);
    }

    #[test]
    fn flag_on_and_this_engines_own_repo_is_refused() {
        for slug in ["DreamLab-AI/dream-engine", "dreamlab-ai/DREAM-ENGINE"] {
            let mut f = facts();
            f.repo_slug = slug.into();
            assert_eq!(
                decide(&cfg(true), &f),
                AutomergeDecision::Refused(Refusal::SelfReferential)
            );
        }
    }

    #[test]
    fn flag_on_and_gate_crates_protected_or_present_is_refused() {
        let mut f = facts();
        f.protects_gate_crates = true;
        assert_eq!(
            decide(&cfg(true), &f),
            AutomergeDecision::Refused(Refusal::SelfReferential)
        );
        let mut f = facts();
        f.contains_gate_crates = true;
        assert_eq!(
            decide(&cfg(true), &f),
            AutomergeDecision::Refused(Refusal::SelfReferential)
        );
    }

    #[test]
    fn flag_on_and_no_required_checks_is_refused() {
        let mut f = facts();
        f.required_checks = Ok(vec![]);
        assert_eq!(
            decide(&cfg(true), &f),
            AutomergeDecision::Refused(Refusal::NoRequiredChecks)
        );
        f.required_checks = Ok(vec!["  ".into()]);
        assert_eq!(
            decide(&cfg(true), &f),
            AutomergeDecision::Refused(Refusal::NoRequiredChecks)
        );
    }

    #[test]
    fn flag_on_and_protection_not_found_or_erroring_is_refused() {
        for e in [
            ForgeError::NotFound,
            ForgeError::Failed("boom".into()),
            ForgeError::Unsupported,
        ] {
            let mut f = facts();
            f.required_checks = Err(e);
            assert!(matches!(
                decide(&cfg(true), &f),
                AutomergeDecision::Refused(Refusal::ChecksUnknown(_))
            ));
        }
    }

    #[test]
    fn only_accept_may_merge() {
        for v in [
            Verdict::Reject,
            Verdict::Inconclusive,
            Verdict::BlockedEnv,
            Verdict::Handoff,
        ] {
            let mut f = facts();
            f.verdict = v;
            assert_eq!(
                decide(&cfg(true), &f),
                AutomergeDecision::Refused(Refusal::NotAccepted)
            );
        }
    }

    #[test]
    fn a_pr_that_is_not_draft_clean_is_refused() {
        let mut f = facts();
        f.draft_clean = false;
        assert_eq!(
            decide(&cfg(true), &f),
            AutomergeDecision::Refused(Refusal::PrNotClean)
        );
    }

    #[test]
    fn self_reference_wins_over_a_perfect_protection_record() {
        let mut f = facts();
        f.repo_slug = "DreamLab-AI/dream-engine".into();
        f.required_checks = Ok(vec!["a".into(), "b".into()]);
        assert!(matches!(
            decide(&cfg(true), &f),
            AutomergeDecision::Refused(_)
        ));
    }

    #[test]
    fn protection_json_yields_contexts_and_checks() {
        let j = r#"{"required_status_checks":{"strict":true,"contexts":["ci"],"checks":[{"context":"ci","app_id":1},{"context":"lint","app_id":null}]}}"#;
        assert_eq!(
            parse_protection(j).unwrap(),
            vec!["ci".to_string(), "lint".to_string()]
        );
    }

    #[test]
    fn protection_json_without_status_checks_is_empty_not_an_error() {
        assert!(parse_protection(r#"{"required_pull_request_reviews":{}}"#)
            .unwrap()
            .is_empty());
        assert!(parse_protection(r#"{"required_status_checks":null}"#)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn garbage_protection_json_is_an_error() {
        assert!(parse_protection("<html>").is_err());
    }
}
