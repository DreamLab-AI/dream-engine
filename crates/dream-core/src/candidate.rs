//! Apply the emitted candidate in isolation and re-run the required evaluators.
//!
//! Everything the model saw was evidence about the *baseline*; the diff it
//! then wrote was never tested. This module closes that loop.
//!
//! The candidate is materialised on a git worktree at the base revision on a
//! fresh branch, so the operator's working tree is never touched, and the
//! worktree's tree hash is recorded so the rerun's receipts bind to exact
//! bytes rather than to a branch name. The required evaluators then run
//! *against that tree*, through the same [`EvaluatorRunner`] seam as the
//! baseline, producing the receipts the gate reads.
//!
//! [`run`] is the one entry point that enforces the order **refuse → guard →
//! worktree → build → evaluate**. A patch that touches a protected path (or
//! cannot be parsed with certainty) is stopped by [`guard::check`] *before any
//! git command, build or evaluator runs*, and its [`CandidateState`] says so.

use std::path::{Path, PathBuf};

use dream_gate::gate::CandidateState;
use dream_gate::guard::{self, GuardViolation, ProtectedPolicy};
use dream_gate::manifest::{digest, EvaluatorIdentity};
use dream_gate::outcome::ExecOutcome;
use dream_gate::receipts::{EvaluatorReceipt, Phase};
use tracing::{info, warn};

use crate::persist::{self, PersistError};
use crate::runner::EvaluatorRunner;

/// A candidate that exists on disk as a real tree, ready to be evaluated.
#[derive(Debug, Clone)]
pub struct PreparedCandidate {
    /// The worktree directory.
    pub worktree: PathBuf,
    /// The candidate branch.
    pub branch: String,
    /// `git rev-parse HEAD^{tree}` inside the worktree — the exact content the
    /// rerun evaluates.
    pub tree_hash: String,
    /// `sha256` of the patch that was applied.
    pub patch_digest: String,
    /// Size of that patch in bytes.
    pub patch_bytes: usize,
    /// How the patch landed (see [`persist::BuiltBranch::applied_via`]).
    pub applied_via: String,
    /// The exact diff text that was applied (see
    /// [`persist::BuiltBranch::applied_patch`]); this is what the gate's
    /// re-check must see.
    pub applied_patch: String,
}

/// Build the candidate tree: a worktree at `base`, on `branch`, with `patch`
/// applied and committed.
///
/// The protected-path guard runs first ([`PersistError::Guard`]), before any
/// git command. Returns [`PersistError::PatchDidNotApply`] when the diff will
/// not land — a harness-class veto, not evidence about the code.
pub fn prepare(
    repo: &Path,
    branch: &str,
    patch: &str,
    commit_msg: &str,
    base: &str,
    policy: &ProtectedPolicy,
) -> Result<PreparedCandidate, PersistError> {
    let built = persist::build_branch_worktree(repo, branch, patch, commit_msg, base, policy)?;
    let tree_hash = crate::git::run(&built.worktree, &["rev-parse", "HEAD^{tree}"])
        .map(|s| s.trim().to_string())
        .map_err(|e| PersistError::Git("rev-parse HEAD^{tree}".into(), e.to_string()))?;
    Ok(PreparedCandidate {
        worktree: built.worktree,
        branch: branch.to_string(),
        tree_hash,
        patch_digest: digest(patch.as_bytes()),
        patch_bytes: patch.len(),
        applied_via: built.applied_via,
        applied_patch: built.applied_patch,
    })
}

/// Remove the candidate worktree, keeping the branch (the draft-PR path may
/// still want to push it).
pub fn cleanup(repo: &Path, prepared: &PreparedCandidate) {
    persist::remove_worktree(repo, &prepared.worktree);
}

/// Remove the candidate worktree *and* its branch — used when the gate rejects
/// the candidate, so a vetoed diff leaves no promotable artefact behind.
pub fn discard(repo: &Path, prepared: &PreparedCandidate) {
    persist::remove_worktree(repo, &prepared.worktree);
    persist::delete_branch(repo, &prepared.branch);
}

/// Re-run the required evaluators against the candidate tree at `work_dir`.
///
/// Only required evaluators run: they are the ones the gate reads, and runner
/// time inside the nightly window is finite. Advisory evaluators keep their
/// baseline receipts.
pub fn evaluate(
    runner: &dyn EvaluatorRunner,
    work_dir: &str,
    required: &[&EvaluatorIdentity],
) -> Vec<EvaluatorReceipt> {
    let mut out = Vec::new();
    for id in required {
        info!(evaluator = %id.name, runner = %runner.describe(), "candidate rerun");
        let exec = runner.run(work_dir, &id.command, id.timeout_secs);
        let receipt = EvaluatorReceipt::from_exec(
            &id.name,
            &id.command,
            Phase::Candidate,
            true,
            id.timeout_secs,
            exec,
        );
        if !receipt.outcome.is_pass() {
            warn!(
                evaluator = %id.name,
                outcome = receipt.outcome.label(),
                "candidate rerun did not pass — acceptance will be vetoed"
            );
        }
        out.push(receipt);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Everything [`run`] needs.
pub struct AttemptInput<'a> {
    /// The checkout the branch is cut from.
    pub repo: &'a Path,
    /// The candidate branch name.
    pub branch: &'a str,
    /// The patch exactly as extracted from the report.
    pub patch: &'a str,
    /// Commit message for the candidate commit.
    pub commit_msg: &'a str,
    /// Base revision (the commit the model was shown).
    pub base: &'a str,
    /// The protected-path policy, built from the **baseline** config.
    pub policy: &'a ProtectedPolicy,
    /// Optional build command and its timeout, run in the candidate tree
    /// before the evaluators.
    pub build: Option<(&'a str, u64)>,
    /// The required evaluators to re-run.
    pub required: &'a [&'a EvaluatorIdentity],
}

/// Why [`run`] could not build a candidate, as data rather than text. The
/// matching [`CandidateState`] still carries the human-readable detail the gate
/// and ledger use; this is what a host branches on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttemptError {
    /// Git refused the diff itself (every apply strategy failed). `stderr` is
    /// the last attempt's git output, without the `patch did not apply: ` prefix
    /// the state's detail adds — feed it to an apply-repair pass.
    PatchDidNotApply {
        /// Git's refusal text.
        stderr: String,
    },
    /// The tree was built but the host's [`Relocate`] callback could not place
    /// it where the evaluators run.
    Provisioning {
        /// The callback's error.
        detail: String,
    },
    /// A git or filesystem failure while preparing the tree (not a refusal of
    /// the diff).
    Prepare {
        /// What failed, as text.
        detail: String,
    },
}

/// What [`run`] produced; feed `state` and `receipts` to
/// [`dream_gate::gate::decide`].
#[derive(Debug)]
pub struct Attempt {
    /// The structural state of the candidate for the gate.
    pub state: CandidateState,
    /// The prepared tree, when one was built (callers publish or
    /// [`discard`] it).
    pub prepared: Option<PreparedCandidate>,
    /// Receipts of the required evaluators on the candidate tree.
    pub receipts: Vec<EvaluatorReceipt>,
    /// The candidate build step's outcome, when one ran.
    pub build: Option<ExecOutcome>,
    /// Why no candidate was built, when that was an apply or provisioning
    /// failure. `None` for an applied candidate and for guard refusals, which
    /// are fully described by `state`.
    pub error: Option<AttemptError>,
}

/// Where the candidate tree should be evaluated, when not in its own
/// worktree: a host that runs evaluators elsewhere (a remote machine, a
/// container) copies the tree and returns the directory to use there, or an
/// error string that becomes a `DidNotApply` harness veto.
pub type Relocate<'a> = &'a dyn Fn(&PreparedCandidate) -> Result<String, String>;

/// Map a guard refusal to the gate's vocabulary: a protected path is
/// [`CandidateState::Protected`] (evidence, `REJECT`); a patch the guard
/// cannot parse with certainty, or that is otherwise unsafe, is
/// [`CandidateState::Refused`] (unproven, `INCONCLUSIVE`) with a `guard:`
/// prefix. Both are model faults, neither is a harness fault.
fn refusal(v: GuardViolation) -> CandidateState {
    match v {
        GuardViolation::Protected { paths } => CandidateState::Protected { paths },
        other => CandidateState::Refused {
            detail: format!("guard: {other}"),
        },
    }
}

/// Prepare, build and evaluate a candidate, in the only safe order.
///
/// 1. a patch that deletes a binary file is [`CandidateState::Refused`];
/// 2. [`guard::check`] runs on the patch: a protected path gives
///    [`CandidateState::Protected`], anything else it cannot parse with
///    certainty gives [`CandidateState::Refused`] — in both cases **no git
///    command, build or evaluator has run**;
/// 3. only then is the worktree built, the optional build step run and the
///    required evaluators re-run.
pub fn run(
    input: &AttemptInput<'_>,
    runner: &dyn EvaluatorRunner,
    relocate: Option<Relocate<'_>>,
) -> Attempt {
    let none = |state| Attempt {
        state,
        prepared: None,
        receipts: Vec::new(),
        build: None,
        error: None,
    };
    if dream_gate::patch::deletes_binary(input.patch) {
        warn!("candidate patch deletes a binary file — refused before applying");
        return none(CandidateState::Refused {
            detail: "the patch deletes a binary file".into(),
        });
    }
    // The guard comes first: nothing below this point may run for a patch the
    // policy refuses.
    if let Err(v) = guard::check(input.patch, input.policy) {
        return none(refusal(v));
    }
    // A stale branch from an interrupted attempt would block the worktree.
    persist::delete_branch(input.repo, input.branch);
    let prepared = match prepare(
        input.repo,
        input.branch,
        input.patch,
        input.commit_msg,
        input.base,
        input.policy,
    ) {
        Ok(p) => p,
        // prepare re-checks the normalised text it actually applies.
        Err(PersistError::Guard(v)) => return none(refusal(v)),
        Err(e) => {
            let error = match &e {
                PersistError::PatchDidNotApply(stderr) => AttemptError::PatchDidNotApply {
                    stderr: stderr.clone(),
                },
                other => AttemptError::Prepare {
                    detail: other.to_string(),
                },
            };
            let mut a = none(CandidateState::DidNotApply {
                detail: e.to_string(),
            });
            a.error = Some(error);
            return a;
        }
    };
    let work_dir = match relocate {
        Some(f) => match f(&prepared) {
            Ok(d) => d,
            Err(e) => {
                let state = CandidateState::DidNotApply {
                    detail: format!("provisioning failed: {e}"),
                };
                return Attempt {
                    state,
                    prepared: Some(prepared),
                    receipts: Vec::new(),
                    build: None,
                    error: Some(AttemptError::Provisioning { detail: e }),
                };
            }
        },
        None => prepared.worktree.display().to_string(),
    };
    let build = input.build.map(|(cmd, timeout)| {
        let b = runner.run(&work_dir, cmd, timeout);
        info!(exit = ?b.exit_code, "candidate build");
        b
    });
    let receipts = evaluate(runner, &work_dir, input.required);
    Attempt {
        state: CandidateState::Applied {
            tree_hash: prepared.tree_hash.clone(),
            patch: prepared.applied_patch.clone(),
        },
        prepared: Some(prepared),
        receipts,
        build,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::LocalRunner;
    use dream_gate::gate::{self, GateContext};
    use dream_gate::manifest::{ExperimentManifest, ModelIdentity};
    use dream_gate::verdict::Verdict;
    use std::process::Command;
    use std::sync::Mutex;

    /// A scratch repo whose "evaluator" is a shell check over its own source.
    fn scratch_repo() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let run = |a: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(a)
                .output()
                .unwrap()
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "dream@test"]);
        run(&["config", "user.name", "dream"]);
        std::fs::write(repo.join("value.txt"), "42\n").unwrap();
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        std::fs::create_dir_all(repo.join(".github/workflows")).unwrap();
        std::fs::write(repo.join(".github/workflows/ci.yml"), "name: ci\n").unwrap();
        std::fs::write(
            repo.join("scripts/check.sh"),
            "#!/bin/bash\nv=$(cat value.txt)\nif [ \"$v\" = \"42\" ]; then echo \"check ok: $v\"; exit 0; fi\necho \"FAIL: expected 42, got $v\"\nexit 1\n",
        )
        .unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "init"]);
        (d, repo)
    }

    fn manifest(deep: &str) -> ExperimentManifest {
        ExperimentManifest {
            schema: 1,
            run_id: "abcdef0123456789".into(),
            night_id: "2026-09-05-scratch".into(),
            repo: "scratch".into(),
            repo_slug: "o/scratch".into(),
            date: "2026-09-05".into(),
            day_int: 20260905,
            deep: deep.into(),
            scan: vec![],
            baseline_revision: "0".repeat(40),
            baseline_tree_hash: "0".repeat(40),
            config_digest: digest(b"cfg"),
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
                model: "test-model".into(),
                max_tokens: 1024,
                fallback: None,
            },
            engine_version: "0.1.0".into(),
            created_at: "2026-09-05T01:00:00Z".into(),
        }
    }

    fn policy() -> ProtectedPolicy {
        ProtectedPolicy::default().with_evaluator_commands(["bash scripts/check.sh"])
    }

    fn accept() -> Result<Verdict, dream_gate::verdict::VerdictParseError> {
        Ok(Verdict::Accept)
    }

    /// Records every command it is asked to run, then delegates.
    struct Recording(Mutex<Vec<String>>);
    impl Recording {
        fn new() -> Self {
            Self(Mutex::new(vec![]))
        }
        fn calls(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }
    impl EvaluatorRunner for Recording {
        fn run(&self, work_dir: &str, command: &str, timeout_secs: u64) -> ExecOutcome {
            self.0.lock().unwrap().push(command.to_string());
            LocalRunner.run(work_dir, command, timeout_secs)
        }
        fn describe(&self) -> String {
            "recording".into()
        }
    }

    fn branches(repo: &Path) -> String {
        let o = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["branch", "--list", "dream/*"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn worktrees(repo: &Path) -> usize {
        let o = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.starts_with("worktree "))
            .count()
    }

    #[test]
    fn a_good_candidate_is_applied_isolated_and_passes_the_rerun() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/README.md b/README.md\nnew file mode 100644\n--- /dev/null\n+++ b/README.md\n@@ -0,0 +1 @@\n+notes\n";
        let c = prepare(
            &repo,
            "dream/good-2026-09-05",
            patch,
            "dream: add notes",
            "HEAD",
            &policy(),
        )
        .unwrap();
        assert_eq!(c.tree_hash.len(), 40);
        assert_eq!(c.patch_digest, digest(patch.as_bytes()));
        let m = manifest("value");
        let receipts = evaluate(&LocalRunner, c.worktree.to_str().unwrap(), &m.required());
        assert_eq!(receipts.len(), 1);
        assert!(receipts[0].outcome.is_pass(), "{:?}", receipts[0]);
        assert_eq!(receipts[0].phase, Phase::Candidate);
        // The main tree is untouched.
        assert!(!repo.join("README.md").exists());
        let pol = policy();
        let d = gate::decide(
            &m,
            &accept(),
            &CandidateState::Applied {
                tree_hash: c.tree_hash.clone(),
                patch: patch.to_string(),
            },
            &receipts,
            &GateContext::new(&pol),
        );
        assert!(d.accepted, "{d:?}");
        discard(&repo, &c);
        assert!(branches(&repo).is_empty());
    }

    #[test]
    fn a_candidate_that_breaks_the_evaluator_is_vetoed() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/value.txt b/value.txt\n--- a/value.txt\n+++ b/value.txt\n@@ -1 +1 @@\n-42\n+41\n";
        let c = prepare(
            &repo,
            "dream/bad-2026-09-05",
            patch,
            "dream: break",
            "HEAD",
            &policy(),
        )
        .unwrap();
        let m = manifest("value");
        let receipts = evaluate(&LocalRunner, c.worktree.to_str().unwrap(), &m.required());
        assert!(!receipts[0].outcome.is_pass());
        let pol = policy();
        let d = gate::decide(
            &m,
            &accept(),
            &CandidateState::Applied {
                tree_hash: c.tree_hash.clone(),
                patch: patch.to_string(),
            },
            &receipts,
            &GateContext::new(&pol),
        );
        assert!(!d.accepted);
        assert_eq!(d.verdict, "REJECT");
        discard(&repo, &c);
    }

    #[test]
    fn a_patch_that_does_not_apply_is_a_harness_class_error() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/value.txt b/value.txt\n--- a/value.txt\n+++ b/value.txt\n@@ -1 +1 @@\n-nope\n+x\n";
        assert!(matches!(
            prepare(
                &repo,
                "dream/noapply-2026-09-05",
                patch,
                "m",
                "HEAD",
                &policy()
            ),
            Err(PersistError::PatchDidNotApply(_))
        ));
        assert!(branches(&repo).is_empty());
    }

    fn input<'a>(
        repo: &'a Path,
        patch: &'a str,
        pol: &'a ProtectedPolicy,
        build: Option<(&'a str, u64)>,
        required: &'a [&'a EvaluatorIdentity],
    ) -> AttemptInput<'a> {
        AttemptInput {
            repo,
            branch: "dream/attempt-2026-09-05",
            patch,
            commit_msg: "dream: attempt",
            base: "HEAD",
            policy: pol,
            build,
            required,
        }
    }

    /// The guard runs BEFORE anything is applied: a protected patch must not
    /// create a branch or worktree, run the build, or run any evaluator.
    #[test]
    fn a_protected_patch_never_reaches_git_the_build_or_an_evaluator() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml\n--- a/.github/workflows/ci.yml\n+++ b/.github/workflows/ci.yml\n@@ -1 +1 @@\n-name: ci\n+name: pwned\n";
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let rec = Recording::new();
        let a = run(
            &input(&repo, patch, &pol, Some(("echo building", 30)), &required),
            &rec,
            None,
        );
        assert!(
            matches!(&a.state, CandidateState::Protected { paths } if paths == &vec![".github/workflows/ci.yml".to_string()]),
            "{:?}",
            a.state
        );
        assert!(rec.calls().is_empty(), "ran: {:?}", rec.calls());
        assert!(a.prepared.is_none() && a.receipts.is_empty() && a.build.is_none());
        assert!(
            branches(&repo).is_empty(),
            "branch created for a protected patch"
        );
        assert_eq!(
            worktrees(&repo),
            1,
            "worktree created for a protected patch"
        );
        // The gate turns that state into a REJECT with a protected-path veto.
        let d = gate::decide(
            &m,
            &accept(),
            &a.state,
            &a.receipts,
            &GateContext::new(&pol),
        );
        assert!(!d.accepted);
        assert_eq!(d.verdict, "REJECT");
        assert!(d.vetoes.iter().any(|v| v.subject == "protected-path"));
    }

    #[test]
    fn a_patch_touching_the_evaluator_script_is_protected_too() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/scripts/check.sh b/scripts/check.sh\n--- a/scripts/check.sh\n+++ b/scripts/check.sh\n@@ -1 +1 @@\n-#!/bin/bash\n+#!/bin/sh\n";
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let rec = Recording::new();
        let a = run(&input(&repo, patch, &pol, None, &required), &rec, None);
        assert!(
            matches!(a.state, CandidateState::Protected { .. }),
            "{:?}",
            a.state
        );
        assert!(rec.calls().is_empty());
        assert!(branches(&repo).is_empty());
    }

    #[test]
    fn an_unparseable_patch_is_refused_before_any_git_command() {
        let (_d, repo) = scratch_repo();
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let rec = Recording::new();
        let a = run(
            &input(&repo, "this is not a diff at all\n", &pol, None, &required),
            &rec,
            None,
        );
        assert!(
            matches!(&a.state, CandidateState::Refused { detail } if detail.starts_with("guard:")),
            "{:?}",
            a.state
        );
        assert!(rec.calls().is_empty());
        assert!(branches(&repo).is_empty());
        assert_eq!(worktrees(&repo), 1);
    }

    #[test]
    fn binary_deletion_is_refused_before_applying() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/x.png b/x.png\ndeleted file mode 100644\nindex 1..0\nBinary files a/x.png and /dev/null differ\n";
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let rec = Recording::new();
        let a = run(&input(&repo, patch, &pol, None, &required), &rec, None);
        assert!(matches!(a.state, CandidateState::Refused { .. }));
        assert!(rec.calls().is_empty());
    }

    #[test]
    fn a_clean_patch_runs_build_then_evaluators_in_the_candidate_tree() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/README.md b/README.md\nnew file mode 100644\n--- /dev/null\n+++ b/README.md\n@@ -0,0 +1 @@\n+notes\n";
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let rec = Recording::new();
        let a = run(
            &input(
                &repo,
                patch,
                &pol,
                Some(("test -f README.md", 30)),
                &required,
            ),
            &rec,
            None,
        );
        assert_eq!(
            rec.calls(),
            vec!["test -f README.md", "bash scripts/check.sh"]
        );
        assert_eq!(
            a.build.as_ref().unwrap().exit_code,
            Some(0),
            "build ran in the candidate tree"
        );
        let CandidateState::Applied { patch: carried, .. } = &a.state else {
            panic!("{:?}", a.state)
        };
        assert_eq!(carried, patch, "the applied patch is carried into the gate");
        assert!(a.receipts[0].outcome.is_pass());
        let d = gate::decide(
            &m,
            &accept(),
            &a.state,
            &a.receipts,
            &GateContext::new(&pol),
        );
        assert!(d.accepted, "{d:?}");
        discard(&repo, a.prepared.as_ref().unwrap());
    }

    #[test]
    fn relocation_failure_is_a_harness_veto_and_keeps_the_tree_for_cleanup() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/README.md b/README.md\nnew file mode 100644\n--- /dev/null\n+++ b/README.md\n@@ -0,0 +1 @@\n+notes\n";
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let rec = Recording::new();
        let fail = |_: &PreparedCandidate| Err("no route".to_string());
        let a = run(
            &input(&repo, patch, &pol, None, &required),
            &rec,
            Some(&fail),
        );
        assert!(
            matches!(&a.state, CandidateState::DidNotApply { detail } if detail.contains("no route"))
        );
        assert!(a.prepared.is_some());
        assert!(rec.calls().is_empty());
        discard(&repo, a.prepared.as_ref().unwrap());
    }

    #[test]
    fn a_non_applying_patch_carries_a_typed_error_with_git_stderr() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/value.txt b/value.txt\n--- a/value.txt\n+++ b/value.txt\n@@ -1 +1 @@\n-nope\n+x\n";
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let rec = Recording::new();
        let a = run(&input(&repo, patch, &pol, None, &required), &rec, None);
        let Some(AttemptError::PatchDidNotApply { stderr }) = &a.error else {
            panic!("{:?}", a.error)
        };
        assert!(!stderr.is_empty());
        // The state text is unchanged, so ledger rows and digests still read it.
        assert_eq!(
            a.state,
            CandidateState::DidNotApply {
                detail: format!("patch did not apply: {stderr}")
            }
        );
        assert!(rec.calls().is_empty());
    }

    #[test]
    fn provisioning_failure_is_typed_and_distinct_from_a_git_refusal() {
        let (_d, repo) = scratch_repo();
        let patch = "diff --git a/README.md b/README.md\nnew file mode 100644\n--- /dev/null\n+++ b/README.md\n@@ -0,0 +1 @@\n+notes\n";
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let fail = |_: &PreparedCandidate| Err("no route".to_string());
        let a = run(
            &input(&repo, patch, &pol, None, &required),
            &Recording::new(),
            Some(&fail),
        );
        assert_eq!(
            a.error,
            Some(AttemptError::Provisioning {
                detail: "no route".into()
            })
        );
        discard(&repo, a.prepared.as_ref().unwrap());
    }

    #[test]
    fn guard_refusals_and_applied_candidates_carry_no_error() {
        let (_d, repo) = scratch_repo();
        let pol = policy();
        let m = manifest("value");
        let required = m.required();
        let a = run(
            &input(&repo, "not a diff\n", &pol, None, &required),
            &Recording::new(),
            None,
        );
        assert!(a.error.is_none(), "{:?}", a.error);
        let ok = "diff --git a/README.md b/README.md\nnew file mode 100644\n--- /dev/null\n+++ b/README.md\n@@ -0,0 +1 @@\n+notes\n";
        let a = run(
            &input(&repo, ok, &pol, None, &required),
            &Recording::new(),
            None,
        );
        assert!(a.error.is_none());
        discard(&repo, a.prepared.as_ref().unwrap());
    }
}
