//! Minimal `git` subprocess helper shared by [`crate::persist`],
//! [`crate::source`] and [`crate::candidate`].

use std::path::Path;
use std::process::Command;

/// A failed `git` invocation.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// `git` could not be spawned.
    #[error("spawning git: {0}")]
    Spawn(#[from] std::io::Error),
    /// `git` ran and exited non-zero. Carries the first argument (the
    /// subcommand) and trimmed stderr.
    #[error("git {0} failed: {1}")]
    Failed(String, String),
}

/// Run `git -C <dir> <args…>` and return its stdout verbatim.
pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(GitError::Failed(
            args.first().copied().unwrap_or("").to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// [`run`] with stdout trimmed.
pub(crate) fn run_trimmed(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    run(dir, args).map(|s| s.trim().to_string())
}
