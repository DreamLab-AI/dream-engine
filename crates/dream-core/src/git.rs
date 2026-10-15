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

/// Run `git -C <dir> <args…>` with `input` on its standard input.
///
/// The input is written from a separate thread so a child that fills its
/// output pipe before reading everything cannot deadlock the caller.
pub(crate) fn run_stdin(dir: &Path, args: &[&str], input: &[u8]) -> Result<String, GitError> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let bytes = input.to_vec();
    let writer = std::thread::spawn(move || {
        // A broken pipe just means git stopped reading; its exit status tells.
        let _ = stdin.write_all(&bytes);
    });
    let out = child.wait_with_output()?;
    let _ = writer.join();
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(GitError::Failed(
            args.first().copied().unwrap_or("").to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
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
