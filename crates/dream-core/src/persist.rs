//! Materialise a candidate as a git branch in an isolated worktree, and
//! publish it as a **draft** pull request.
//!
//! On ACCEPT the model emits its candidate as a unified diff in a
//! ```dream-patch fenced block ([`dream_gate::patch::extract_patch`]). This
//! module applies that diff on an isolated `git worktree` at a chosen base
//! (never touching the operator's working tree) on a namespaced
//! `dream/<deep>-<date>` branch. Publishing is a separate, network-touching
//! step behind the [`Forge`](crate::seams::Forge) seam, so the
//! commit-building logic stays unit-testable offline. The merge stays human:
//! evaluation is not promotion.

use std::path::{Path, PathBuf};

use dream_gate::guard::{self, GuardViolation, ProtectedPolicy};
use dream_gate::patch::normalise_patch;
use thiserror::Error;

use crate::git;

/// Why building or publishing a candidate branch failed.
#[derive(Debug, Error)]
pub enum PersistError {
    /// Filesystem trouble.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A git invocation failed (subcommand, stderr).
    #[error("git {0} failed: {1}")]
    Git(String, String),
    /// Every apply strategy in [`apply_attempts`] failed; carries the last
    /// attempt's stderr so the gate's `DidNotApply` detail says why.
    #[error("patch did not apply: {0}")]
    PatchDidNotApply(String),
    /// The report carried no candidate patch.
    #[error("no candidate patch in report")]
    NoPatch,
    /// The protected-path guard refused the patch. Raised **before** any git
    /// command touches the repository.
    #[error("patch refused by the guard: {0}")]
    Guard(GuardViolation),
}

impl From<git::GitError> for PersistError {
    fn from(e: git::GitError) -> Self {
        match e {
            git::GitError::Spawn(e) => PersistError::Io(e),
            git::GitError::Failed(cmd, err) => PersistError::Git(cmd, err),
        }
    }
}

pub use crate::seams::PrOutcome;

/// `dream/<slug(deep)>-<date>` — the namespaced branch for tonight's candidate.
///
/// ```
/// assert_eq!(dream_core::persist::branch_name("Ledger Signals!", "2026-08-27"),
///            "dream/ledger-signals-2026-08-27");
/// ```
pub fn branch_name(deep: &str, date: &str) -> String {
    let slug: String = deep
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "cycle" } else { slug };
    format!("dream/{}-{}", slug, date)
}

/// The ordered `git apply` strategies tried against the candidate worktree.
///
/// Model-written diffs fail in predictable ways: wrong hunk line counts
/// (`--recount` recomputes them from the hunk body), whitespace drift in
/// context lines (`--ignore-whitespace`), and occasionally a stale base that a
/// three-way merge can still land. `git apply` is atomic without `--reject`,
/// so a failed strategy leaves the tree untouched for the next one. `--3way`
/// only helps when the diff names real blob ids (`index <a>..<b>` lines) that
/// the repo holds, so it is tried last and only then.
pub fn apply_attempts(patch: &str) -> Vec<Vec<&'static str>> {
    let mut v = vec![
        vec!["apply"],
        vec!["apply", "--recount"],
        vec!["apply", "--recount", "--ignore-whitespace"],
    ];
    let names_blobs = patch.lines().any(|l| {
        l.strip_prefix("index ")
            .and_then(|r| r.split_whitespace().next())
            .and_then(|range| range.split_once(".."))
            .is_some_and(|(a, b)| {
                a.len() >= 7
                    && b.len() >= 7
                    && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_hexdigit())
            })
    });
    if names_blobs {
        v.push(vec!["apply", "--3way"]);
    }
    v
}

/// A branch built in an isolated worktree.
#[derive(Debug, Clone)]
pub struct BuiltBranch {
    /// The worktree directory (remove with [`remove_worktree`]).
    pub worktree: PathBuf,
    /// How the patch landed: the `git apply` strategy that succeeded, prefixed
    /// `normalised(…): ` when it only landed after
    /// [`dream_gate::patch::normalise_patch`].
    pub applied_via: String,
    /// The exact diff text that was applied: the patch as written, or its
    /// normalised form when only that landed. Both were checked against the
    /// protected policy before being applied.
    pub applied_patch: String,
}

/// Create the branch + commit in an isolated worktree at `base`, apply the
/// patch. Returns the worktree, or an error after cleaning up the worktree
/// and branch. Does NOT push.
///
/// The protected-path guard runs on `patch` **first**, before any git
/// command, and again on the normalised form if that is what ends up being
/// applied (normalisation synthesises headers and could in principle name a
/// path the original did not). A refusal is [`PersistError::Guard`].
pub fn build_branch_worktree(
    repo: &Path,
    branch: &str,
    patch: &str,
    commit_msg: &str,
    base: &str,
    policy: &ProtectedPolicy,
) -> Result<BuiltBranch, PersistError> {
    guard::check(patch, policy).map_err(PersistError::Guard)?;
    // Unique per (repo, branch): two repos may use the same branch name.
    let tag = &dream_gate::manifest::digest(repo.display().to_string().as_bytes())[..8];
    let wt = std::env::temp_dir().join(format!("dream-wt-{tag}-{}", branch.replace('/', "_")));
    // Stale worktree dir, if any.
    let _ = std::fs::remove_dir_all(&wt);
    // Isolated checkout on a fresh branch — the operator's working tree
    // (uncommitted ledger/report edits) is never touched.
    git::run(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            &wt.display().to_string(),
            base,
        ],
    )?;

    let cleanup = |repo: &Path, wt: &Path| {
        let _ = git::run(
            repo,
            &["worktree", "remove", "--force", &wt.display().to_string()],
        );
        let _ = git::run(repo, &["branch", "-D", branch]);
    };

    let patch_file =
        std::env::temp_dir().join(format!("dream-{tag}-{}.patch", branch.replace('/', "_")));
    if let Err(e) = std::fs::write(&patch_file, patch) {
        cleanup(repo, &wt);
        return Err(e.into());
    }
    // Apply against the worktree, strategy by strategy (see apply_attempts).
    let patch_arg = patch_file.display().to_string();
    let mut last_err = String::new();
    let mut applied_via: Option<String> = None;
    let mut applied_patch = patch.to_string();
    for attempt in apply_attempts(patch) {
        let mut args: Vec<&str> = attempt.clone();
        args.push(patch_arg.as_str());
        match git::run(&wt, &args) {
            Ok(_) => {
                applied_via = Some(attempt.join(" "));
                break;
            }
            Err(e) => {
                last_err = format!("{}: {}", attempt.join(" "), e);
                // A failed --3way can leave conflict state; reset before any
                // further attempt or cleanup.
                let _ = git::run(&wt, &["reset", "--hard", "-q"]);
            }
        }
    }
    // Last resort: the mechanical repairs in normalise_patch, tried only when
    // the diff as written failed every strategy and the repairs changed it.
    if applied_via.is_none() {
        let (normalised, notes) = normalise_patch(patch, |p| crate::source::read_blob(repo, base, p));
        if !notes.is_empty() {
            // The normalised text is what would be applied: guard it too.
            if let Err(v) = guard::check(&normalised, policy) {
                let _ = std::fs::remove_file(&patch_file);
                cleanup(repo, &wt);
                return Err(PersistError::Guard(v));
            }
        }
        if !notes.is_empty() && std::fs::write(&patch_file, &normalised).is_ok() {
            let as_written = last_err.clone();
            for attempt in [
                vec!["apply", "--recount"],
                vec!["apply", "--recount", "--ignore-whitespace"],
            ] {
                let mut args: Vec<&str> = attempt.clone();
                args.push(patch_arg.as_str());
                match git::run(&wt, &args) {
                    Ok(_) => {
                        applied_via = Some(format!(
                            "normalised({}): {}",
                            notes.join("; "),
                            attempt.join(" ")
                        ));
                        applied_patch = normalised.clone();
                        break;
                    }
                    Err(e) => {
                        // Keep the as-written failure first: it is the one that
                        // describes the model's diff.
                        last_err = format!(
                            "{}; after normalising ({}): {}: {}",
                            as_written.trim(),
                            notes.join("; "),
                            attempt.join(" "),
                            e
                        );
                        let _ = git::run(&wt, &["reset", "--hard", "-q"]);
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_file(&patch_file);
    let Some(applied_via) = applied_via else {
        cleanup(repo, &wt);
        return Err(PersistError::PatchDidNotApply(last_err.trim().to_string()));
    };

    if let Err(e) = git::run(&wt, &["add", "-A"]).and_then(|_| git::run(&wt, &["commit", "-m", commit_msg])) {
        cleanup(repo, &wt);
        return Err(e.into());
    }
    Ok(BuiltBranch {
        worktree: wt,
        applied_via,
        applied_patch,
    })
}

/// Delete a local branch. Used when the required-check gate vetoes a candidate:
/// a diff that failed its rerun must not be left behind as a promotable
/// artefact for a human to find and mistake for a validated win.
pub fn delete_branch(repo: &Path, branch: &str) {
    let _ = git::run(repo, &["branch", "-D", branch]);
}

/// Remove a worktree created by [`build_branch_worktree`] (keeps the branch).
pub fn remove_worktree(repo: &Path, wt: &Path) {
    let _ = git::run(
        repo,
        &["worktree", "remove", "--force", &wt.display().to_string()],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn branch_name_slugifies_and_dates() {
        assert_eq!(
            branch_name("sovereign-mesh", "2026-08-27"),
            "dream/sovereign-mesh-2026-08-27"
        );
        assert_eq!(
            branch_name("Ledger Signals!", "2026-08-27"),
            "dream/ledger-signals-2026-08-27"
        );
        assert_eq!(branch_name("", "2026-08-27"), "dream/cycle-2026-08-27");
    }

    /// Scratch repo with one committed file.
    fn scratch(content: &str) -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_path_buf();
        let run = |a: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(a)
                .output()
                .unwrap()
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), content).unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "init"]);
        (d, dir)
    }

    fn show(wt: &Path, spec: &str) -> String {
        let o = Command::new("git")
            .arg("-C")
            .arg(wt)
            .args(["show", spec])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    #[test]
    fn build_branch_worktree_applies_patch_in_isolation() {
        let (_g, dir) = scratch("one\n");
        std::fs::write(dir.join("dirty.txt"), "operator wip\n").unwrap(); // uncommitted
        let patch =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1,2 @@\n one\n+two\n";
        let b = build_branch_worktree(&dir, "dream/x-2026-08-27", patch, "dream: add two", "HEAD", &ProtectedPolicy::default())
            .unwrap();
        assert_eq!(show(&b.worktree, "HEAD:f.txt"), "one\ntwo\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("dirty.txt")).unwrap(),
            "operator wip\n"
        );
        remove_worktree(&dir, &b.worktree);
    }

    #[test]
    fn mid_file_hunk_without_trailing_context_lands_after_normalising() {
        // Leading context, none trailing: git anchors such a hunk to
        // end-of-file, so every plain strategy refuses it.
        let (_g, dir) = scratch("a\nb\nc\nd\ne\nf\ng\n");
        let patch = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,3 @@\n a\n b\n-c\n+C\n";
        let b = build_branch_worktree(&dir, "dream/trail-2026-10-11", patch, "m", "HEAD", &ProtectedPolicy::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(b.worktree.join("f.txt")).unwrap(),
            "a\nb\nC\nd\ne\nf\ng\n"
        );
        assert!(b.applied_via.starts_with("normalised("), "{}", b.applied_via);
        assert!(b.applied_via.contains("trailing context"));
        remove_worktree(&dir, &b.worktree);
    }

    #[test]
    fn headerless_two_file_patch_lands_after_normalising() {
        // No `diff --git` lines. With --recount git read the second file's
        // `--- a/…` header as a deletion in the first hunk.
        let (_g, dir) = scratch("one\ntwo\nthree\n");
        std::fs::write(dir.join("g.txt"), "x\ny\nz\n").unwrap();
        for a in [["add", "g.txt"], ["commit", "-qm"]] {
            let mut c = Command::new("git");
            c.arg("-C").arg(&dir).args(a);
            if a[0] == "commit" {
                c.arg("g");
            }
            c.output().unwrap();
        }
        let patch = "--- a/f.txt\n+++ b/f.txt\n@@ -1,9 +1,9 @@\n one\n-two\n+TWO\n three\n\n--- a/g.txt\n+++ b/g.txt\n@@ -1,9 +1,9 @@\n x\n-y\n+Y\n z\n";
        let b = build_branch_worktree(&dir, "dream/headers-2026-10-11", patch, "m", "HEAD", &ProtectedPolicy::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(b.worktree.join("f.txt")).unwrap(),
            "one\nTWO\nthree\n"
        );
        assert_eq!(
            std::fs::read_to_string(b.worktree.join("g.txt")).unwrap(),
            "x\nY\nz\n"
        );
        assert!(b.applied_via.contains("header synthesised"), "{}", b.applied_via);
        remove_worktree(&dir, &b.worktree);
    }

    #[test]
    fn apply_attempts_order_and_three_way_gate() {
        let plain = "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(
            apply_attempts(plain),
            vec![
                vec!["apply"],
                vec!["apply", "--recount"],
                vec!["apply", "--recount", "--ignore-whitespace"],
            ]
        );
        let with_blobs = "diff --git a/f b/f\nindex 1234abc..5678def 100644\n--- a/f\n+++ b/f\n";
        assert_eq!(
            apply_attempts(with_blobs).last().unwrap(),
            &vec!["apply", "--3way"]
        );
        assert_eq!(apply_attempts("index xyz..abc\n").len(), 3);
    }

    #[test]
    fn miscounted_hunk_lands_via_recount() {
        let (_g, dir) = scratch("one\n");
        let patch =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1,5 @@\n one\n+two\n";
        let b = build_branch_worktree(&dir, "dream/recount-2026-09-25", patch, "m", "HEAD", &ProtectedPolicy::default()).unwrap();
        assert_eq!(show(&b.worktree, "HEAD:f.txt"), "one\ntwo\n");
        remove_worktree(&dir, &b.worktree);
    }

    #[test]
    fn unappliable_patch_reports_last_strategy_detail_and_cleans_up() {
        let (_g, dir) = scratch("one\n");
        let patch =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-absent\n+x\n";
        let err =
            build_branch_worktree(&dir, "dream/noapply-2026-09-25", patch, "m", "HEAD", &ProtectedPolicy::default()).unwrap_err();
        match err {
            PersistError::PatchDidNotApply(detail) => {
                assert!(
                    detail.starts_with("apply --recount --ignore-whitespace"),
                    "{detail}"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        let br = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["branch", "--list", "dream/noapply-2026-09-25"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&br.stdout).trim().is_empty());
    }

    #[test]
    fn worktree_can_be_pinned_to_an_older_base() {
        let (_g, dir) = scratch("one\n");
        let base = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        std::fs::write(dir.join("f.txt"), "moved\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["commit", "-qam", "later"])
            .output()
            .unwrap();
        let patch =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1,2 @@\n one\n+two\n";
        let b = build_branch_worktree(&dir, "dream/base-2026-09-25", patch, "m", &base, &ProtectedPolicy::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(b.worktree.join("f.txt")).unwrap(),
            "one\ntwo\n"
        );
        remove_worktree(&dir, &b.worktree);
    }

    #[test]
    fn protected_patch_is_refused_before_any_worktree_exists() {
        let (_g, dir) = scratch("one\n");
        let patch = "diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml\n--- a/.github/workflows/ci.yml\n+++ b/.github/workflows/ci.yml\n@@ -1 +1 @@\n-a\n+b\n";
        let err = build_branch_worktree(
            &dir,
            "dream/prot-2026-10-11",
            patch,
            "m",
            "HEAD",
            &ProtectedPolicy::default(),
        )
        .unwrap_err();
        assert!(matches!(err, PersistError::Guard(GuardViolation::Protected { .. })), "{err:?}");
        let wl = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["worktree", "list"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&wl.stdout).lines().count(), 1);
        let br = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["branch", "--list", "dream/*"])
            .output()
            .unwrap();
        assert!(br.stdout.is_empty());
    }
}
