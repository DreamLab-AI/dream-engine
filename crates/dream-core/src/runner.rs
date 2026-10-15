//! How an evaluator command is actually executed.
//!
//! The acceptance path consumes [`ExecOutcome`] and never knows which runner
//! produced it. This crate ships two: [`LocalRunner`] (a real `bash` in a real
//! directory) and [`ScriptedRunner`] (replays fixed outcomes, for tests and
//! dry runs). A host that wants to run evaluators somewhere else (a remote
//! machine, a container) implements [`EvaluatorRunner`] itself; the trait is
//! deliberately synchronous and dyn-safe.
//!
//! # Example
//!
//! ```
//! use dream_core::runner::{EvaluatorRunner, ScriptedRunner};
//!
//! let r = ScriptedRunner::new().passing("cargo test", "ok");
//! assert_eq!(r.run("/anywhere", "cargo test", 60).exit_code, Some(0));
//! // An unscripted command is a harness fault, never an accidental pass.
//! assert!(r.run("/anywhere", "cargo clippy", 60).transport_error.is_some());
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use dream_gate::outcome::shell_quote;
pub use dream_gate::outcome::ExecOutcome;

/// Runs one evaluator command in a working directory under a wall-clock budget.
pub trait EvaluatorRunner: Send + Sync {
    /// Execute `command` with `work_dir` as the current directory, killing it
    /// at `timeout_secs`.
    fn run(&self, work_dir: &str, command: &str, timeout_secs: u64) -> ExecOutcome;
    /// Short description for logs and receipt provenance.
    fn describe(&self) -> String;
}

/// Runs the command locally with `bash -c` in `work_dir`.
///
/// The command is wrapped in GNU `timeout` and `bash -o pipefail`. `pipefail`
/// is load-bearing: evaluators routinely tail their output
/// (`cargo build 2>&1 | tail -12`), and without it the pipeline's status is
/// `tail`'s `0`, so a failing build would be recorded as a pass — a vacuous
/// required gate.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalRunner;

impl EvaluatorRunner for LocalRunner {
    fn run(&self, work_dir: &str, command: &str, timeout_secs: u64) -> ExecOutcome {
        let started_at = chrono::Utc::now().to_rfc3339();
        let t0 = Instant::now();
        let wrapped = format!(
            "timeout --signal=TERM --kill-after=30s {}s bash -o pipefail -c {}",
            timeout_secs,
            shell_quote(command)
        );
        let out = Command::new("bash")
            .arg("-c")
            .arg(&wrapped)
            .current_dir(PathBuf::from(work_dir))
            .output();
        match out {
            Ok(o) => ExecOutcome {
                exit_code: o.status.code(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
                duration_ms: t0.elapsed().as_millis(),
                transport_error: None,
                timed_out: false,
                started_at,
            },
            Err(e) => ExecOutcome::blocked(format!("spawning bash in {work_dir}: {e}")),
        }
    }

    fn describe(&self) -> String {
        "local".into()
    }
}

/// A runner that replays pre-set outcomes, keyed by the exact command string.
/// An unknown command is [`ExecOutcome::blocked`] — a test that forgets to
/// script an evaluator sees a harness fault, never an accidental pass.
#[derive(Debug, Clone, Default)]
pub struct ScriptedRunner {
    outcomes: HashMap<String, ExecOutcome>,
}

impl ScriptedRunner {
    /// An empty script.
    pub fn new() -> Self {
        Self::default()
    }

    /// Script an exact outcome for `command`.
    #[must_use]
    pub fn with(mut self, command: &str, outcome: ExecOutcome) -> Self {
        self.outcomes.insert(command.to_string(), outcome);
        self
    }

    /// Convenience: a clean pass printing `stdout`.
    #[must_use]
    pub fn passing(self, command: &str, stdout: &str) -> Self {
        self.with(
            command,
            ExecOutcome {
                exit_code: Some(0),
                stdout: stdout.into(),
                stderr: String::new(),
                duration_ms: 5,
                transport_error: None,
                timed_out: false,
                started_at: "2026-09-05T01:00:00Z".into(),
            },
        )
    }

    /// Convenience: a non-zero exit with `stderr`.
    #[must_use]
    pub fn failing(self, command: &str, code: i32, stderr: &str) -> Self {
        self.with(
            command,
            ExecOutcome {
                exit_code: Some(code),
                stdout: String::new(),
                stderr: stderr.into(),
                duration_ms: 5,
                transport_error: None,
                timed_out: false,
                started_at: "2026-09-05T01:00:00Z".into(),
            },
        )
    }
}

impl EvaluatorRunner for ScriptedRunner {
    fn run(&self, _work_dir: &str, command: &str, _timeout_secs: u64) -> ExecOutcome {
        self.outcomes
            .get(command)
            .cloned()
            .unwrap_or_else(|| ExecOutcome::blocked(format!("no scripted outcome for {command:?}")))
    }

    fn describe(&self) -> String {
        "scripted".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failing producer piped through `tail` must surface its own exit code,
    /// not tail's 0.
    #[test]
    fn local_runner_does_not_let_a_tail_pipe_mask_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let out = LocalRunner.run(
            dir.path().to_str().unwrap(),
            "sh -c 'echo error: manifest missing; exit 101' 2>&1 | tail -3",
            30,
        );
        assert_eq!(
            out.exit_code,
            Some(101),
            "pipefail must propagate the producer's exit: {out:?}"
        );
        assert!(out.stdout.contains("manifest missing"));
    }

    #[test]
    fn local_runner_captures_streams_and_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let o = LocalRunner.run(
            dir.path().to_str().unwrap(),
            "echo out; echo err >&2; exit 3",
            30,
        );
        assert_eq!(o.exit_code, Some(3));
        assert_eq!(o.stdout.trim(), "out");
        assert_eq!(o.stderr.trim(), "err");
        assert!(o.transport_error.is_none());
    }

    #[test]
    fn local_runner_runs_in_the_given_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "hi").unwrap();
        let o = LocalRunner.run(dir.path().to_str().unwrap(), "cat marker.txt", 30);
        assert_eq!(o.stdout, "hi");
    }

    #[test]
    fn local_runner_enforces_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let o = LocalRunner.run(dir.path().to_str().unwrap(), "sleep 5", 1);
        assert_eq!(o.exit_code, Some(124), "GNU timeout signals expiry as 124");
    }

    #[test]
    fn scripted_runner_blocks_on_unscripted_commands() {
        let r = ScriptedRunner::new().passing("cargo test", "ok");
        assert_eq!(r.run("/x", "cargo test", 60).exit_code, Some(0));
        let unknown = r.run("/x", "cargo clippy", 60);
        assert!(
            unknown.transport_error.is_some(),
            "an unscripted command must not pass"
        );
    }

    #[test]
    fn runner_is_dyn_safe() {
        let r: Box<dyn EvaluatorRunner> = Box::new(ScriptedRunner::new());
        assert_eq!(r.describe(), "scripted");
    }
}
