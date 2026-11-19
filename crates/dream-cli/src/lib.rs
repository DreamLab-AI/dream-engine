//! The engine behind the `dream` command-line tool.
//!
//! Everything here is offline and pure of process state: each command takes
//! its inputs, reads the files it is pointed at and returns a [`Report`] with
//! the text for standard output, the text for standard error and an exit
//! code. `main` only prints it. That keeps the commands testable in-process
//! and leaves the binary a thin shell.
//!
//! # Exit codes
//!
//! * `0`: the check passed.
//! * `1`: the check ran and the answer is *no* (an invalid config, a ledger
//!   with bad rows, a witness that does not match, a recorded night that does
//!   not replay, a patch the guard refuses).
//! * `2`: the check could not run (a file that cannot be read, a malformed
//!   commit hash).
//!
//! `dream validate` is the one exception: an unreadable config exits `1`,
//! not `2`, since either way the engine could not use it.
//!
//! # Example
//!
//! ```
//! use dream_cli::{guard_check, GuardArgs};
//!
//! # let dir = std::env::temp_dir().join(format!("dream-cli-doc-{}", std::process::id()));
//! # std::fs::create_dir_all(&dir)?;
//! # let patch = dir.join("p.diff");
//! std::fs::write(
//!     &patch,
//!     "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b\n",
//! )?;
//! let r = guard_check(&GuardArgs { patch: patch.clone(), config: None });
//! assert_eq!(r.code, 0);
//! assert_eq!(r.stdout.trim(), "src/a.rs");
//! # std::fs::remove_dir_all(&dir)?;
//! # Ok::<(), std::io::Error>(())
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use dream_core::ledger::verify_ledger;
use dream_core::replay;
use dream_gate::config::DreamConfig;
use dream_gate::guard::{self, ProtectedPolicy};
use dream_gate::witness;

/// What a command produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Text for standard output.
    pub stdout: String,
    /// Text for standard error.
    pub stderr: String,
    /// Process exit code (see the [crate docs](crate)).
    pub code: u8,
}

impl Report {
    fn ok(stdout: impl Into<String>) -> Self {
        Report {
            stdout: stdout.into(),
            stderr: String::new(),
            code: 0,
        }
    }
    fn no(stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Report {
            stdout: stdout.into(),
            stderr: stderr.into(),
            code: 1,
        }
    }
    fn cannot(stderr: impl Into<String>) -> Self {
        Report {
            stdout: String::new(),
            stderr: stderr.into(),
            code: 2,
        }
    }
}

/// The `dream` command line.
#[derive(Debug, Parser)]
#[command(
    name = "dream",
    version,
    about = "Offline checks for the dream-engine gate"
)]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate a `dream.config.json` with the gate's own validation.
    Validate(ValidateArgs),
    /// Work with the markdown ledger.
    Ledger {
        /// The ledger action.
        #[command(subcommand)]
        action: LedgerAction,
    },
    /// Stamp or verify a witness binding a report to a commit.
    Witness {
        /// The witness action.
        #[command(subcommand)]
        action: WitnessAction,
    },
    /// Work with a recorded night.
    Replay {
        /// The replay action.
        #[command(subcommand)]
        action: ReplayAction,
    },
    /// Check a patch against the protected-path guard.
    Guard {
        /// The guard action.
        #[command(subcommand)]
        action: GuardAction,
    },
}

/// Arguments of `dream validate`.
#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// Config file to validate.
    #[arg(long, default_value = "dream.config.json")]
    pub config: PathBuf,
}

/// Ledger actions.
#[derive(Debug, Subcommand)]
pub enum LedgerAction {
    /// Structurally verify a ledger file.
    Verify {
        /// The ledger markdown file.
        path: PathBuf,
    },
}

/// Witness actions.
#[derive(Debug, Subcommand)]
pub enum WitnessAction {
    /// Print the witness binding a report to a commit.
    Stamp(StampArgs),
    /// Check a claimed witness against a report and commit.
    Verify(WitnessVerifyArgs),
}

/// Arguments of `dream witness stamp`.
#[derive(Debug, Args)]
pub struct StampArgs {
    /// The report file.
    #[arg(long)]
    pub report: PathBuf,
    /// The commit hash (7 to 64 hex characters).
    #[arg(long)]
    pub commit: String,
}

/// Arguments of `dream witness verify`.
#[derive(Debug, Args)]
pub struct WitnessVerifyArgs {
    /// The report file.
    #[arg(long)]
    pub report: PathBuf,
    /// The commit hash (7 to 64 hex characters).
    #[arg(long)]
    pub commit: String,
    /// The claimed witness (64 hex characters).
    #[arg(long)]
    pub witness: String,
}

/// Replay actions.
#[derive(Debug, Subcommand)]
pub enum ReplayAction {
    /// Re-judge a recorded night and compare with the recorded decision.
    ///
    /// This shows that the stored inputs and decision agree. It is not tamper
    /// evidence on its own: bind the run directory with a witness for that.
    Verify {
        /// The run directory written by `dream_core::replay::capture`.
        run_dir: PathBuf,
    },
}

/// Guard actions.
#[derive(Debug, Subcommand)]
pub enum GuardAction {
    /// Print the paths a patch touches, or the violation and exit 1.
    Check(GuardArgs),
}

/// Arguments of `dream guard check`.
#[derive(Debug, Args)]
pub struct GuardArgs {
    /// The unified diff to check.
    #[arg(long)]
    pub patch: PathBuf,
    /// A config whose protected globs, ledger path and evaluator commands
    /// extend the built-in protected set.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Run a parsed command line.
pub fn run(cli: &Cli) -> Report {
    match &cli.command {
        Command::Validate(a) => validate(a),
        Command::Ledger {
            action: LedgerAction::Verify { path },
        } => ledger_verify(path),
        Command::Witness {
            action: WitnessAction::Stamp(a),
        } => witness_stamp(a),
        Command::Witness {
            action: WitnessAction::Verify(a),
        } => witness_verify(a),
        Command::Replay {
            action: ReplayAction::Verify { run_dir },
        } => replay_verify(run_dir),
        Command::Guard {
            action: GuardAction::Check(a),
        } => guard_check(a),
    }
}

fn read(path: &Path) -> Result<String, Report> {
    std::fs::read_to_string(path).map_err(|e| Report::cannot(format!("{}: {e}\n", path.display())))
}

/// `dream validate`: an unreadable or invalid config is reported as invalid
/// (exit 1), since either way the engine could not use it.
pub fn validate(a: &ValidateArgs) -> Report {
    match DreamConfig::load(&a.config) {
        Ok(cfg) => Report::ok(format!(
            "ok: {} ({} slot(s), {} evaluator(s), autoMerge {})\n",
            cfg.repo,
            cfg.slots.len(),
            cfg.evaluator_entrypoints.len(),
            if cfg.auto_merge { "on" } else { "off" }
        )),
        Err(e) => Report::no("", format!("{}: {e}\n", a.config.display())),
    }
}

/// `dream ledger verify`.
pub fn ledger_verify(path: &Path) -> Report {
    let md = match read(path) {
        Ok(m) => m,
        Err(r) => return r,
    };
    let v = verify_ledger(&md);
    let mut text = String::new();
    for w in &v.warnings {
        text.push_str(&format!("warning: {w}\n"));
    }
    if v.ok {
        text.push_str(&format!("ok: {} row(s)\n", v.row_count));
        Report::ok(text)
    } else {
        for e in &v.errors {
            text.push_str(&format!("{e}\n"));
        }
        Report::no(text, format!("{}: ledger is not valid\n", path.display()))
    }
}

/// `dream witness stamp`.
pub fn witness_stamp(a: &StampArgs) -> Report {
    let report = match read(&a.report) {
        Ok(r) => r,
        Err(r) => return r,
    };
    match witness::witness(&report, &a.commit) {
        Ok(w) => Report::ok(format!("{w}\n")),
        Err(e) => Report::cannot(format!("{e}\n")),
    }
}

/// `dream witness verify`.
pub fn witness_verify(a: &WitnessVerifyArgs) -> Report {
    let report = match read(&a.report) {
        Ok(r) => r,
        Err(r) => return r,
    };
    match witness::verify(&report, &a.commit, &a.witness) {
        Ok(true) => Report::ok("ok: witness matches\n"),
        Ok(false) => Report::no("", "witness does not match the report and commit\n"),
        Err(e) => Report::cannot(format!("{e}\n")),
    }
}

/// `dream replay verify`.
pub fn replay_verify(run_dir: &Path) -> Report {
    match replay::verify(run_dir) {
        Err(e) => Report::cannot(format!("{}: {e}\n", run_dir.display())),
        Ok(v) if v.matches => Report::ok(format!(
            "ok: recorded verdict {} reproduced\n",
            v.recorded.verdict
        )),
        Ok(v) => {
            let mut text = String::new();
            for d in &v.diffs {
                text.push_str(&format!("{d}\n"));
            }
            if v.guard_agrees == Some(false) {
                text.push_str(
                    "guard: re-checking the stored patch disagrees with the recorded state\n",
                );
            }
            Report::no(text, "recorded night does not replay\n")
        }
    }
}

/// `dream guard check`.
pub fn guard_check(a: &GuardArgs) -> Report {
    let patch = match read(&a.patch) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let policy = match &a.config {
        None => ProtectedPolicy::default(),
        Some(path) => match DreamConfig::load(path) {
            Ok(cfg) => ProtectedPolicy::from_config(&cfg),
            Err(e) => return Report::cannot(format!("{}: {e}\n", path.display())),
        },
    };
    match guard::check(&patch, &policy) {
        Ok(paths) => {
            let mut text = String::new();
            for p in paths.iter() {
                text.push_str(p);
                text.push('\n');
            }
            Report::ok(text)
        }
        Err(v) => Report::no("", format!("{v}\n")),
    }
}
