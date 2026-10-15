//! The repository-side configuration *schema* and its validation.
//!
//! Only the shape of a repository's evolution config lives here: slots,
//! evaluator entrypoints, ledger path, additive protected globs. **No
//! environment- or estate-specific default exists in this module** — every
//! value a caller must choose (schedule, ledger location, branch prefix) is
//! empty unless the document supplies it, and the caller applies its own
//! defaults afterwards.
//!
//! ```
//! use dream_gate::config::DreamConfig;
//! let cfg = DreamConfig::from_json(br#"{
//!     "repo": "owner/name",
//!     "slots": [{"deep": "core", "scan": ["src"]}],
//!     "evaluatorEntrypoints": {"tests": "cargo test"},
//!     "protected": ["docs/frozen/**"]
//! }"#)?;
//! assert!(cfg.evaluator_entrypoints["tests"].required);
//! assert!(cfg.ledger_path.is_empty()); // the caller supplies defaults
//! # Ok::<(), dream_gate::config::ConfigError>(())
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

/// Errors from reading or validating a config document.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("reading config: {0}")]
    Io(#[from] std::io::Error),
    /// The document is not valid JSON for this schema.
    #[error("parsing config: {0}")]
    Parse(#[from] serde_json::Error),
    /// The document parsed but breaks a validation rule.
    #[error("validation: {0}")]
    Validation(String),
}

/// One rotation slot: a named focus area and the paths it scans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    /// Slot name (a "deep").
    pub deep: String,
    /// Repo-relative paths the slot scans.
    pub scan: Vec<String>,
}

/// An optional build step run before evaluators.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildStep {
    /// The command line.
    pub cmd: String,
    /// Whether a WASM-target failure degrades instead of failing the build.
    #[serde(default)]
    pub degrade_on_wasm_failure: bool,
}

/// The machine-readable per-test output format of an evaluator, when its
/// output can be parsed into per-test outcomes (see `dream-core`'s `testfmt`).
///
/// Declared in the config, so it is frozen with the config digest and with
/// the manifest: the format an evaluator is *read* with cannot change between
/// baseline and candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TestFormat {
    /// `cargo test` text output (`test name ... ok`).
    Libtest,
    /// JUnit XML (`<testsuite>`/`<testcase>`).
    Junit,
    /// Vitest `--reporter=json` output (Jest-compatible shape).
    VitestJson,
}

/// ADR numbering convention.
///
/// Accepts what the reference validator accepts: `"3-digit"`, `"4-digit"` or
/// an object `{ "pad": <integer >= 1>, "dir": "<non-empty>" }`. Absent means
/// [`AdrConvention::Unset`] and this crate has no opinion.
///
/// ```
/// use dream_gate::config::AdrConvention;
/// let c: AdrConvention = serde_json::from_str(r#"{"pad":3,"dir":"docs/adr"}"#).unwrap();
/// assert_eq!(c.pad(), 3);
/// assert_eq!(c.dir(), Some("docs/adr"));
/// assert!(serde_json::from_str::<AdrConvention>("\"5-digit\"").is_err());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AdrConvention {
    /// Not configured.
    #[default]
    Unset,
    /// `ADR-001`.
    ThreeDigit,
    /// `ADR-0001`.
    FourDigit,
    /// A free-form numbering template such as `ADR-XX-NNN`, where the longest
    /// run of `N` sets the zero-padding width. Accepted so that configs written
    /// for earlier engines, which took any string, keep loading.
    Template(String),
    /// A custom zero-padding width and directory.
    Custom {
        /// Zero-padding width, at least 1.
        pad: u32,
        /// Directory holding the ADRs, non-empty.
        dir: String,
    },
}

/// Width of the longest run of `N` in a numbering template, or `None` when the
/// text has no `N` at all and so is not a template.
fn template_pad(t: &str) -> Option<usize> {
    let (mut best, mut run) = (0usize, 0usize);
    for c in t.chars() {
        if c == 'N' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    (best > 0).then_some(best)
}

impl AdrConvention {
    /// True when not configured.
    pub fn is_unset(&self) -> bool {
        matches!(self, AdrConvention::Unset)
    }

    /// Zero-padding width; an unset convention reads as four digits, as in
    /// the reference implementation.
    pub fn pad(&self) -> usize {
        match self {
            AdrConvention::ThreeDigit => 3,
            AdrConvention::Template(t) => template_pad(t).unwrap_or(4),
            AdrConvention::Custom { pad, .. } => *pad as usize,
            AdrConvention::Unset | AdrConvention::FourDigit => 4,
        }
    }

    /// The configured directory, for the custom form only.
    pub fn dir(&self) -> Option<&str> {
        match self {
            AdrConvention::Custom { dir, .. } => Some(dir),
            _ => None,
        }
    }
}

impl Serialize for AdrConvention {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self {
            // Never written: the field is skipped when unset.
            AdrConvention::Unset => s.serialize_none(),
            AdrConvention::ThreeDigit => s.serialize_str("3-digit"),
            AdrConvention::FourDigit => s.serialize_str("4-digit"),
            AdrConvention::Template(t) => s.serialize_str(t),
            AdrConvention::Custom { pad, dir } => {
                let mut st = s.serialize_struct("AdrConvention", 2)?;
                st.serialize_field("pad", pad)?;
                st.serialize_field("dir", dir)?;
                st.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for AdrConvention {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Name(String),
            Custom { pad: u32, dir: String },
        }
        match Raw::deserialize(d)? {
            Raw::Name(n) => match n.as_str() {
                "3-digit" => Ok(AdrConvention::ThreeDigit),
                "4-digit" => Ok(AdrConvention::FourDigit),
                other if template_pad(other).is_some() => {
                    Ok(AdrConvention::Template(other.to_string()))
                }
                other => Err(D::Error::custom(format!(
                    "adrConvention must be \"3-digit\", \"4-digit\", a template such as \"ADR-XX-NNN\", or {{ pad, dir }} (got {other:?})"
                ))),
            },
            Raw::Custom { pad, dir } => Ok(AdrConvention::Custom { pad, dir }),
        }
    }
}

/// Default wall-clock budget for a single evaluator run, in seconds.
///
/// Thirty minutes sits above a cold full test run and well below a nightly
/// window, so a hung evaluator surfaces as a timed-out veto rather than eating
/// the whole window.
pub const DEFAULT_EVALUATOR_TIMEOUT_SECS: u64 = 1800;

/// One evaluator entrypoint: the command, whether it can veto acceptance, which
/// deep slots it covers, and its wall-clock budget.
///
/// Deserialises from either a bare string (`"tests": "cargo test"`) or the
/// explicit object form. The bare string is read fail-closed — `required:
/// true`, all deeps — because an evaluator a repo bothered to declare is
/// evidence the run is expected to honour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluatorSpec {
    /// Shell command line.
    pub cmd: String,
    /// Whether a bad result vetoes acceptance.
    pub required: bool,
    /// Deep slots covered. Empty means every deep.
    pub deeps: Vec<String>,
    /// Wall-clock budget in seconds.
    pub timeout_secs: u64,
    /// How to read per-test outcomes from the evaluator's output. `None`
    /// (the default) counts the evaluator as one case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_format: Option<TestFormat>,
}

impl EvaluatorSpec {
    /// The bare-string reading: required, every deep, default timeout.
    pub fn command(cmd: &str) -> Self {
        Self {
            cmd: cmd.to_string(),
            required: true,
            deeps: Vec::new(),
            timeout_secs: DEFAULT_EVALUATOR_TIMEOUT_SECS,
            test_format: None,
        }
    }

    /// An advisory evaluator: it runs and is recorded, but cannot veto.
    pub fn advisory(cmd: &str) -> Self {
        Self {
            required: false,
            ..Self::command(cmd)
        }
    }

    /// True when this evaluator applies to `deep` (empty `deeps` ⇒ all deeps).
    pub fn covers(&self, deep: &str) -> bool {
        self.deeps.is_empty() || self.deeps.iter().any(|d| d == deep)
    }
}

impl<'de> Deserialize<'de> for EvaluatorSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Detailed {
            cmd: String,
            #[serde(default = "default_true")]
            required: bool,
            #[serde(default)]
            deeps: Vec<String>,
            #[serde(default = "default_evaluator_timeout")]
            timeout_secs: u64,
            #[serde(default)]
            test_format: Option<TestFormat>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Command(String),
            Detailed(Detailed),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Command(cmd) => EvaluatorSpec::command(&cmd),
            Raw::Detailed(x) => EvaluatorSpec {
                cmd: x.cmd,
                required: x.required,
                deeps: x.deeps,
                timeout_secs: x.timeout_secs,
                test_format: x.test_format,
            },
        })
    }
}

fn default_true() -> bool {
    true
}

fn default_evaluator_timeout() -> u64 {
    DEFAULT_EVALUATOR_TIMEOUT_SECS
}

/// A repository's evolution configuration document (`dream.config.json`).
///
/// Fields that other layers must default (`cron`, `ledger_path`,
/// `branch_prefix`) are **empty when absent**, and `adr_convention` is
/// [`AdrConvention::Unset`]: this crate has no opinion about them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamConfig {
    /// `owner/name` slug.
    pub repo: String,
    /// Schedule expression; empty when unset.
    #[serde(default)]
    pub cron: String,
    /// Rotation slots; must be non-empty.
    pub slots: Vec<Slot>,
    /// Modulus → extra-slot-name map used by the scheduler.
    #[serde(default)]
    pub bonus_moduli: HashMap<String, String>,
    /// Control-plane probe commands.
    #[serde(default)]
    pub control_plane_probes: Vec<String>,
    /// Optional pre-evaluator build step.
    #[serde(default)]
    pub build_step: Option<BuildStep>,
    /// Sibling workspace repos (names relative to the workspace root) whose
    /// sources the build or evaluators need alongside this repo. Empty means
    /// none.
    #[serde(default, alias = "annexeInclude")]
    pub sibling_repos: Vec<String>,
    /// Evaluator entrypoints, keyed by evaluator name.
    ///
    /// Two value forms are accepted; the plain string is the canonical short
    /// form:
    ///
    /// ```json
    /// "evaluatorEntrypoints": {
    ///   "tests":  "cargo test 2>&1 | tail -15",
    ///   "hooks":  { "cmd": "bash scripts/hooks.sh", "required": false,
    ///               "deeps": ["hooks-pipeline"], "timeoutSecs": 600 }
    /// }
    /// ```
    ///
    /// A bare string means `required: true`, every deep, default timeout.
    #[serde(default)]
    pub evaluator_entrypoints: HashMap<String, EvaluatorSpec>,
    /// Competitor repositories to study.
    #[serde(default)]
    pub competitors: Vec<String>,
    /// ADR numbering convention; [`AdrConvention::Unset`] when absent.
    #[serde(default, skip_serializing_if = "AdrConvention::is_unset")]
    pub adr_convention: AdrConvention,
    /// Extra discipline names.
    #[serde(default)]
    pub extra_disciplines: Vec<String>,
    /// Repo-relative ledger path; empty when unset. A non-empty value is
    /// always protected from candidate patches.
    #[serde(default)]
    pub ledger_path: String,
    /// Branch prefix for persisted candidates; empty when unset.
    #[serde(default)]
    pub branch_prefix: String,
    /// Pull-request labels.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Whether an accepted candidate's pull request may be set to auto-merge.
    ///
    /// **Default off.** Accepted as `autoMerge` or `automerge`. The config is a
    /// protected path, so a candidate patch cannot switch it on. Even when
    /// `true` the host still applies `dream_core::automerge::decide`, which
    /// refuses self-referential targets and branches without required status
    /// checks.
    #[serde(default, alias = "automerge")]
    pub auto_merge: bool,
    /// Extra protected path globs. **Additive only**: these extend the
    /// built-in protected set and can never remove from it (negation with a
    /// leading `!` is a validation error).
    #[serde(default)]
    pub protected: Vec<String>,
}

impl DreamConfig {
    /// Parse and validate a config document from raw bytes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ConfigError> {
        let cfg: Self = serde_json::from_slice(bytes)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Read, parse and validate a config file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::from_json(&std::fs::read(path)?)
    }

    /// Check the document's invariants.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let bad = |m: String| Err(ConfigError::Validation(m));
        if !valid_repo_slug(&self.repo) {
            return bad(format!("repo must be \"owner/name\" (got {:?})", self.repo));
        }
        if !self.cron.is_empty() {
            if let Some(why) = bad_cron(&self.cron) {
                return bad(format!("cron: {why}"));
            }
        }
        for k in self.bonus_moduli.keys() {
            if k.is_empty() || !k.bytes().all(|b| b.is_ascii_digit()) {
                return bad(format!("bonusModuli key {k:?} must be an integer"));
            }
        }
        if let AdrConvention::Custom { pad, dir } = &self.adr_convention {
            if *pad < 1 {
                return bad("adrConvention.pad must be an integer >= 1".into());
            }
            if dir.trim().is_empty() {
                return bad("adrConvention.dir must be a non-empty string".into());
            }
        }
        if self.slots.is_empty() {
            return bad("slots must not be empty".into());
        }
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.deep.is_empty() {
                return bad(format!("slot[{i}].deep must not be empty"));
            }
        }
        let known_deeps: Vec<&str> = self.slots.iter().map(|s| s.deep.as_str()).collect();
        for (name, spec) in &self.evaluator_entrypoints {
            if spec.cmd.trim().is_empty() {
                return bad(format!(
                    "evaluatorEntrypoints[{name}]: command must not be empty"
                ));
            }
            // The `darwin` evolution harness defaults to a `--sandbox real`
            // mode that is independent of the evolved surfaces: the run goes
            // green while learning nothing. Only `mock` and `agent` exercise
            // them, so anything else is refused rather than silently no-opping.
            let cmd = &spec.cmd;
            let is_darwin =
                cmd.contains("@metaharness/darwin") || cmd.contains("metaharness-darwin");
            if is_darwin && !cmd.contains("--sandbox mock") && !cmd.contains("--sandbox agent") {
                return bad(format!(
                    "evaluatorEntrypoints[{name}]: @metaharness/darwin entrypoints must pass \
                     --sandbox mock or --sandbox agent (the default 'real' sandbox is \
                     surface-independent and silently no-ops the run)"
                ));
            }
            // A `deeps` entry naming no declared slot silently disables the
            // evaluator for every run — the "declared but never runs" failure.
            for d in &spec.deeps {
                if !known_deeps.contains(&d.as_str()) {
                    return bad(format!(
                        "evaluatorEntrypoints[{name}].deeps: {d:?} is not a declared slot deep \
                         (known: {})",
                        known_deeps.join(", ")
                    ));
                }
            }
            if spec.timeout_secs == 0 {
                return bad(format!(
                    "evaluatorEntrypoints[{name}].timeoutSecs must be greater than zero"
                ));
            }
        }
        for g in &self.protected {
            if let Some(why) = bad_protected_glob(g) {
                return bad(format!("protected[{g:?}]: {why}"));
            }
        }
        Ok(())
    }
}

/// `^[\w.-]+/[\w.-]+$` with JavaScript's ASCII `\w`.
fn valid_repo_slug(repo: &str) -> bool {
    let ok = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    };
    matches!(repo.split_once('/'), Some((o, n)) if ok(o) && ok(n))
}

/// Why a cron expression is unacceptable, or `None`: five whitespace-separated
/// fields, and a minute field that is a single value `0..=59` (so the minimum
/// interval is one hour).
fn bad_cron(cron: &str) -> Option<&'static str> {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return Some("must be a 5-field expression");
    }
    let m = fields[0];
    let single = !m.is_empty()
        && m.len() <= 2
        && m.bytes().all(|b| b.is_ascii_digit())
        && (m.len() == 1 || m.as_bytes()[0] != b'0')
        && m.parse::<u8>().is_ok_and(|v| v <= 59);
    if !single {
        return Some(
            "minute field must be a single value from 0 to 59; minimum interval is 1 hour",
        );
    }
    None
}

/// Why a `protected` glob is unacceptable, or `None` when it is fine.
///
/// The set is additive, so a glob must be a plain repo-relative pattern:
/// non-empty, not negated, not absolute, no `..` segment, no backslash or
/// control character.
fn bad_protected_glob(g: &str) -> Option<&'static str> {
    if g.trim().is_empty() {
        return Some("must not be empty");
    }
    if g.starts_with('!') {
        return Some("negation is not allowed: the protected set is additive only");
    }
    if g.starts_with('/') || g.starts_with('~') || g.as_bytes().get(1) == Some(&b':') {
        return Some("must be repo-relative, not absolute");
    }
    if g.split('/').any(|seg| seg == "..") {
        return Some("must not contain a `..` segment");
    }
    if g.contains('\\') || g.chars().any(char::is_control) {
        return Some("must not contain backslashes or control characters");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(extra: serde_json::Value) -> Result<DreamConfig, ConfigError> {
        let mut v = serde_json::json!({
            "repo": "o/r",
            "slots": [{"deep": "a", "scan": []}]
        });
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        DreamConfig::from_json(v.to_string().as_bytes())
    }

    #[test]
    fn defaults_are_neutral_and_empty() {
        let c = cfg(serde_json::json!({})).unwrap();
        assert!(c.cron.is_empty());
        assert!(c.adr_convention.is_unset());
        assert!(c.ledger_path.is_empty());
        assert!(c.branch_prefix.is_empty());
        assert!(c.protected.is_empty());
        assert!(c.sibling_repos.is_empty());
        assert!(!c.auto_merge);
    }

    #[test]
    fn sibling_repos_parses() {
        let c = cfg(serde_json::json!({"siblingRepos": ["sibling-a", "sibling-b"]})).unwrap();
        assert_eq!(c.sibling_repos, vec!["sibling-a", "sibling-b"]);
        let legacy = cfg(serde_json::json!({"annexeInclude": ["old"]})).unwrap();
        assert_eq!(legacy.sibling_repos, vec!["old"]);
    }

    #[test]
    fn rejects_empty_slots() {
        let r = DreamConfig::from_json(br#"{"repo":"o/r","slots":[]}"#);
        assert!(matches!(r, Err(ConfigError::Validation(_))));
    }

    #[test]
    fn bare_string_evaluator_is_required_and_fail_closed() {
        let c = cfg(serde_json::json!({"evaluatorEntrypoints": {"t": "cargo test"}})).unwrap();
        let s = &c.evaluator_entrypoints["t"];
        assert!(s.required && s.deeps.is_empty());
        assert_eq!(s.timeout_secs, DEFAULT_EVALUATOR_TIMEOUT_SECS);
    }

    #[test]
    fn undeclared_deep_and_zero_timeout_and_empty_command_are_rejected() {
        let e = |v| cfg(serde_json::json!({ "evaluatorEntrypoints": v }));
        assert!(e(serde_json::json!({"x": {"cmd": "c", "deeps": ["nope"]}})).is_err());
        assert!(e(serde_json::json!({"x": {"cmd": "c", "timeoutSecs": 0}})).is_err());
        assert!(e(serde_json::json!({"x": "   "})).is_err());
        assert!(e(serde_json::json!({"x": {"cmd": "c", "deeps": ["a"]}})).is_ok());
    }

    #[test]
    fn protected_globs_are_additive_and_repo_relative() {
        assert!(cfg(serde_json::json!({"protected": ["docs/frozen/**", "a/*.md"]})).is_ok());
        for bad in [
            "",
            "!src/**",
            "/etc/passwd",
            "a/../b",
            "~/x",
            "C:/x",
            "a\\b",
        ] {
            assert!(
                cfg(serde_json::json!({"protected": [bad]})).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    fn with_evaluator(cmd: &str) -> Result<DreamConfig, ConfigError> {
        cfg(serde_json::json!({"evaluatorEntrypoints": {"fitness": cmd}}))
    }

    #[test]
    fn rejects_darwin_evaluator_without_sandbox_flag() {
        assert!(with_evaluator("npx -y @metaharness/darwin@~0.8.0 evolve .").is_err());
        assert!(with_evaluator("metaharness-darwin evolve . --sandbox real").is_err());
    }

    #[test]
    fn accepts_darwin_evaluator_with_mock_or_agent_sandbox() {
        assert!(with_evaluator("metaharness-darwin evolve . --sandbox mock").is_ok());
        assert!(
            with_evaluator("npx -y @metaharness/darwin@~0.8.0 evolve . --sandbox agent").is_ok()
        );
    }

    #[test]
    fn non_darwin_evaluators_unaffected() {
        assert!(with_evaluator("cargo test --workspace").is_ok());
        assert!(with_evaluator("node /opt/tool/cli.js evolve .").is_ok());
    }

    #[test]
    fn evaluator_spec_helpers() {
        assert!(EvaluatorSpec::command("x").required);
        assert!(!EvaluatorSpec::advisory("x").required);
        let mut s = EvaluatorSpec::command("x");
        s.deeps = vec!["d".into()];
        assert!(s.covers("d") && !s.covers("e"));
    }
}
