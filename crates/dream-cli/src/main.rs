//! The `dream` binary: parses the command line, runs it, prints the result.

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use dream_cli::{run, Cli};

fn main() -> ExitCode {
    let report = run(&Cli::parse());
    let _ = std::io::stdout().write_all(report.stdout.as_bytes());
    let _ = std::io::stderr().write_all(report.stderr.as_bytes());
    ExitCode::from(report.code)
}
