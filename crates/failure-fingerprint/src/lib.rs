//! Stable identity for failure modes across the steps of an evidence-gated
//! self-improvement loop.
//!
//! A loop that runs night after night needs to know whether today's failure is
//! a **new** one, one that is **persisting** from the previous step, or one that
//! has been **fixed**. Prose cannot answer that, because the same failure is
//! reported with different temp directories, line numbers and request ids every
//! time. This crate reduces each failure to a short [`Fingerprint`] that survives
//! that noise, and keeps a [`Manifest`] of what was observed so [`advance`] can
//! classify the next step. A proposer then reads structured records rather than
//! free text.
//!
//! The fingerprint is the first 8 bytes of SHA-256 over the task, the stage and
//! the *normalised* cause, joined with the ASCII unit separator (`0x1F`). See
//! [`normalize_cause`] for exactly what is stripped; as a side effect, long
//! opaque identifiers (tokens, keys) never reach a persisted cause.
//!
//! # Example
//!
//! ```
//! use failure_fingerprint::{advance, Manifest, Observation, Stage};
//!
//! // Night 1: one failure, with a volatile temp path and line number.
//! let night1 = [Observation::new(
//!     "build-docs",
//!     Stage::new("launch"),
//!     "panic at /tmp/.tmpAb12/src/main.rs:41 in worker 7",
//! )];
//! let (m1, c1) = advance(&Manifest::new(0), &night1, 1);
//! assert_eq!(c1.new.len(), 1);
//!
//! // Night 2: same failure, different path and line.
//! let night2 = [Observation::new(
//!     "build-docs",
//!     Stage::new("launch"),
//!     "panic at /var/tmp/zz9/src/main.rs:97 in worker 3",
//! )];
//! let (m2, c2) = advance(&m1, &night2, 2);
//! assert_eq!(c2.persisting.len(), 1);
//! assert_eq!(m2.records[0].count, 2);
//!
//! // Night 3: gone.
//! let (m3, c3) = advance(&m2, &[], 3);
//! assert_eq!(c3.fixed.len(), 1);
//! assert!(m3.records.is_empty());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// Default maximum length, in `char`s, of a normalised cause.
pub const DEFAULT_MAX_CAUSE_CHARS: usize = 200;

/// The `kind` tag every serialised [`Manifest`] carries.
pub const MANIFEST_KIND: &str = "failure-manifest/1";

const SEPARATOR: u8 = 0x1F;
const MIN_ID_RUN: usize = 16;

// ---------------------------------------------------------------------------
// Normalisation
// ---------------------------------------------------------------------------

/// Characters that may start an absolute path: start of text, whitespace, or
/// one of a few opening or assigning delimiters.
fn is_path_lead(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\'' | '=' | ',' | ';' | ':')
}

/// Characters that may continue a path run.
fn is_path_char(c: char) -> bool {
    !(c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | ':' | '|'
        ))
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn replace_paths(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '/' && (i == 0 || is_path_lead(chars[i - 1])) {
            let mut j = i;
            while j < chars.len() && is_path_char(chars[j]) {
                j += 1;
            }
            let segments = chars[i..j]
                .split(|c| *c == '/')
                .filter(|seg| !seg.is_empty())
                .count();
            if segments >= 2 {
                out.push_str("<path>");
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn replace_ids(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if is_id_char(chars[i]) {
            let mut j = i;
            while j < chars.len() && is_id_char(chars[j]) {
                j += 1;
            }
            if j - i >= MIN_ID_RUN {
                out.push_str("<id>");
            } else {
                out.extend(&chars[i..j]);
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn replace_digits(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_run = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !in_run {
                out.push_str("<n>");
                in_run = true;
            }
        } else {
            in_run = false;
            out.push(c);
        }
    }
    out
}

/// Normalise a free-text failure cause so that equal failures collide.
///
/// Uses [`DEFAULT_MAX_CAUSE_CHARS`] as the length bound; see
/// [`normalize_cause_with_limit`]. Steps, in order:
///
/// 1. absolute path runs (two or more `/`-separated segments) become `<path>`;
/// 2. runs of 16 or more `[A-Za-z0-9_-]` characters become `<id>`, which also
///    keeps key material and credentials out of persisted causes;
/// 3. ASCII digit runs become `<n>`;
/// 4. whitespace is collapsed to single spaces and the ends trimmed;
/// 5. the result is truncated on a `char` boundary.
///
/// The function is idempotent.
///
/// ```
/// use failure_fingerprint::normalize_cause;
/// assert_eq!(
///     normalize_cause("open /home/a/b/c.rs:12 failed   (req 9f8e7d6c5b4a39281706)"),
///     "open <path>:<n> failed (req <id>)"
/// );
/// ```
pub fn normalize_cause(cause: &str) -> String {
    normalize_cause_with_limit(cause, DEFAULT_MAX_CAUSE_CHARS)
}

/// As [`normalize_cause`], truncating to at most `max_chars` `char`s.
///
/// Truncation never splits a code point, and trailing whitespace exposed by it
/// is trimmed so the result stays idempotent.
///
/// ```
/// use failure_fingerprint::normalize_cause_with_limit;
/// assert_eq!(normalize_cause_with_limit("ééééé", 3), "ééé");
/// ```
pub fn normalize_cause_with_limit(cause: &str, max_chars: usize) -> String {
    let s = replace_paths(cause);
    let s = replace_ids(&s);
    let s = replace_digits(&s);
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(max_chars).collect();
    truncated.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Fingerprint and Stage
// ---------------------------------------------------------------------------

/// The stage of a pipeline at which a failure occurred.
///
/// The caller owns the vocabulary. Typical values are `launch`, `trajectory`,
/// `exit` and `evaluator`, but any string is accepted.
///
/// ```
/// use failure_fingerprint::Stage;
/// let s = Stage::new("evaluator");
/// assert_eq!(s.as_str(), "evaluator");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Stage(String);

impl Stage {
    /// Create a stage from any string-like value.
    pub fn new(name: impl Into<String>) -> Self {
        Stage(name.into())
    }

    /// The stage name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Stage {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Stage {
    fn from(s: &str) -> Self {
        Stage(s.to_string())
    }
}

impl From<String> for Stage {
    fn from(s: String) -> Self {
        Stage(s)
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A 64-bit failure identity, displayed as 16 lowercase hex characters.
///
/// Orders and hashes by its bytes, so sorting fingerprints sorts their hex.
///
/// ```
/// use failure_fingerprint::Fingerprint;
/// let f: Fingerprint = "0123456789abcdef".parse().unwrap();
/// assert_eq!(f.to_string(), "0123456789abcdef");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint([u8; 8]);

impl Fingerprint {
    /// The raw 8 bytes.
    pub fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Error returned when a string is not a valid [`Fingerprint`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFingerprintError {
    input_len: usize,
}

impl fmt::Display for ParseFingerprintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid fingerprint: expected 16 hex characters, got {} bytes of input",
            self.input_len
        )
    }
}

impl std::error::Error for ParseFingerprintError {}

impl FromStr for Fingerprint {
    type Err = ParseFingerprintError;

    /// Parses exactly 16 hex characters (either case).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseFingerprintError { input_len: s.len() };
        let bytes = s.as_bytes();
        if bytes.len() != 16 {
            return Err(err());
        }
        let mut out = [0u8; 8];
        for (slot, pair) in out.iter_mut().zip(bytes.chunks(2)) {
            let hi = hex_val(pair[0]).ok_or_else(err)?;
            let lo = hex_val(pair[1]).ok_or_else(err)?;
            *slot = (hi << 4) | lo;
        }
        Ok(Fingerprint(out))
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Compute the [`Fingerprint`] of a failure.
///
/// The cause is normalised with [`normalize_cause`] first. The digest is
/// SHA-256 over `task 0x1F stage 0x1F normalised_cause`, truncated to 8 bytes.
///
/// ```
/// use failure_fingerprint::fingerprint;
/// let a = fingerprint("t", "exit", "died at /tmp/x1/y/z.rs:10");
/// let b = fingerprint("t", "exit", "died at /var/q9/w/v.rs:99");
/// assert_eq!(a, b);
/// assert_ne!(a, fingerprint("t", "launch", "died at /tmp/x1/y/z.rs:10"));
/// ```
pub fn fingerprint(task: &str, stage: impl AsRef<str>, cause: &str) -> Fingerprint {
    fingerprint_normalised(task, stage.as_ref(), &normalize_cause(cause))
}

fn fingerprint_normalised(task: &str, stage: &str, normalised_cause: &str) -> Fingerprint {
    let mut h = Sha256::new();
    h.update(task.as_bytes());
    h.update([SEPARATOR]);
    h.update(stage.as_bytes());
    h.update([SEPARATOR]);
    h.update(normalised_cause.as_bytes());
    let digest = h.finalize();
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    Fingerprint(out)
}

// ---------------------------------------------------------------------------
// Observations, records, manifest
// ---------------------------------------------------------------------------

/// One failure seen during a step, before normalisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// The task that failed.
    pub task: String,
    /// The stage at which it failed.
    pub stage: Stage,
    /// The raw, free-text cause.
    pub cause: String,
}

impl Observation {
    /// Build an observation.
    pub fn new(task: impl Into<String>, stage: impl Into<Stage>, cause: impl Into<String>) -> Self {
        Observation {
            task: task.into(),
            stage: stage.into(),
            cause: cause.into(),
        }
    }

    /// The fingerprint of this observation.
    pub fn fingerprint(&self) -> Fingerprint {
        fingerprint(&self.task, &self.stage, &self.cause)
    }
}

/// A failure mode tracked across steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureRecord {
    /// Stable identity.
    pub fingerprint: Fingerprint,
    /// The task that failed.
    pub task: String,
    /// The stage at which it failed.
    pub stage: Stage,
    /// The normalised cause (never the raw text).
    pub cause: String,
    /// Number of consecutive steps in which this failure was observed.
    pub count: u64,
    /// Step at which it was first observed in the current run of steps.
    pub first_seen_step: u64,
    /// Step at which it was last observed.
    pub last_seen_step: u64,
}

/// Error from reading a [`Manifest`].
#[derive(Debug)]
pub enum ManifestError {
    /// The `kind` field was not [`MANIFEST_KIND`].
    UnknownKind {
        /// The kind that was found.
        found: String,
    },
    /// The input was not valid JSON for a manifest.
    Json(serde_json::Error),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::UnknownKind { found } => {
                write!(
                    f,
                    "unknown manifest kind {found:?}, expected {MANIFEST_KIND:?}"
                )
            }
            ManifestError::Json(e) => write!(f, "invalid manifest JSON: {e}"),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ManifestError::Json(e) => Some(e),
            ManifestError::UnknownKind { .. } => None,
        }
    }
}

#[derive(Deserialize)]
struct RawManifest {
    kind: String,
    step: u64,
    records: Vec<FailureRecord>,
}

impl TryFrom<RawManifest> for Manifest {
    type Error = ManifestError;

    fn try_from(raw: RawManifest) -> Result<Self, Self::Error> {
        if raw.kind != MANIFEST_KIND {
            return Err(ManifestError::UnknownKind { found: raw.kind });
        }
        Ok(Manifest {
            kind: raw.kind,
            step: raw.step,
            records: raw.records,
        })
    }
}

/// The failures active at a given step, sorted by fingerprint.
///
/// Deserialisation rejects any `kind` other than [`MANIFEST_KIND`]; use
/// [`Manifest::from_json`] to get the typed [`ManifestError`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawManifest")]
pub struct Manifest {
    /// Always [`MANIFEST_KIND`].
    pub kind: String,
    /// The step this manifest describes.
    pub step: u64,
    /// Active failures, sorted by fingerprint.
    pub records: Vec<FailureRecord>,
}

impl Manifest {
    /// An empty manifest at `step`.
    pub fn new(step: u64) -> Self {
        Manifest {
            kind: MANIFEST_KIND.to_string(),
            step,
            records: Vec::new(),
        }
    }

    /// Parse a manifest from JSON, returning a typed error for an unknown kind.
    ///
    /// ```
    /// use failure_fingerprint::{Manifest, ManifestError};
    /// let bad = r#"{"kind":"other/9","step":1,"records":[]}"#;
    /// assert!(matches!(Manifest::from_json(bad), Err(ManifestError::UnknownKind { .. })));
    /// ```
    pub fn from_json(json: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest = serde_json::from_str(json).map_err(ManifestError::Json)?;
        raw.try_into()
    }

    /// Serialise to compact JSON.
    pub fn to_json(&self) -> String {
        // Plain structs of strings and integers cannot fail to serialise.
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// How the failures of a step relate to the previous manifest.
///
/// Every list is sorted by fingerprint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Classification {
    /// Observed now, absent from the previous manifest (count 1).
    pub new: Vec<FailureRecord>,
    /// Observed now and in the previous manifest (count incremented).
    pub persisting: Vec<FailureRecord>,
    /// In the previous manifest, not observed now. These are the previous
    /// records, unchanged; they are dropped from the next manifest.
    pub fixed: Vec<FailureRecord>,
}

/// Advance the loop one step.
///
/// Observations are fingerprinted and de-duplicated (the first of any
/// duplicates wins). Returns the next [`Manifest`] at `step` together with the
/// [`Classification`]. The `count` of a persisting record is simply the previous
/// count plus one; callers who need strict consecutiveness should check
/// `prev.step + 1 == step`.
///
/// ```
/// use failure_fingerprint::{advance, Manifest, Observation};
/// let obs = [Observation::new("t", "exit", "boom"), Observation::new("t", "exit", "boom")];
/// let (m, c) = advance(&Manifest::new(0), &obs, 1);
/// assert_eq!(m.records.len(), 1);
/// assert_eq!(c.new.len(), 1);
/// ```
pub fn advance(
    prev: &Manifest,
    observations: &[Observation],
    step: u64,
) -> (Manifest, Classification) {
    let prev_by_fp: BTreeMap<Fingerprint, &FailureRecord> =
        prev.records.iter().map(|r| (r.fingerprint, r)).collect();

    let mut now: BTreeMap<Fingerprint, FailureRecord> = BTreeMap::new();
    for o in observations {
        let cause = normalize_cause(&o.cause);
        let fp = fingerprint_normalised(&o.task, o.stage.as_str(), &cause);
        now.entry(fp).or_insert_with(|| FailureRecord {
            fingerprint: fp,
            task: o.task.clone(),
            stage: o.stage.clone(),
            cause,
            count: 1,
            first_seen_step: step,
            last_seen_step: step,
        });
    }

    let mut class = Classification::default();
    for (fp, rec) in now.iter_mut() {
        match prev_by_fp.get(fp) {
            Some(p) => {
                rec.count = p.count.saturating_add(1);
                rec.first_seen_step = p.first_seen_step;
                class.persisting.push(rec.clone());
            }
            None => class.new.push(rec.clone()),
        }
    }
    for (fp, p) in &prev_by_fp {
        if !now.contains_key(fp) {
            class.fixed.push((*p).clone());
        }
    }

    let next = Manifest {
        kind: MANIFEST_KIND.to_string(),
        step,
        records: now.into_values().collect(),
    };
    (next, class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn sha256_published_vector_proves_wiring() {
        // FIPS 180-2 / NIST: SHA-256("abc").
        let d = Sha256::digest(b"abc");
        let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn fingerprint_is_prefix_of_known_digest() {
        // SHA-256("a\x1fb\x1fc") computed independently; check structure instead
        // of a magic string: 16 lowercase hex chars, equal to the digest prefix.
        let f = fingerprint("a", "b", "c");
        let mut h = Sha256::new();
        h.update(b"a\x1fb\x1fc");
        let d = h.finalize();
        assert_eq!(f.as_bytes(), &d[..8]);
        let s = f.to_string();
        assert_eq!(s.len(), 16);
        assert!(s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }

    #[test]
    fn separator_prevents_field_smearing() {
        assert_ne!(fingerprint("ab", "c", "d"), fingerprint("a", "bc", "d"));
        assert_ne!(fingerprint("a", "bc", "d"), fingerprint("a", "b", "cd"));
    }

    #[test]
    fn paths_become_placeholder() {
        assert_eq!(normalize_cause("at /a/b/c.rs"), "at <path>");
        assert_eq!(normalize_cause("at /a/b/c.rs:12:3"), "at <path>:<n>:<n>");
        assert_eq!(normalize_cause("cwd=/tmp/x"), "cwd=<path>");
        assert_eq!(normalize_cause("(see /a/b)"), "(see <path>)");
    }

    #[test]
    fn single_segment_and_relative_are_kept() {
        assert_eq!(normalize_cause("in /usr now"), "in /usr now");
        assert_eq!(normalize_cause("a/b/c"), "a/b/c");
        assert_eq!(normalize_cause("and/or"), "and/or");
    }

    #[test]
    fn long_ids_are_redacted() {
        assert_eq!(normalize_cause("tok abcdefghijklmnop end"), "tok <id> end");
        assert_eq!(
            normalize_cause("short abcdefghijklmno"),
            "short abcdefghijklmno"
        );
        let secret = "sk_live_ABCDEF0123456789xyz";
        assert!(!normalize_cause(secret).contains("ABCDEF"));
    }

    #[test]
    fn digits_and_whitespace() {
        assert_eq!(
            normalize_cause("  worker  42\n\tfailed 7 times "),
            "worker <n> failed <n> times"
        );
    }

    #[test]
    fn truncation_is_char_safe() {
        let s = "é".repeat(500);
        let n = normalize_cause(&s);
        assert_eq!(n.chars().count(), DEFAULT_MAX_CAUSE_CHARS);
        assert_eq!(normalize_cause_with_limit("héllo wörld", 2), "hé");
        assert_eq!(normalize_cause_with_limit("ab cd", 3), "ab");
        assert_eq!(normalize_cause_with_limit("abc", 0), "");
    }

    #[test]
    fn survives_volatile_parts() {
        let a = fingerprint(
            "t",
            "trajectory",
            "assertion failed at /tmp/.tmpAAAA/work/src/lib.rs:10 (req 0123456789abcdef0123)",
        );
        let b = fingerprint(
            "t",
            "trajectory",
            "assertion failed at /var/folders/zz/ZZ99/src/lib.rs:2048 (req fedcba98765432100000)",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn differs_on_task_stage_or_words() {
        let base = fingerprint("t", "exit", "disk full at /a/b/c:1");
        assert_ne!(base, fingerprint("u", "exit", "disk full at /a/b/c:1"));
        assert_ne!(base, fingerprint("t", "launch", "disk full at /a/b/c:1"));
        assert_ne!(base, fingerprint("t", "exit", "disk empty at /a/b/c:1"));
    }

    #[test]
    fn fingerprint_parse_display_serde() {
        let f = fingerprint("t", "s", "c");
        let back: Fingerprint = f.to_string().parse().unwrap();
        assert_eq!(f, back);
        let upper: Fingerprint = f.to_string().to_uppercase().parse().unwrap();
        assert_eq!(f, upper);
        let j = serde_json::to_string(&f).unwrap();
        assert_eq!(j, format!("\"{f}\""));
        assert_eq!(serde_json::from_str::<Fingerprint>(&j).unwrap(), f);
        for bad in [
            "",
            "zz",
            "0123456789abcde",
            "0123456789abcdeg",
            "0123456789abcdef0",
            "é123456789abcde",
        ] {
            assert!(bad.parse::<Fingerprint>().is_err(), "{bad}");
        }
    }

    fn obs(cause: &str) -> Observation {
        Observation::new("task", Stage::new("exit"), cause)
    }

    #[test]
    fn new_persisting_fixed_lifecycle() {
        let (m1, c1) = advance(&Manifest::new(0), &[obs("alpha"), obs("beta")], 1);
        assert_eq!(c1.new.len(), 2);
        assert!(c1.persisting.is_empty() && c1.fixed.is_empty());
        assert!(m1
            .records
            .iter()
            .all(|r| r.count == 1 && r.first_seen_step == 1));

        let (m2, c2) = advance(&m1, &[obs("beta"), obs("gamma")], 2);
        assert_eq!(c2.new.len(), 1);
        assert_eq!(c2.new[0].cause, "gamma");
        assert_eq!(c2.persisting.len(), 1);
        assert_eq!(c2.persisting[0].cause, "beta");
        assert_eq!(c2.persisting[0].count, 2);
        assert_eq!(c2.persisting[0].first_seen_step, 1);
        assert_eq!(c2.persisting[0].last_seen_step, 2);
        assert_eq!(c2.fixed.len(), 1);
        assert_eq!(c2.fixed[0].cause, "alpha");
        assert_eq!(m2.records.len(), 2);
        assert!(m2.records.iter().all(|r| r.cause != "alpha"));
        assert_eq!(m2.step, 2);

        let (m3, c3) = advance(&m2, &[obs("beta")], 3);
        assert_eq!(c3.persisting[0].count, 3);
        assert_eq!(c3.fixed.len(), 1);
        assert_eq!(m3.records.len(), 1);
    }

    #[test]
    fn duplicates_collapse_and_output_sorted() {
        let o: Vec<Observation> = ["z", "a", "m", "a", "z", "k"]
            .iter()
            .map(|c| obs(c))
            .collect();
        let (m, c) = advance(&Manifest::new(0), &o, 5);
        assert_eq!(m.records.len(), 4);
        assert!(m
            .records
            .windows(2)
            .all(|w| w[0].fingerprint < w[1].fingerprint));
        assert!(c
            .new
            .windows(2)
            .all(|w| w[0].fingerprint < w[1].fingerprint));
        let mut rev = o.clone();
        rev.reverse();
        let (m2, _) = advance(&Manifest::new(0), &rev, 5);
        assert_eq!(m, m2);
    }

    #[test]
    fn fixed_then_returns_is_new_again() {
        let (m1, _) = advance(&Manifest::new(0), &[obs("x")], 1);
        let (m2, _) = advance(&m1, &[], 2);
        let (m3, c3) = advance(&m2, &[obs("x")], 3);
        assert_eq!(c3.new.len(), 1);
        assert_eq!(m3.records[0].count, 1);
        assert_eq!(m3.records[0].first_seen_step, 3);
    }

    #[test]
    fn stored_cause_is_normalised() {
        let (m, _) = advance(&Manifest::new(0), &[obs("fail /a/b/c 12")], 1);
        assert_eq!(m.records[0].cause, "fail <path> <n>");
    }

    #[test]
    fn manifest_round_trips() {
        let (m, _) = advance(&Manifest::new(0), &[obs("p"), obs("q")], 4);
        let j = m.to_json();
        assert!(j.contains("\"kind\":\"failure-manifest/1\""));
        assert_eq!(Manifest::from_json(&j).unwrap(), m);
        assert_eq!(serde_json::from_str::<Manifest>(&j).unwrap(), m);
    }

    #[test]
    fn unknown_kind_rejected_with_typed_error() {
        let j = r#"{"kind":"failure-manifest/2","step":1,"records":[]}"#;
        match Manifest::from_json(j) {
            Err(ManifestError::UnknownKind { found }) => assert_eq!(found, "failure-manifest/2"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(serde_json::from_str::<Manifest>(j).is_err());
        assert!(matches!(
            Manifest::from_json("{"),
            Err(ManifestError::Json(_))
        ));
        assert!(Manifest::from_json(j)
            .unwrap_err()
            .to_string()
            .contains("failure-manifest/2"));
    }

    proptest! {
        #[test]
        fn normalisation_is_idempotent(s in "\\PC{0,300}") {
            let once = normalize_cause(&s);
            prop_assert_eq!(normalize_cause(&once), once);
        }

        #[test]
        fn idempotent_on_path_and_id_heavy_input(s in "[ a-z0-9/_:.<>()=,-]{0,200}") {
            let once = normalize_cause(&s);
            prop_assert_eq!(normalize_cause(&once), once);
        }

        #[test]
        fn idempotent_with_small_limits(s in "[ a-zé/0-9]{0,80}", limit in 0usize..40) {
            let once = normalize_cause_with_limit(&s, limit);
            prop_assert_eq!(normalize_cause_with_limit(&once, limit), once);
        }

        #[test]
        fn truncation_bounded_and_boundary_safe(s in "[é日本語🙂 a-z]{0,400}", limit in 0usize..60) {
            let n = normalize_cause_with_limit(&s, limit);
            prop_assert!(n.chars().count() <= limit);
            prop_assert!(n.is_char_boundary(n.len()));
            prop_assert!(!n.ends_with(char::is_whitespace));
        }

        #[test]
        fn volatile_parts_do_not_change_fingerprint(
            d1 in "[a-zA-Z0-9]{1,12}", d2 in "[a-zA-Z0-9]{1,12}",
            l1 in 1u32..100000, l2 in 1u32..100000,
            r1 in "[a-f0-9]{20,32}", r2 in "[a-f0-9]{20,32}",
        ) {
            let c1 = format!("panic at /tmp/.tmp{d1}/src/lib.rs:{l1} req {r1}");
            let c2 = format!("panic at /var/x/{d2}/src/lib.rs:{l2} req {r2}");
            prop_assert_eq!(fingerprint("t", "exit", &c1), fingerprint("t", "exit", &c2));
        }
    }
}
