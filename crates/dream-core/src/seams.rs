//! The four seams a host plugs into: a language model ([`Llm`]), a durable
//! memory ([`MemorySink`]), an operator channel ([`Notifier`]) and a code
//! forge ([`Forge`]).
//!
//! The async traits return a plain-std [`BoxFut`], so this crate has no
//! executor dependency: any runtime (or none) can drive them.
//!
//! # Example
//!
//! ```
//! use dream_core::seams::{Llm, LlmError, LlmRequest, BoxFut};
//! use dream_gate::manifest::ModelIdentity;
//!
//! struct Echo;
//! impl Llm for Echo {
//!     fn complete<'a>(&'a self, req: &'a LlmRequest) -> BoxFut<'a, Result<String, LlmError>> {
//!         Box::pin(async move { Ok(format!("echo: {}", req.prompt)) })
//!     }
//!     fn identity(&self) -> ModelIdentity {
//!         ModelIdentity { provider: "local".into(), model: "echo".into(), max_tokens: 16, fallback: None }
//!     }
//! }
//!
//! let reply = futures::executor::block_on(Echo.complete(&LlmRequest::new("hi", 16)));
//! assert_eq!(reply.unwrap(), "echo: hi");
//! ```

use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;

use dream_gate::manifest::ModelIdentity;
use serde::{Deserialize, Serialize};

/// A boxed, sendable future borrowed for `'a`. The only async vocabulary in
/// the crate's public traits.
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---------------------------------------------------------------------------
// Llm
// ---------------------------------------------------------------------------

/// One completion request.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmRequest {
    /// The full prompt text.
    pub prompt: String,
    /// Output token budget.
    pub max_tokens: u32,
    /// Sampling temperature; `None` leaves it to the provider default.
    pub temperature: Option<f32>,
}

impl LlmRequest {
    /// A request with the provider's default temperature.
    pub fn new(prompt: impl Into<String>, max_tokens: u32) -> Self {
        Self {
            prompt: prompt.into(),
            max_tokens,
            temperature: None,
        }
    }
}

/// Why a completion failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LlmError {
    /// The request never completed (connection, timeout, gateway 5xx).
    #[error("transport: {0}")]
    Transport(String),
    /// The provider answered with an error.
    #[error("API error: {0}")]
    Api(String),
    /// The provider answered with an empty body.
    #[error("empty response from model")]
    Empty,
    /// Credentials were absent or refused.
    #[error("missing credentials: {0}")]
    MissingCredentials(String),
}

impl LlmError {
    /// True for failures worth one retry or a fallback model.
    pub fn is_transient(&self) -> bool {
        match self {
            LlmError::Transport(_) | LlmError::Empty => true,
            LlmError::Api(m) => m.contains("HTTP 5"),
            LlmError::MissingCredentials(_) => false,
        }
    }
}

/// A language model. Implemented by the host.
pub trait Llm: Send + Sync {
    /// Complete `req`.
    fn complete<'a>(&'a self, req: &'a LlmRequest) -> BoxFut<'a, Result<String, LlmError>>;
    /// The identity frozen into the experiment manifest before any call.
    fn identity(&self) -> ModelIdentity;
}

// ---------------------------------------------------------------------------
// MemorySink
// ---------------------------------------------------------------------------

/// One durable memory entry about a night.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    /// Stable key, unique per night and kind.
    pub key: String,
    /// What this entry is (`night`, `finding`, …); free text chosen by the host.
    pub kind: String,
    /// Repository the night ran on.
    pub repo: String,
    /// Night identifier.
    pub night_id: String,
    /// Verdict token recorded for the night.
    pub verdict: String,
    /// The entry text.
    pub body: String,
}

/// Why a [`MemorySink`] write failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("memory sink: {0}")]
pub struct SinkError(pub String);

/// A durable memory. Writes are best-effort: callers log and continue.
pub trait MemorySink: Send + Sync {
    /// Persist `rec`.
    fn record<'a>(&'a self, rec: &'a MemoryRecord) -> BoxFut<'a, Result<(), SinkError>>;
}

/// Discards every record.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullSink;

impl MemorySink for NullSink {
    fn record<'a>(&'a self, _rec: &'a MemoryRecord) -> BoxFut<'a, Result<(), SinkError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Appends each record as one JSON line to a file.
///
/// ```
/// use dream_core::seams::{JsonlSink, MemoryRecord, MemorySink};
/// let dir = tempfile::tempdir().unwrap();
/// let sink = JsonlSink::new(dir.path().join("memory.jsonl"));
/// let rec = MemoryRecord { key: "k".into(), kind: "night".into(), repo: "r".into(),
///     night_id: "n".into(), verdict: "REJECT".into(), body: "b".into() };
/// futures::executor::block_on(sink.record(&rec)).unwrap();
/// assert_eq!(std::fs::read_to_string(dir.path().join("memory.jsonl")).unwrap().lines().count(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct JsonlSink {
    path: PathBuf,
}

impl JsonlSink {
    /// A sink writing to `path` (parent directories are created on demand).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn append(&self, rec: &MemoryRecord) -> Result<(), SinkError> {
        let err = |e: &dyn std::fmt::Display| SinkError(format!("{}: {e}", self.path.display()));
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| err(&e))?;
        }
        let mut line = serde_json::to_string(rec).map_err(|e| err(&e))?;
        line.push('\n');
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| err(&e))?;
        f.write_all(line.as_bytes()).map_err(|e| err(&e))
    }
}

impl MemorySink for JsonlSink {
    fn record<'a>(&'a self, rec: &'a MemoryRecord) -> BoxFut<'a, Result<(), SinkError>> {
        Box::pin(async move { self.append(rec) })
    }
}

// ---------------------------------------------------------------------------
// Notifier
// ---------------------------------------------------------------------------

/// Something an operator should hear about.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Notice {
    /// End-of-night digest.
    NightSummary {
        /// Repository name.
        repo: String,
        /// Night identifier.
        night_id: String,
        /// Verdict token.
        verdict: String,
        /// The ledger finding.
        finding: String,
    },
    /// A verified candidate whose draft PR never opened.
    LostPr {
        /// Directory name of the checkout holding the verified branch.
        repo_dir: String,
        /// `owner/name` slug.
        repo_slug: String,
        /// The local branch holding the verified commit.
        branch: String,
        /// Whether the push itself succeeded.
        pushed: bool,
    },
    /// The same environment failures recurred unchanged across nights, so the
    /// harness (not the candidate) needs a human.
    HarnessStuck {
        /// Repository name.
        repo: String,
        /// The persisting failure fingerprints, hex.
        fingerprints: Vec<String>,
        /// Night identifier at which the stall was detected.
        night_id: String,
    },
    /// An ACCEPT survived the gate.
    Accepted {
        /// Repository name.
        repo: String,
        /// Night identifier.
        night_id: String,
        /// Branch carrying the candidate.
        branch: String,
        /// URL of the draft PR, when one opened.
        pr_url: Option<String>,
    },
}

impl Notice {
    /// Human-readable text for the notice.
    pub fn render(&self) -> String {
        match self {
            Notice::NightSummary {
                repo,
                night_id,
                verdict,
                finding,
            } => format!("[{repo}] {night_id}: {verdict} — {finding}"),
            Notice::LostPr {
                repo_dir,
                repo_slug,
                branch,
                pushed,
            } => {
                let what = if *pushed {
                    "pushed the branch but could not open the draft pull request".to_string()
                } else {
                    format!(
                        "could not push the branch to {repo_slug} (a branch with the same name \
                         may already exist there, for example from a second checkout)"
                    )
                };
                format!(
                    "A change on `{branch}` passed every required check, but the engine {what}. \
                     The verified commit is kept on the local branch in {repo_dir}. To keep it: \
                     `git -C {repo_dir} push origin {branch}` (or push it under a new name), \
                     then `gh pr create --repo {repo_slug} --draft --head {branch}`."
                )
            }
            Notice::HarnessStuck {
                repo,
                fingerprints,
                night_id,
            } => format!(
                "[{repo}] {night_id}: the evaluation harness is stuck — the same environment \
                 failures ({}) recurred unchanged; the dry streak was not advanced.",
                fingerprints.join(", ")
            ),
            Notice::Accepted {
                repo,
                night_id,
                branch,
                pr_url,
            } => format!(
                "[{repo}] {night_id}: ACCEPT on `{branch}`{}",
                pr_url
                    .as_deref()
                    .map(|u| format!(" — {u}"))
                    .unwrap_or_default()
            ),
        }
    }
}

/// Delivers a [`Notice`] to a person. **Infallible by signature**: a failing
/// channel must never fail a night, so implementations swallow and log their
/// own errors.
pub trait Notifier: Send + Sync {
    /// Deliver `notice`, best-effort.
    fn notify<'a>(&'a self, notice: &'a Notice) -> BoxFut<'a, ()>;
}

/// Writes each notice to standard error.
#[derive(Debug, Clone, Copy, Default)]
pub struct StderrNotifier;

impl Notifier for StderrNotifier {
    fn notify<'a>(&'a self, notice: &'a Notice) -> BoxFut<'a, ()> {
        Box::pin(async move { eprintln!("{}", notice.render()) })
    }
}

// ---------------------------------------------------------------------------
// Forge
// ---------------------------------------------------------------------------

/// A draft pull request to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftPr {
    /// Local checkout holding the branch.
    pub repo_dir: PathBuf,
    /// `owner/name` slug on the forge.
    pub repo_slug: String,
    /// Branch to push.
    pub branch: String,
    /// PR title.
    pub title: String,
    /// PR body.
    pub body: String,
}

/// What publishing a branch achieved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrOutcome {
    /// The branch.
    pub branch: String,
    /// The PR URL, when one opened.
    pub pr_url: Option<String>,
    /// Whether the push succeeded.
    pub pushed: bool,
}

/// A code forge that can host a draft PR. Fail-open: a failure is reported in
/// the [`PrOutcome`], never raised, so the night still records its verdict.
pub trait Forge: Send + Sync {
    /// Push `pr.branch` and open a **draft** PR for it.
    fn publish_draft(&self, pr: &DraftPr) -> PrOutcome;

    /// Names of the status checks the protection rules of `branch` in
    /// `repo` require before a merge.
    ///
    /// The default is [`ForgeError::Unsupported`], which the automerge policy
    /// treats as a refusal: a forge that cannot say what protects a branch
    /// never gets automerge.
    fn required_status_checks(&self, repo: &str, branch: &str) -> Result<Vec<String>, ForgeError> {
        let _ = (repo, branch);
        Err(ForgeError::Unsupported)
    }

    /// Ask the forge to squash-merge the PR at `pr_url` once its required
    /// checks pass. Only called after the automerge policy said
    /// [`Allowed`](crate::automerge::AutomergeDecision::Allowed).
    ///
    /// The default is [`ForgeError::Unsupported`].
    fn enable_automerge(&self, repo: &str, pr_url: &str) -> Result<(), ForgeError> {
        let _ = (repo, pr_url);
        Err(ForgeError::Unsupported)
    }
}

/// Why a forge query or action did not succeed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForgeError {
    /// The forge has no such resource (for example a branch with no
    /// protection rules answers 404).
    #[error("not found")]
    NotFound,
    /// The call failed; the text says how.
    #[error("forge call failed: {0}")]
    Failed(String),
    /// This forge does not implement the operation.
    #[error("operation not supported by this forge")]
    Unsupported,
}

/// [`Forge`] backed by `git push` and the `gh` command-line tool. Shells out
/// only; no network library is linked.
#[derive(Debug, Clone, Copy, Default)]
pub struct GhForge;

impl Forge for GhForge {
    fn publish_draft(&self, pr: &DraftPr) -> PrOutcome {
        let pushed = crate::git::run(&pr.repo_dir, &["push", "-u", "origin", &pr.branch]).is_ok();
        let pr_url = if pushed {
            gh_pr_create(&pr.repo_dir, pr)
        } else {
            None
        };
        PrOutcome {
            branch: pr.branch.clone(),
            pr_url,
            pushed,
        }
    }

    fn required_status_checks(&self, repo: &str, branch: &str) -> Result<Vec<String>, ForgeError> {
        let body = Self::gh_api(&format!("repos/{repo}/branches/{branch}/protection"))?;
        crate::automerge::parse_protection(&body)
    }

    fn enable_automerge(&self, repo: &str, pr_url: &str) -> Result<(), ForgeError> {
        let out = Command::new("gh")
            .args(["pr", "merge", pr_url, "--repo", repo, "--auto", "--squash"])
            .output()
            .map_err(|e| ForgeError::Failed(format!("cannot run gh: {e}")))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(ForgeError::Failed(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    }
}

impl GhForge {
    fn gh_api(path: &str) -> Result<String, ForgeError> {
        let out = Command::new("gh")
            .args(["api", path])
            .output()
            .map_err(|e| ForgeError::Failed(format!("cannot run gh: {e}")))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("404") || err.contains("Not Found") {
            Err(ForgeError::NotFound)
        } else {
            Err(ForgeError::Failed(err.trim().to_string()))
        }
    }
}

fn gh_pr_create(dir: &Path, pr: &DraftPr) -> Option<String> {
    let out = Command::new("gh")
        .current_dir(dir)
        .args([
            "pr",
            "create",
            "--repo",
            &pr.repo_slug,
            "--head",
            &pr.branch,
            "--draft",
            "--title",
            &pr.title,
            "--body",
            &pr.body,
        ])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn transient_classification() {
        assert!(LlmError::Empty.is_transient());
        assert!(LlmError::Api("HTTP 524".into()).is_transient());
        assert!(!LlmError::Api("HTTP 400".into()).is_transient());
        assert!(!LlmError::MissingCredentials("k".into()).is_transient());
    }

    #[test]
    fn jsonl_sink_appends_one_line_per_record() {
        let d = tempfile::tempdir().unwrap();
        let sink = JsonlSink::new(d.path().join("a/b.jsonl"));
        let rec = MemoryRecord {
            key: "k".into(),
            kind: "night".into(),
            repo: "r".into(),
            night_id: "n".into(),
            verdict: "ACCEPT".into(),
            body: "line\nbreak".into(),
        };
        block_on(sink.record(&rec)).unwrap();
        block_on(sink.record(&rec)).unwrap();
        let text = std::fs::read_to_string(d.path().join("a/b.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2, "embedded newline must be escaped");
        let back: MemoryRecord = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn null_sink_and_stderr_notifier_never_fail() {
        let rec = MemoryRecord {
            key: "k".into(),
            kind: "x".into(),
            repo: "r".into(),
            night_id: "n".into(),
            verdict: "REJECT".into(),
            body: String::new(),
        };
        assert!(block_on(NullSink.record(&rec)).is_ok());
        block_on(StderrNotifier.notify(&Notice::HarnessStuck {
            repo: "r".into(),
            fingerprints: vec!["ab".into()],
            night_id: "n".into(),
        }));
    }

    #[test]
    fn lost_pr_text_names_the_branch_and_the_recovery_commands() {
        let t = Notice::LostPr {
            repo_dir: "work/r".into(),
            repo_slug: "o/r".into(),
            branch: "dream/x-2026-10-01".into(),
            pushed: false,
        }
        .render();
        assert!(t.contains("dream/x-2026-10-01") && t.contains("could not push"));
        assert!(t.contains("gh pr create --repo o/r --draft --head dream/x-2026-10-01"));
        let pushed = Notice::LostPr {
            repo_dir: "work/r".into(),
            repo_slug: "o/r".into(),
            branch: "b".into(),
            pushed: true,
        }
        .render();
        assert!(pushed.contains("could not open the draft pull request"));
    }

    #[test]
    fn traits_are_dyn_safe() {
        let _: Box<dyn MemorySink> = Box::new(NullSink);
        let _: Box<dyn Notifier> = Box::new(StderrNotifier);
        let _: Box<dyn Forge> = Box::new(GhForge);
    }
}
