//! The raw result of executing one evaluator command.
//!
//! [`ExecOutcome`] is the seam between whatever *runs* an evaluator (SSH, a
//! container, a local shell, a scripted replay) and everything that *judges*
//! it. This crate never executes anything; it only consumes this value.

/// A raw execution: both streams, the exit status, how long it took, and
/// whether the transport itself failed (which is not the same as the command
/// failing).
///
/// ```
/// use dream_gate::outcome::ExecOutcome;
/// let o = ExecOutcome::blocked("runner unreachable");
/// assert!(o.exit_code.is_none());
/// assert_eq!(o.transport_error.as_deref(), Some("runner unreachable"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    /// Process exit status; `None` when the process produced none.
    pub exit_code: Option<i32>,
    /// Captured standard output, verbatim.
    pub stdout: String,
    /// Captured standard error, verbatim.
    pub stderr: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u128,
    /// Set when the command could not be delivered or started at all.
    pub transport_error: Option<String>,
    /// True when the runner killed the command at its time budget.
    pub timed_out: bool,
    /// RFC 3339 start time, supplied by the runner.
    pub started_at: String,
}

impl ExecOutcome {
    /// An outcome for a command that never started: a transport fault.
    pub fn blocked(detail: impl Into<String>) -> Self {
        Self {
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 0,
            transport_error: Some(detail.into()),
            timed_out: false,
            started_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Single-quote a string for safe embedding in a POSIX shell command line.
///
/// ```
/// use dream_gate::outcome::shell_quote;
/// assert_eq!(shell_quote("it's"), r"'it'\''s'");
/// ```
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_embedded_single_quotes() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn blocked_carries_the_detail_and_no_exit_status() {
        let o = ExecOutcome::blocked("x");
        assert_eq!(o.exit_code, None);
        assert!(!o.timed_out);
    }
}
