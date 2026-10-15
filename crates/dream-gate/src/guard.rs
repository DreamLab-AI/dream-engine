//! Protected-path guard: refuse any patch that could touch the machinery that
//! judges it.
//!
//! A self-improving loop that may edit its own gate, CI, thresholds, held-out
//! tests or evaluator scripts has no gate at all. [`check`] parses a unified
//! git diff **strictly** and returns the normalised set of repo-relative paths
//! it touches, or a [`GuardViolation`] refusing it. The function is pure and
//! deterministic.
//!
//! # Fail closed
//!
//! Any parse failure is a refusal. Every path source is collected — both sides
//! of `diff --git`, `---`/`+++` headers, `rename`/`copy` `from`/`to` — and a
//! false refusal is always preferred to a missed path. Symlinks (mode
//! `120000`), gitlinks, unusual modes and binary patches are refused outright.
//! Paths are normalised (`.` and `..` resolved, repeated separators collapsed)
//! and matched case-insensitively, so `./gate.rs`, `a/../gate.rs`, and
//! `crates/x/../../.github/y.yml` cannot sidestep a rule. Absolute paths and
//! paths escaping the repository are refused.
//!
//! # Duty of the caller
//!
//! **Call [`check`] before applying a patch.** Applying first and checking
//! afterwards lets a hostile diff run its effects (and any later rollback may
//! be incomplete). [`crate::gate::decide`] re-derives the touched paths from
//! the patch itself as defence in depth, but that is a second line, not a
//! substitute.
//!
//! # Protected by default
//!
//! `.github/**`, `.git/**`, `.gitmodules`, `.cargo/**`; `CODEOWNERS` and
//! `SECURITY.md` anywhere; every path component *starting* with `gate` or
//! `safety`, or *containing* `threshold`, `promotion` or `scorer` (this
//! over-matches, e.g. `gateway.rs`, deliberately); manifests and lockfiles
//! (`Cargo.toml`, `Cargo.lock`, `package.json`, `package-lock.json`,
//! `npm-shrinkwrap.json`, `yarn.lock`, `pnpm-lock.yaml`, `deny.toml`,
//! `rust-toolchain*`); `dream.config.json`; `tests/heldout/**`; any component
//! named `receipts`, `receipt`, `witness` or `witnesses`. A
//! [`ProtectedPolicy`] adds the ledger path, evaluator-command paths, the
//! config's own `protected` globs (additive only) and, optionally, the gate
//! crates themselves.
//!
//! ```
//! use dream_gate::guard::{check, GuardViolation, ProtectedPolicy};
//! let policy = ProtectedPolicy::default().with_globs(["docs/frozen/**"]);
//! let patch = "diff --git a/docs/frozen/a.md b/docs/frozen/a.md\n--- a/docs/frozen/a.md\n+++ b/docs/frozen/a.md\n@@ -1 +1 @@\n-x\n+y\n";
//! assert!(matches!(check(patch, &policy), Err(GuardViolation::Protected { .. })));
//! ```

use serde::Serialize;
use std::collections::BTreeSet;
use thiserror::Error;

use crate::config::DreamConfig;
use crate::manifest::{digest, ExperimentManifest};

/// Longest path (bytes) the guard will entertain.
const MAX_PATH_BYTES: usize = 4096;

/// The normalised, repo-relative paths a patch touches, sorted and unique.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TouchedPaths(BTreeSet<String>);

impl TouchedPaths {
    /// Whether `path` (already normalised) is among the touched paths.
    pub fn contains(&self, path: &str) -> bool {
        self.0.contains(path)
    }
    /// Number of distinct paths.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// True when no path is touched.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Iterate paths in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
    /// The paths as an owned, sorted vector.
    pub fn to_vec(&self) -> Vec<String> {
        self.0.iter().cloned().collect()
    }
}

/// Why a patch was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GuardViolation {
    /// The patch is empty or whitespace.
    #[error("patch is empty")]
    Empty,
    /// The patch could not be parsed with certainty. Always a refusal.
    #[error("malformed patch (line {line}): {detail}")]
    Malformed {
        /// 1-based line number, or 0 for a structural problem.
        line: usize,
        /// What was wrong.
        detail: String,
    },
    /// A path is absolute.
    #[error("absolute path refused: {path:?}")]
    Absolute {
        /// The offending path as written.
        path: String,
    },
    /// A path climbs out of the repository.
    #[error("path escapes the repository: {path:?}")]
    Escapes {
        /// The offending path as written.
        path: String,
    },
    /// The patch creates, edits or deletes a symbolic link.
    #[error("symlink refused: {path:?}")]
    Symlink {
        /// The affected path.
        path: String,
    },
    /// A file mode other than `100644` or `100755` (for example a gitlink).
    #[error("unsupported file mode {mode} on {path:?}")]
    UnsupportedMode {
        /// The affected path.
        path: String,
        /// The mode as written.
        mode: String,
    },
    /// The patch carries binary content.
    #[error("binary patch refused")]
    Binary,
    /// One or more touched paths are protected; the whole patch is refused.
    #[error("protected paths touched: {paths:?}")]
    Protected {
        /// The protected paths, normalised and sorted.
        paths: Vec<String>,
    },
}

/// Why a [`ProtectedPolicy`] could not be built from a config.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    /// The config bytes do not hash to the manifest's frozen `config_digest`,
    /// so they are not the configuration the experiment was frozen against.
    #[error("config digest {actual} does not match the frozen manifest digest {frozen}")]
    ConfigDigestMismatch {
        /// Digest recorded in the manifest.
        frozen: String,
        /// Digest of the bytes supplied.
        actual: String,
    },
}

/// The set of protected paths.
///
/// [`ProtectedPolicy::default`] carries only the built-in rules (see the
/// [module docs](self)); builders add to it. Nothing can ever be removed.
#[derive(Debug, Clone, Default)]
pub struct ProtectedPolicy {
    /// Config globs, lowercased and split into segments.
    globs: Vec<Vec<String>>,
    /// Exact-or-ancestor entries (ledger, command paths, gate crates),
    /// lowercased component vectors.
    prefixes: Vec<Vec<String>>,
}

impl ProtectedPolicy {
    /// Add glob patterns (`*`, `?` within a segment; `**` for any number of
    /// segments). A pattern without `/` matches at any depth; a pattern that
    /// matches a directory protects everything beneath it. Matching is
    /// case-insensitive.
    #[must_use]
    pub fn with_globs<I, S>(mut self, globs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for g in globs {
            let g: String = g.into();
            let g = g.trim().trim_start_matches("./").trim_end_matches('/');
            if g.is_empty() {
                continue;
            }
            let lower = g.to_lowercase();
            let mut segs: Vec<String> = lower.split('/').map(str::to_string).collect();
            if segs.len() == 1 && segs[0] != "**" {
                segs.insert(0, "**".to_string());
            }
            self.globs.push(segs);
        }
        self
    }

    /// Protect the ledger file (a no-op for an empty path).
    #[must_use]
    pub fn with_ledger_path(self, ledger_path: &str) -> Self {
        self.with_prefix_paths([ledger_path.to_string()])
    }

    /// Protect every repo-relative path named inside each frozen evaluator
    /// command (see [`evaluator_command_paths`]). A named directory protects
    /// its whole tree.
    #[must_use]
    pub fn with_evaluator_commands<I, S>(self, commands: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let paths: Vec<String> = commands
            .into_iter()
            .flat_map(|c| evaluator_command_paths(c.as_ref()))
            .collect();
        self.with_prefix_paths(paths)
    }

    /// When `on`, also protect the gate crates themselves
    /// (`crates/dream-gate/**`, `crates/pairwise-gate/**`,
    /// `crates/failure-fingerprint/**`).
    #[must_use]
    pub fn with_self_referential(self, on: bool) -> Self {
        if !on {
            return self;
        }
        self.with_prefix_paths([
            "crates/dream-gate".to_string(),
            "crates/pairwise-gate".to_string(),
            "crates/failure-fingerprint".to_string(),
        ])
    }

    fn with_prefix_paths<I: IntoIterator<Item = String>>(mut self, paths: I) -> Self {
        for p in paths {
            if let Ok(norm) = normalise_rel(p.trim()) {
                self.prefixes
                    .push(norm.to_lowercase().split('/').map(str::to_string).collect());
            }
        }
        self
    }

    /// Build the policy a configuration implies: its `protected` globs, its
    /// ledger path, and every path named in its evaluator commands.
    #[must_use]
    pub fn from_config(cfg: &DreamConfig) -> Self {
        let mut cmds: Vec<&str> = cfg
            .evaluator_entrypoints
            .values()
            .map(|s| s.cmd.as_str())
            .collect();
        cmds.sort_unstable();
        Self::default()
            .with_globs(cfg.protected.iter().cloned())
            .with_ledger_path(&cfg.ledger_path)
            .with_evaluator_commands(cmds)
    }

    /// As [`from_config`](Self::from_config), but only if `config_bytes` hash
    /// to the manifest's frozen `config_digest`.
    ///
    /// A policy must be derived from the **baseline** configuration the
    /// experiment was frozen against — never from a candidate's. This is the
    /// coupling between manifest and config: the manifest holds the digest, the
    /// config holds the globs, and this constructor refuses to join a policy to
    /// the wrong document.
    pub fn for_manifest(
        manifest: &ExperimentManifest,
        cfg: &DreamConfig,
        config_bytes: &[u8],
    ) -> Result<Self, PolicyError> {
        let actual = digest(config_bytes);
        if actual != manifest.config_digest {
            return Err(PolicyError::ConfigDigestMismatch {
                frozen: manifest.config_digest.clone(),
                actual,
            });
        }
        Ok(Self::from_config(cfg))
    }

    /// Whether a **normalised** repo-relative path is protected. Prefer
    /// [`check`]; this exists for callers that already hold clean paths.
    pub fn is_protected(&self, path: &str) -> bool {
        let lower = path.to_lowercase();
        let comps: Vec<&str> = lower.split('/').filter(|c| !c.is_empty()).collect();
        if comps.is_empty() {
            return false;
        }
        if builtin_protected(&comps) {
            return true;
        }
        if self
            .prefixes
            .iter()
            .any(|p| comps.len() >= p.len() && p.iter().zip(&comps).all(|(a, b)| a.as_str() == *b))
        {
            return true;
        }
        // A glob that matches the path or any ancestor directory protects it.
        self.globs.iter().any(|g| {
            let g: Vec<&str> = g.iter().map(String::as_str).collect();
            (1..=comps.len()).any(|k| glob_path(&g, &comps[..k]))
        })
    }
}

fn builtin_protected(comps: &[&str]) -> bool {
    const BASENAMES: &[&str] = &[
        "codeowners",
        "security.md",
        "cargo.toml",
        "cargo.lock",
        "package.json",
        "package-lock.json",
        "npm-shrinkwrap.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "deny.toml",
        "dream.config.json",
        ".gitmodules",
    ];
    const DIRS: &[&str] = &["receipts", "receipt", "witness", "witnesses", ".git"];
    if matches!(comps[0], ".github" | ".cargo") {
        return true;
    }
    if comps.windows(2).any(|w| w == ["tests", "heldout"]) {
        return true;
    }
    let base = comps[comps.len() - 1];
    if BASENAMES.contains(&base) || base.starts_with("rust-toolchain") {
        return true;
    }
    comps.iter().any(|c| {
        DIRS.contains(c)
            || c.starts_with("gate")
            || c.starts_with("safety")
            || c.contains("threshold")
            || c.contains("promotion")
            || c.contains("scorer")
    })
}

/// Match glob segments `pat` against path components `path`; `**` consumes any
/// number (including zero) of components.
fn glob_path(pat: &[&str], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|k| glob_path(rest, &path[k..])),
        Some((p, rest)) => match path.split_first() {
            Some((c, tail)) => glob_segment(p, c) && glob_path(rest, tail),
            None => false,
        },
    }
}

/// Match one segment: `*` any run, `?` any single char.
fn glob_segment(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Repo-relative paths named inside a shell command, for freezing into the
/// protected set.
///
/// The command is split on whitespace and shell metacharacters; a token counts
/// as a path when it contains `/` or looks like `name.ext`. Options, URLs,
/// absolute paths, `~` paths, plain numbers and anything escaping the repo are
/// ignored; `--flag=value` contributes `value`. Paths are normalised and
/// returned in first-seen order without duplicates.
///
/// This cannot see files an evaluator reads *indirectly* (a `Makefile` behind
/// `make test`, a `conftest.py` behind `pytest`); protect those explicitly
/// with the config's `protected` globs.
///
/// ```
/// use dream_gate::guard::evaluator_command_paths;
/// assert_eq!(
///     evaluator_command_paths("bash scripts/check.sh --cfg=conf/a.json | tail -3"),
///     vec!["scripts/check.sh".to_string(), "conf/a.json".to_string()]
/// );
/// ```
pub fn evaluator_command_paths(cmd: &str) -> Vec<String> {
    let is_delim = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ';' | '|' | '&' | '<' | '>' | '(' | ')' | '"' | '\'' | '`' | '=' | '{' | '}' | ','
            )
    };
    let mut out: Vec<String> = Vec::new();
    for tok in cmd.split(is_delim).filter(|t| !t.is_empty()) {
        if tok.starts_with('-')
            || tok.starts_with('/')
            || tok.starts_with('~')
            || tok.starts_with('$')
        {
            continue;
        }
        if tok.contains("://") || tok.chars().all(|c| c.is_ascii_digit() || c == '.') {
            continue;
        }
        let looks_like_file = tok.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && !ext.is_empty()
                && ext.len() <= 6
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && stem
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        });
        if !tok.contains('/') && !looks_like_file {
            continue;
        }
        if let Ok(norm) = normalise_rel(tok) {
            if !out.contains(&norm) {
                out.push(norm);
            }
        }
    }
    out
}

/// Normalise a repo-relative path: resolve `.`/`..`, collapse separators,
/// refuse absolute paths, escapes, control characters and backslashes.
fn normalise_rel(raw: &str) -> Result<String, GuardViolation> {
    let malformed = |d: &str| GuardViolation::Malformed {
        line: 0,
        detail: format!("{d}: {raw:?}"),
    };
    if raw.is_empty() {
        return Err(malformed("empty path"));
    }
    if raw.len() > MAX_PATH_BYTES {
        return Err(malformed("path too long"));
    }
    if raw.chars().any(|c| c.is_control() || c == '\\') {
        return Err(malformed("control character or backslash in path"));
    }
    let b = raw.as_bytes();
    if raw.starts_with('/') || (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()) {
        return Err(GuardViolation::Absolute { path: raw.into() });
    }
    let mut stack: Vec<&str> = Vec::new();
    for seg in raw.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.pop().is_none() {
                    return Err(GuardViolation::Escapes { path: raw.into() });
                }
            }
            s => stack.push(s),
        }
    }
    if stack.is_empty() {
        return Err(malformed("path names no file"));
    }
    Ok(stack.join("/"))
}

/// Decode a git C-style quoted string starting at `s[0] == '"'`. Returns the
/// text and the byte length consumed (including both quotes).
fn parse_quoted(s: &str) -> Result<(String, usize), String> {
    let b = s.as_bytes();
    debug_assert_eq!(b.first(), Some(&b'"'));
    let mut out: Vec<u8> = Vec::new();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let text = String::from_utf8(out).map_err(|_| "quoted path is not UTF-8")?;
                return Ok((text, i + 1));
            }
            b'\\' => {
                i += 1;
                let c = *b.get(i).ok_or("dangling backslash in quoted path")?;
                match c {
                    b'a' => out.push(7),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'v' => out.push(11),
                    b'\\' => out.push(b'\\'),
                    b'"' => out.push(b'"'),
                    b'0'..=b'3' => {
                        let d = b.get(i..i + 3).ok_or("short octal escape")?;
                        if !d.iter().all(|x| (b'0'..=b'7').contains(x)) {
                            return Err("bad octal escape in quoted path".into());
                        }
                        out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                        i += 2;
                    }
                    _ => return Err(format!("unknown escape \\{} in quoted path", c as char)),
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Err("unterminated quoted path".into())
}

/// One file section of the diff while it is being parsed.
#[derive(Default)]
struct Section {
    start_line: usize,
    /// Raw (prefix-stripped, decoded, not yet normalised) paths.
    paths: Vec<String>,
    old_header: bool,
    new_header: bool,
    rename_from: bool,
    rename_to: bool,
    copy_from: bool,
    copy_to: bool,
    modes: Vec<String>,
    has_effect: bool,
}

fn malformed(line: usize, detail: impl Into<String>) -> GuardViolation {
    GuardViolation::Malformed {
        line,
        detail: detail.into(),
    }
}

/// Strip the `a/`/`b/` prefix demanded of every header path.
fn strip_ab(p: &str, line: usize) -> Result<String, GuardViolation> {
    p.strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .map(str::to_string)
        .ok_or_else(|| malformed(line, format!("header path lacks an a/ or b/ prefix: {p:?}")))
}

/// Parse a `---`/`+++` header value: `None` for `/dev/null`.
fn header_path(rest: &str, line: usize) -> Result<Option<String>, GuardViolation> {
    let rest = rest.trim_end_matches(' ');
    let value = if rest.starts_with('"') {
        let (text, used) = parse_quoted(rest).map_err(|e| malformed(line, e))?;
        let tail = &rest[used..];
        if !tail.is_empty() && !tail.starts_with('\t') {
            return Err(malformed(line, "trailing text after quoted header path"));
        }
        text
    } else {
        rest.split('\t')
            .next()
            .unwrap_or("")
            .trim_end_matches(' ')
            .to_string()
    };
    if value == "/dev/null" {
        return Ok(None);
    }
    strip_ab(&value, line).map(Some)
}

/// Parse a `rename`/`copy` path (no `a/`/`b/` prefix).
fn bare_path(rest: &str, line: usize) -> Result<String, GuardViolation> {
    if rest.starts_with('"') {
        let (text, used) = parse_quoted(rest).map_err(|e| malformed(line, e))?;
        if used != rest.len() {
            return Err(malformed(line, "trailing text after quoted path"));
        }
        Ok(text)
    } else {
        Ok(rest.to_string())
    }
}

/// Resolve the two paths of a `diff --git` line; `Ok(vec![])` when unquoted
/// names make the split ambiguous (other headers must then supply the paths).
fn git_line_paths(rest: &str, line: usize) -> Result<Vec<String>, GuardViolation> {
    if rest.starts_with('"') {
        let (left, used) = parse_quoted(rest).map_err(|e| malformed(line, e))?;
        let tail = rest[used..]
            .strip_prefix(' ')
            .ok_or_else(|| malformed(line, "diff --git line lacks a second path"))?;
        let right = if tail.starts_with('"') {
            let (r, u) = parse_quoted(tail).map_err(|e| malformed(line, e))?;
            if u != tail.len() {
                return Err(malformed(line, "trailing text after second quoted path"));
            }
            r
        } else {
            tail.to_string()
        };
        return Ok(vec![strip_ab(&left, line)?, strip_ab(&right, line)?]);
    }
    // Unquoted: try every space as the separator.
    let mut candidates: Vec<(String, String)> = Vec::new();
    for (i, _) in rest.match_indices(' ') {
        let (l, r) = (&rest[..i], &rest[i + 1..]);
        if let (Some(l), Some(r)) = (l.strip_prefix("a/"), r.strip_prefix("b/")) {
            candidates.push((l.to_string(), r.to_string()));
        }
    }
    if let Some((l, r)) = candidates.iter().find(|(l, r)| l == r) {
        return Ok(vec![l.clone(), r.clone()]);
    }
    if candidates.len() == 1 {
        let (l, r) = candidates.remove(0);
        return Ok(vec![l, r]);
    }
    Ok(Vec::new())
}

fn finish(sec: Section, touched: &mut BTreeSet<String>) -> Result<(), GuardViolation> {
    let at = sec.start_line;
    if sec.rename_from != sec.rename_to {
        return Err(malformed(at, "rename without both from and to"));
    }
    if sec.copy_from != sec.copy_to {
        return Err(malformed(at, "copy without both from and to"));
    }
    if sec.old_header != sec.new_header {
        return Err(malformed(at, "unpaired --- / +++ header"));
    }
    if sec.paths.is_empty() {
        return Err(malformed(at, "file section names no path"));
    }
    if !sec.has_effect {
        return Err(malformed(at, "file section carries no change (truncated?)"));
    }
    let mut normalised = Vec::with_capacity(sec.paths.len());
    for p in &sec.paths {
        normalised.push(normalise_rel(p)?);
    }
    for mode in &sec.modes {
        let path = normalised[0].clone();
        match mode.as_str() {
            "100644" | "100755" => {}
            "120000" => return Err(GuardViolation::Symlink { path }),
            other => {
                return Err(GuardViolation::UnsupportedMode {
                    path,
                    mode: other.to_string(),
                })
            }
        }
    }
    touched.extend(normalised);
    Ok(())
}

/// Parse a unified git diff into the set of paths it touches, applying no
/// protection policy. Fails closed exactly as [`check`] does for everything but
/// protection.
pub fn touched_paths(patch: &str) -> Result<TouchedPaths, GuardViolation> {
    if patch.trim().is_empty() {
        return Err(GuardViolation::Empty);
    }
    let lines: Vec<&str> = patch
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut touched: BTreeSet<String> = BTreeSet::new();
    let mut cur: Option<Section> = None;
    let mut in_body = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let ln = i + 1;
        i += 1;
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(s) = cur.take() {
                finish(s, &mut touched)?;
            }
            let mut sec = Section {
                start_line: ln,
                ..Section::default()
            };
            sec.paths = git_line_paths(rest, ln)?;
            cur = Some(sec);
            in_body = false;
            continue;
        }
        // `--- X` immediately followed by `+++ Y` is a file header wherever it
        // appears, even inside what looks like a hunk: over-collecting costs a
        // false refusal at worst, under-collecting would hide a path.
        if let Some(old) = line.strip_prefix("--- ") {
            if let Some(new) = lines.get(i).and_then(|l| l.strip_prefix("+++ ")) {
                let start_new = match &cur {
                    None => true,
                    Some(s) => in_body || s.old_header,
                };
                if start_new {
                    if let Some(s) = cur.take() {
                        finish(s, &mut touched)?;
                    }
                    cur = Some(Section {
                        start_line: ln,
                        ..Section::default()
                    });
                }
                let sec = cur.as_mut().expect("section just ensured");
                for (raw, mark_old) in [(old, true), (new, false)] {
                    if let Some(p) = header_path(raw, ln)? {
                        sec.paths.push(p);
                    }
                    if mark_old {
                        sec.old_header = true;
                    } else {
                        sec.new_header = true;
                    }
                }
                if old.trim_end() == "/dev/null" && new.trim_end() == "/dev/null" {
                    return Err(malformed(ln, "both sides of a file header are /dev/null"));
                }
                in_body = false;
                i += 1;
                continue;
            }
        }
        if in_body {
            if line.starts_with("@@ -") {
                continue;
            }
            match line.as_bytes().first() {
                None | Some(b' ') | Some(b'+') | Some(b'-') | Some(b'\\') => continue,
                _ => return Err(malformed(ln, "unexpected line inside a hunk")),
            }
        }
        if line.is_empty() {
            continue;
        }
        let Some(sec) = cur.as_mut() else {
            return Err(malformed(ln, "content before the first file header"));
        };
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            return Err(malformed(ln, "unpaired --- / +++ header"));
        }
        if line.starts_with("@@ -") {
            if !(sec.old_header && sec.new_header) {
                return Err(malformed(ln, "hunk without ---/+++ file headers"));
            }
            sec.has_effect = true;
            in_body = true;
            continue;
        }
        if let Some(m) = line
            .strip_prefix("new file mode ")
            .or_else(|| line.strip_prefix("deleted file mode "))
            .or_else(|| line.strip_prefix("old mode "))
            .or_else(|| line.strip_prefix("new mode "))
        {
            sec.modes.push(m.trim().to_string());
            sec.has_effect = true;
        } else if let Some(r) = line.strip_prefix("index ") {
            if let Some(mode) = r.split_whitespace().nth(1) {
                sec.modes.push(mode.to_string());
            }
        } else if line.starts_with("similarity index ") || line.starts_with("dissimilarity index ")
        {
            // metadata only
        } else if let Some(r) = line.strip_prefix("rename from ") {
            sec.paths.push(bare_path(r, ln)?);
            sec.rename_from = true;
            sec.has_effect = true;
        } else if let Some(r) = line.strip_prefix("rename to ") {
            sec.paths.push(bare_path(r, ln)?);
            sec.rename_to = true;
            sec.has_effect = true;
        } else if let Some(r) = line.strip_prefix("copy from ") {
            sec.paths.push(bare_path(r, ln)?);
            sec.copy_from = true;
            sec.has_effect = true;
        } else if let Some(r) = line.strip_prefix("copy to ") {
            sec.paths.push(bare_path(r, ln)?);
            sec.copy_to = true;
            sec.has_effect = true;
        } else if line.starts_with("GIT binary patch")
            || (line.starts_with("Binary files ") && line.ends_with(" differ"))
        {
            return Err(GuardViolation::Binary);
        } else {
            return Err(malformed(ln, format!("unrecognised header line: {line:?}")));
        }
    }
    match cur.take() {
        Some(s) => finish(s, &mut touched)?,
        None => return Err(malformed(0, "no file sections found")),
    }
    Ok(TouchedPaths(touched))
}

/// Parse `patch` strictly and refuse it if it touches anything protected.
///
/// Returns the touched paths on success. The whole patch is refused if even
/// one path is protected, and any parse doubt is a refusal. Pure and
/// deterministic.
///
/// **Call this before applying the patch.** See the [module docs](self).
pub fn check(patch: &str, policy: &ProtectedPolicy) -> Result<TouchedPaths, GuardViolation> {
    let touched = touched_paths(patch)?;
    let protected: Vec<String> = touched
        .iter()
        .filter(|p| policy.is_protected(p))
        .map(str::to_string)
        .collect();
    if protected.is_empty() {
        Ok(touched)
    } else {
        Err(GuardViolation::Protected { paths: protected })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modify(p: &str) -> String {
        format!("diff --git a/{p} b/{p}\n--- a/{p}\n+++ b/{p}\n@@ -1 +1 @@\n-old\n+new\n")
    }

    fn pol() -> ProtectedPolicy {
        ProtectedPolicy::default()
    }

    /// Assert the patch is refused as protected and the violation names `path`.
    #[track_caller]
    fn assert_protected(patch: &str, path: &str) {
        match check(patch, &pol()) {
            Err(GuardViolation::Protected { paths }) => {
                assert!(paths.iter().any(|p| p == path), "{paths:?} lacks {path}")
            }
            other => panic!("expected Protected({path}), got {other:?}\n{patch}"),
        }
    }

    #[track_caller]
    fn assert_refused(patch: &str) -> GuardViolation {
        match check(patch, &pol()) {
            Err(v) => v,
            Ok(t) => panic!("expected refusal, got Ok({t:?})\n{patch}"),
        }
    }

    #[test]
    fn plain_modify_of_an_ordinary_file_is_allowed_and_reported() {
        let t = check(&modify("src/lib.rs"), &pol()).expect("ordinary edit");
        assert!(t.contains("src/lib.rs"));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn plain_modify_of_protected_files_is_refused() {
        for p in [
            "gate.rs",
            "crates/x/gate.rs",
            ".github/workflows/ci.yml",
            "CODEOWNERS",
            "docs/CODEOWNERS",
            "SECURITY.md",
            "Cargo.toml",
            "crates/x/Cargo.toml",
            "Cargo.lock",
            "package.json",
            "web/package-lock.json",
            "deny.toml",
            "rust-toolchain.toml",
            "dream.config.json",
            "tests/heldout/case.rs",
            "src/safety_net.rs",
            "src/promotion.rs",
            "scorer.py",
            "lib/Threshold.ts",
            "receipts/baseline/x.json",
            ".git/hooks/pre-commit",
        ] {
            assert_protected(&modify(p), p);
        }
    }

    #[test]
    fn rename_with_a_protected_old_side_is_refused() {
        let p = "diff --git a/crates/x/gate.rs b/src/other.rs\nsimilarity index 100%\nrename from crates/x/gate.rs\nrename to src/other.rs\n";
        assert_protected(p, "crates/x/gate.rs");
    }

    #[test]
    fn rename_with_a_protected_new_side_is_refused() {
        let p = "diff --git a/src/a.rs b/src/gate.rs\nsimilarity index 100%\nrename from src/a.rs\nrename to src/gate.rs\n";
        assert_protected(p, "src/gate.rs");
    }

    #[test]
    fn copy_onto_or_from_a_protected_path_is_refused() {
        let onto = "diff --git a/src/a.rs b/Cargo.toml\nsimilarity index 100%\ncopy from src/a.rs\ncopy to Cargo.toml\n";
        assert_protected(onto, "Cargo.toml");
        let from = "diff --git a/gate.rs b/src/new.rs\nsimilarity index 100%\ncopy from gate.rs\ncopy to src/new.rs\n";
        assert_protected(from, "gate.rs");
    }

    #[test]
    fn rename_with_only_one_side_is_malformed() {
        let p = "diff --git a/src/a.rs b/src/b.rs\nrename from src/a.rs\n";
        assert!(matches!(
            assert_refused(p),
            GuardViolation::Malformed { .. }
        ));
    }

    #[test]
    fn a_mode_only_change_on_a_protected_file_is_refused() {
        let p = "diff --git a/Cargo.toml b/Cargo.toml\nold mode 100644\nnew mode 100755\n";
        assert_protected(p, "Cargo.toml");
    }

    #[test]
    fn a_mode_only_change_on_an_ordinary_file_is_allowed_but_not_to_a_symlink() {
        let ok = "diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n";
        assert!(check(ok, &pol()).expect("exec bit").contains("run.sh"));
        let ln = "diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 120000\n";
        assert!(matches!(assert_refused(ln), GuardViolation::Symlink { .. }));
    }

    #[test]
    fn symlink_creation_is_refused_even_on_an_allowed_path() {
        let p = "diff --git a/src/link b/src/link\nnew file mode 120000\nindex 0000000..e69de29\n--- /dev/null\n+++ b/src/link\n@@ -0,0 +1 @@\n+../../etc/passwd\n\\ No newline at end of file\n";
        assert!(matches!(assert_refused(p), GuardViolation::Symlink { .. }));
        let edit = "diff --git a/src/link b/src/link\nindex 1234567..89abcde 120000\n--- a/src/link\n+++ b/src/link\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b\n\\ No newline at end of file\n";
        assert!(matches!(
            assert_refused(edit),
            GuardViolation::Symlink { .. }
        ));
    }

    #[test]
    fn gitlinks_and_odd_modes_are_refused() {
        let p = "diff --git a/sub b/sub\nnew file mode 160000\nindex 0000000..1234567\n";
        assert!(matches!(
            assert_refused(p),
            GuardViolation::UnsupportedMode { .. }
        ));
    }

    #[test]
    fn binary_patches_are_refused() {
        let git_bin = "diff --git a/img.png b/img.png\nindex 1234567..89abcde 100644\nGIT binary patch\nliteral 3\nKcmZQz\n\nliteral 0\nHcmV?d00001\n\n";
        assert_eq!(assert_refused(git_bin), GuardViolation::Binary);
        let differ = "diff --git a/img.png b/img.png\nindex 1234567..89abcde 100644\nBinary files a/img.png and b/img.png differ\n";
        assert_eq!(assert_refused(differ), GuardViolation::Binary);
    }

    #[test]
    fn dot_slash_segments_are_normalised_before_matching() {
        assert_protected(&modify("./gate.rs"), "gate.rs");
        assert_protected(&modify("src/./gate.rs"), "src/gate.rs");
        assert_protected(&modify("crates//x///Cargo.toml"), "crates/x/Cargo.toml");
    }

    #[test]
    fn parent_segments_are_resolved_not_banned() {
        // a/ prefix then ../gate.rs: leaves the repo.
        assert!(matches!(
            assert_refused(&modify("../gate.rs")),
            GuardViolation::Escapes { .. }
        ));
        // Resolves back inside the repo onto a protected file.
        assert_protected(&modify("a/../gate.rs"), "gate.rs");
        assert_protected(
            &modify("crates/x/../../.github/workflows/y.yml"),
            ".github/workflows/y.yml",
        );
        // A `..` that stays inside and lands on an ordinary file is fine.
        let t = check(&modify("src/deep/../lib.rs"), &pol()).expect("resolves to src/lib.rs");
        assert!(t.contains("src/lib.rs"));
        // Too many parents escapes even when the tail looks harmless.
        assert!(matches!(
            assert_refused(&modify("src/../../x.rs")),
            GuardViolation::Escapes { .. }
        ));
    }

    #[test]
    fn absolute_paths_are_refused() {
        let p = "diff --git a//etc/passwd b//etc/passwd\n--- a//etc/passwd\n+++ b//etc/passwd\n@@ -1 +1 @@\n-a\n+b\n";
        assert!(matches!(assert_refused(p), GuardViolation::Absolute { .. }));
        let bare = "--- /etc/passwd\n+++ /etc/passwd\n@@ -1 +1 @@\n-a\n+b\n";
        assert_refused(bare);
        let drive = "diff --git a/C:/x b/C:/x\n--- a/C:/x\n+++ b/C:/x\n@@ -1 +1 @@\n-a\n+b\n";
        assert_refused(drive);
    }

    #[test]
    fn empty_and_prose_only_patches_are_refused() {
        assert_eq!(assert_refused(""), GuardViolation::Empty);
        assert_eq!(assert_refused("  \n\n\t\n"), GuardViolation::Empty);
        assert!(matches!(
            assert_refused("Sure! Here is my change to gate.rs."),
            GuardViolation::Malformed { .. }
        ));
    }

    #[test]
    fn truncated_and_garbled_patches_are_refused() {
        for p in [
            "diff --git a/src/lib.rs",
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n",
            "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n",
            "@@ -1 +1 @@\n-a\n+b\n",
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ garbled\n-a\n+b\n",
            "diff --git a/src/lib.rs b/src/lib.rs\nmystery header\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n",
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\nHope this helps!\n",
            "--- src/lib.rs\n+++ src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n",
            "diff --git a/\u{0}x b/\u{0}x\n--- a/\u{0}x\n+++ b/\u{0}x\n@@ -1 +1 @@\n-a\n+b\n",
        ] {
            assert_refused(p);
        }
    }

    #[test]
    fn crlf_line_endings_do_not_hide_a_protected_path() {
        let p = modify("Cargo.toml").replace('\n', "\r\n");
        assert_protected(&p, "Cargo.toml");
        let ok = modify("src/lib.rs").replace('\n', "\r\n");
        let t = check(&ok, &pol()).expect("crlf ordinary edit");
        assert!(t.contains("src/lib.rs"), "{t:?}");
    }

    #[test]
    fn quoted_paths_with_spaces_and_escapes_are_decoded() {
        let p = "diff --git \"a/dir with space/f.rs\" \"b/dir with space/f.rs\"\n--- \"a/dir with space/f.rs\"\n+++ \"b/dir with space/f.rs\"\n@@ -1 +1 @@\n-a\n+b\n";
        let t = check(p, &pol()).expect("quoted ordinary path");
        assert!(t.contains("dir with space/f.rs"), "{t:?}");
        // Octal escapes are bytes: \303\251 is é.
        let e = "diff --git \"a/caf\\303\\251.rs\" \"b/caf\\303\\251.rs\"\n--- \"a/caf\\303\\251.rs\"\n+++ \"b/caf\\303\\251.rs\"\n@@ -1 +1 @@\n-a\n+b\n";
        assert!(check(e, &pol()).expect("utf-8 name").contains("café.rs"));
        // An escaped quote is a literal quote.
        let q = "diff --git \"a/x\\\"y.rs\" \"b/x\\\"y.rs\"\n--- \"a/x\\\"y.rs\"\n+++ \"b/x\\\"y.rs\"\n@@ -1 +1 @@\n-a\n+b\n";
        assert!(check(q, &pol()).expect("quote").contains("x\"y.rs"));
    }

    #[test]
    fn quoting_cannot_smuggle_a_protected_path() {
        // \103 is 'C': the quoted name decodes to Cargo.toml.
        let oct = "diff --git \"a/\\103argo.toml\" \"b/\\103argo.toml\"\n--- \"a/\\103argo.toml\"\n+++ \"b/\\103argo.toml\"\n@@ -1 +1 @@\n-a\n+b\n";
        assert_protected(oct, "Cargo.toml");
        // Quoted spaces plus a parent segment.
        let up = "diff --git \"a/dir with space/sub/../../Cargo.toml\" \"b/dir with space/sub/../../Cargo.toml\"\n--- \"a/dir with space/sub/../../Cargo.toml\"\n+++ \"b/dir with space/sub/../../Cargo.toml\"\n@@ -1 +1 @@\n-a\n+b\n";
        assert_protected(up, "Cargo.toml");
    }

    #[test]
    fn control_characters_and_unknown_escapes_in_quoted_paths_are_refused() {
        let tab = "diff --git \"a/x\\ty.rs\" \"b/x\\ty.rs\"\n--- \"a/x\\ty.rs\"\n+++ \"b/x\\ty.rs\"\n@@ -1 +1 @@\n-a\n+b\n";
        assert_refused(tab);
        let unk = "diff --git \"a/x\\qy.rs\" \"b/x\\qy.rs\"\n--- \"a/x\\qy.rs\"\n+++ \"b/x\\qy.rs\"\n@@ -1 +1 @@\n-a\n+b\n";
        assert_refused(unk);
        let unterminated =
            "diff --git \"a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert_refused(unterminated);
    }

    #[test]
    fn one_protected_file_refuses_the_whole_patch() {
        let both = format!("{}{}", modify("src/ok.rs"), modify("crates/x/gate.rs"));
        match check(&both, &pol()) {
            Err(GuardViolation::Protected { paths }) => {
                assert_eq!(paths, vec!["crates/x/gate.rs".to_string()]);
            }
            other => panic!("whole patch must be refused, got {other:?}"),
        }
        // Order must not matter.
        let rev = format!("{}{}", modify("crates/x/gate.rs"), modify("src/ok.rs"));
        assert_protected(&rev, "crates/x/gate.rs");
    }

    #[test]
    fn a_header_that_disagrees_with_the_git_line_cannot_hide_a_path() {
        let p = "diff --git a/src/ok.rs b/src/ok.rs\n--- a/src/ok.rs\n+++ b/gate.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert_protected(p, "gate.rs");
        let hdrless = "--- a/src/ok.rs\n+++ b/src/ok.rs\n@@ -1 +1 @@\n-a\n+b\n--- a/gate.rs\n+++ b/gate.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert_protected(hdrless, "gate.rs");
    }

    #[test]
    fn new_and_deleted_files_report_their_real_path() {
        let new = "diff --git a/src/n.rs b/src/n.rs\nnew file mode 100644\nindex 0000000..e69de29\n--- /dev/null\n+++ b/src/n.rs\n@@ -0,0 +1 @@\n+x\n";
        assert!(check(new, &pol()).expect("new file").contains("src/n.rs"));
        let del = "diff --git a/gate.rs b/gate.rs\ndeleted file mode 100644\nindex e69de29..0000000\n--- a/gate.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n";
        assert_protected(del, "gate.rs");
    }

    #[test]
    fn default_policy_matches_the_documented_set() {
        let p = pol();
        for path in [
            ".github/CODEOWNERS",
            "a/b/CODEOWNERS",
            "SECURITY.md",
            "x/safety/mod.rs",
            "x/gate_impl.rs",
            "promotion/x.rs",
            "a/scorer/b.rs",
            "a/ThresholdTable.rs",
            "yarn.lock",
            "pnpm-lock.yaml",
            "rust-toolchain",
            "x/witness/y.json",
            "CARGO.TOML",
            ".gitmodules",
        ] {
            assert!(p.is_protected(path), "{path} should be protected");
        }
        for path in [
            "src/main.rs",
            "src/lib.rs",
            "docs/readme.md",
            "tests/unit.rs",
        ] {
            assert!(!p.is_protected(path), "{path} should be free");
        }
    }

    #[test]
    fn config_derived_protection_is_additive() {
        let p = pol()
            .with_globs(["docs/frozen/**", "*.secret", "vendor/*/LICENSE"])
            .with_ledger_path("docs/ledger/LEDGER.md");
        assert!(p.is_protected("docs/frozen/a/b.md"));
        assert!(p.is_protected("k/x.secret"));
        assert!(p.is_protected("vendor/foo/LICENSE"));
        assert!(!p.is_protected("vendor/foo/bar/LICENSE"));
        assert!(p.is_protected("docs/ledger/LEDGER.md"));
        assert!(p.is_protected("Cargo.toml"), "built-ins remain");
        assert!(!p.is_protected("docs/free.md"));
    }

    #[test]
    fn evaluator_command_paths_are_protected_files_and_directories() {
        assert_eq!(
            evaluator_command_paths("bash scripts/hooks.sh --config=conf/x.json | tail -3"),
            vec!["scripts/hooks.sh".to_string(), "conf/x.json".to_string()]
        );
        assert_eq!(
            evaluator_command_paths("cd web && node ./tests/run.js 2>&1"),
            vec!["tests/run.js".to_string()]
        );
        assert!(evaluator_command_paths("cargo test --workspace 2>&1 | tail -15").is_empty());
        assert!(evaluator_command_paths("curl https://example.com/a.sh").is_empty());
        let p = pol().with_evaluator_commands(["python3 -m pytest checks/ scripts/eval.py"]);
        assert!(p.is_protected("scripts/eval.py"));
        assert!(
            p.is_protected("checks/test_one.py"),
            "directory protects its tree"
        );
        assert!(!p.is_protected("src/x.py"));
    }

    #[test]
    fn self_referential_flag_protects_the_gate_crates() {
        let off = pol();
        let on = pol().with_self_referential(true);
        for p in [
            "crates/dream-gate/src/lib.rs",
            "crates/pairwise-gate/src/x.rs",
            "crates/failure-fingerprint/a.rs",
        ] {
            assert!(on.is_protected(p), "{p}");
            assert!(!off.is_protected(p), "{p} is free when the flag is off");
        }
    }

    #[test]
    fn check_is_deterministic() {
        let patch = format!("{}{}", modify("src/b.rs"), modify("src/a.rs"));
        let a = check(&patch, &pol());
        let b = check(&patch, &pol());
        assert_eq!(a, b);
        let t = a.expect("ordinary");
        assert_eq!(t.iter().collect::<Vec<_>>(), vec!["src/a.rs", "src/b.rs"]);
    }
    #[test]
    fn a_file_header_pair_hidden_inside_a_hunk_is_still_collected() {
        let p = "diff --git a/src/ok.rs b/src/ok.rs\n--- a/src/ok.rs\n+++ b/src/ok.rs\n@@ -1 +1 @@\n-a\n+b\n--- a/gate.rs\n+++ b/gate.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert_protected(p, "gate.rs");
    }

    #[test]
    fn deleted_and_added_lines_that_merely_look_like_headers_are_not_a_bypass() {
        // `-- x` removed and `++ x` added render as `--- x` / `+++ x`; the
        // guard over-collects and refuses (false refusal beats a missed path).
        let p = "diff --git a/src/ok.rs b/src/ok.rs\n--- a/src/ok.rs\n+++ b/src/ok.rs\n@@ -1 +1 @@\n--- x\n+++ x\n";
        assert_refused(p);
    }

    fn cfg_and_bytes() -> (DreamConfig, Vec<u8>) {
        let bytes = br#"{"repo":"o/r","slots":[{"deep":"a","scan":[]}],
            "ledger_path_ignored":1,
            "ledgerPath":"docs/ledger.md",
            "protected":["docs/frozen/**"],
            "evaluatorEntrypoints":{"t":"bash checks/run.sh"}}"#
            .to_vec();
        (DreamConfig::from_json(&bytes).expect("valid"), bytes)
    }

    #[test]
    fn from_config_protects_ledger_globs_and_evaluator_paths() {
        let (cfg, _) = cfg_and_bytes();
        let p = ProtectedPolicy::from_config(&cfg);
        assert!(p.is_protected("docs/ledger.md"));
        assert!(p.is_protected("docs/frozen/x.md"));
        assert!(p.is_protected("checks/run.sh"));
        assert!(!p.is_protected("src/lib.rs"));
    }

    #[test]
    fn for_manifest_refuses_a_config_that_is_not_the_frozen_one() {
        let (cfg, bytes) = cfg_and_bytes();
        let mut m = crate::manifest::build(
            &cfg,
            &bytes,
            "r",
            "n",
            "2026-01-01",
            20260101,
            "a",
            &[],
            "rev",
            "tree",
            crate::manifest::ModelIdentity {
                provider: "p".into(),
                model: "m".into(),
                max_tokens: 1,
                fallback: None,
            },
            "0",
        );
        assert!(ProtectedPolicy::for_manifest(&m, &cfg, &bytes).is_ok());
        let mut edited = bytes.clone();
        edited.push(b' ');
        assert!(matches!(
            ProtectedPolicy::for_manifest(&m, &cfg, &edited),
            Err(PolicyError::ConfigDigestMismatch { .. })
        ));
        m.config_digest = "0".repeat(64);
        assert!(ProtectedPolicy::for_manifest(&m, &cfg, &bytes).is_err());
    }
}
