//! Compile a deterministic nightly prompt from a config, tonight's slot and a
//! template.
//!
//! The *mechanism* is a placeholder substitution over a [`PromptTemplate`]
//! with sections (bonus dives, competitors, evaluators, build step) rendered
//! from the config. [`PromptTemplate::builtin`] is a neutral, self-contained
//! template for a **single completion**: the model is told what the engine has
//! already done and will do after the reply, how to freeze a hypothesis, how to
//! grade evidence, how to emit a candidate diff and a ledger row, and that it
//! must never claim to have run, published or merged anything. Hosts that want
//! different wording supply their own template; nothing else changes.
//!
//! Substitution is **single pass**: text inside a substituted value is never
//! re-expanded, so a repo name or evaluator command containing `{{x}}` cannot
//! inject another placeholder.
//!
//! # Example
//!
//! ```
//! use dream_core::compile::{compile, PromptTemplate};
//! use dream_gate::config::DreamConfig;
//!
//! let cfg = DreamConfig::from_json(br#"{
//!   "repo": "example/widgets",
//!   "slots": [{"deep": "parser", "scan": ["tokeniser"]}],
//!   "evaluatorEntrypoints": {"tests": "cargo test"}
//! }"#)?;
//! let prompt = compile(&cfg, &cfg.slots[0], 20260815, &[], &PromptTemplate::builtin())?;
//! assert!(prompt.contains("example/widgets") && prompt.contains("VERDICT: ACCEPT"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use dream_gate::config::{DreamConfig, Slot};

/// Why a template could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// The template names a placeholder this crate does not provide.
    #[error("unknown placeholder {{{{{0}}}}} in prompt template")]
    UnknownPlaceholder(String),
    /// A `{{` was opened and never closed.
    #[error("unterminated placeholder in prompt template")]
    Unterminated,
}

/// A prompt template: literal text with `{{name}}` placeholders.
///
/// Known placeholders: `repo`, `day_int`, `slot_idx`, `deep`, `scans`,
/// `date_iso`, `bonus_section`, `competitors_section`, `evaluators_section`,
/// `build_section`, `merge_policy`, `branch_prefix`, `labels`,
/// `discipline_list`, `ledger_path`, `adr_example`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate(String);

/// The placeholders [`compile`] provides.
pub const PLACEHOLDERS: [&str; 16] = [
    "repo",
    "day_int",
    "slot_idx",
    "deep",
    "scans",
    "date_iso",
    "bonus_section",
    "competitors_section",
    "evaluators_section",
    "build_section",
    "merge_policy",
    "branch_prefix",
    "labels",
    "discipline_list",
    "ledger_path",
    "adr_example",
];

impl PromptTemplate {
    /// A template from caller-supplied text. Validated when rendered.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The template text.
    pub fn text(&self) -> &str {
        &self.0
    }

    /// The neutral built-in single-completion template.
    pub fn builtin() -> Self {
        Self(BUILTIN.to_string())
    }

    /// Check that every placeholder is known and every `{{` is closed,
    /// without rendering.
    pub fn validate(&self) -> Result<(), TemplateError> {
        self.render_with(|_| Some(String::new())).map(|_| ())
    }

    fn render_with(
        &self,
        mut value: impl FnMut(&str) -> Option<String>,
    ) -> Result<String, TemplateError> {
        let mut out = String::with_capacity(self.0.len() + 512);
        let mut rest = self.0.as_str();
        while let Some(open) = rest.find("{{") {
            out.push_str(&rest[..open]);
            let after = &rest[open + 2..];
            let close = after.find("}}").ok_or(TemplateError::Unterminated)?;
            let name = after[..close].trim();
            if !PLACEHOLDERS.contains(&name) {
                return Err(TemplateError::UnknownPlaceholder(name.to_string()));
            }
            out.push_str(&value(name).unwrap_or_default());
            rest = &after[close + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

/// Compile the nightly prompt for `slot` on `day_int` (`YYYYMMDD`).
///
/// Deterministic: the same inputs give byte-identical output.
pub fn compile(
    cfg: &DreamConfig,
    slot: &Slot,
    day_int: u32,
    bonus_dives: &[String],
    template: &PromptTemplate,
) -> Result<String, TemplateError> {
    let slot_idx = (day_int as usize) % cfg.slots.len().max(1);
    let scans = slot.scan.join(", ");

    let bonus_section = if bonus_dives.is_empty() {
        String::new()
    } else {
        format!(
            "\n## Bonus dives (triggered tonight)\n{}\n",
            bonus_dives
                .iter()
                .enumerate()
                .map(|(i, d)| format!("{}. {}", i + 1, d))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let competitors_section = if cfg.competitors.is_empty() {
        String::new()
    } else {
        format!(
            "\n## Competitive landscape\nPosition findings relative to: {}.\n",
            cfg.competitors.join(", ")
        )
    };

    let evaluators_section = if cfg.evaluator_entrypoints.is_empty() {
        String::new()
    } else {
        let mut entries: Vec<_> = cfg.evaluator_entrypoints.iter().collect();
        entries.sort_by_key(|(k, _)| (*k).clone());
        let lines: Vec<String> = entries
            .iter()
            .map(|(name, spec)| {
                format!(
                    "- **{}** ({}): `{}`",
                    name,
                    if spec.required {
                        "REQUIRED — a bad result vetoes acceptance"
                    } else {
                        "advisory"
                    },
                    spec.cmd
                )
            })
            .collect();
        format!(
            "\n## Evaluator entrypoints (the engine runs these on the baseline and on your candidate — you cannot)\n{}\n\
             A REQUIRED evaluator that is missing, silent, blocked, timed out or failing \
             vetoes ACCEPT deterministically, whatever this report says.\n",
            lines.join("\n")
        )
    };

    let build_section = match &cfg.build_step {
        Some(bs) => format!(
            "\n## Build step\n```bash\n{}\n```\n{}\n",
            bs.cmd,
            if bs.degrade_on_wasm_failure {
                "If WASM compilation fails, degrade gracefully — do not halt."
            } else {
                ""
            }
        ),
        None => String::new(),
    };

    let merge_policy = if cfg.auto_merge {
        "GUARDED auto-merge is enabled. The PR may be merged automatically if ALL gates pass."
    } else {
        "Human-review-only. The nightly NEVER merges; a human does."
    };

    let disciplines = [
        "evaluation-is-not-promotion",
        "witness-every-quantitative-claim",
    ]
    .into_iter()
    .map(str::to_string)
    .chain(cfg.extra_disciplines.iter().cloned());
    let discipline_list = disciplines
        .map(|d| format!("- {d}"))
        .collect::<Vec<_>>()
        .join("\n");

    let adr_example = format!("ADR-{:0>width$}", 1, width = cfg.adr_convention.pad());
    let labels = if cfg.labels.is_empty() {
        "none".to_string()
    } else {
        cfg.labels.join(", ")
    };

    template.render_with(|name| {
        Some(match name {
            "repo" => cfg.repo.clone(),
            "day_int" => day_int.to_string(),
            "slot_idx" => slot_idx.to_string(),
            "deep" => slot.deep.clone(),
            "scans" => scans.clone(),
            "date_iso" => date_iso(day_int),
            "bonus_section" => bonus_section.clone(),
            "competitors_section" => competitors_section.clone(),
            "evaluators_section" => evaluators_section.clone(),
            "build_section" => build_section.clone(),
            "merge_policy" => merge_policy.to_string(),
            "branch_prefix" => cfg.branch_prefix.clone(),
            "labels" => labels.clone(),
            "discipline_list" => discipline_list.clone(),
            "ledger_path" => cfg.ledger_path.clone(),
            "adr_example" => adr_example.clone(),
            _ => return None,
        })
    })
}

/// `20260815` → `2026-08-15`; anything else is passed through unchanged so the
/// row stays well-formed text even for a synthetic day integer.
fn date_iso(day_int: u32) -> String {
    let s = day_int.to_string();
    if s.len() == 8 {
        format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..])
    } else {
        s
    }
}

const BUILTIN: &str = r#"# Nightly evolution prompt
# Repo: {{repo}}
# Date integer: {{day_int}} / Slot: {{slot_idx}} / Deep: {{deep}} / Scans: {{scans}}

You are an overnight research analyst for **{{repo}}**.
Tonight's deep dive: **{{deep}}**. Surface scans: **{{scans}}**.
{{bonus_section}}{{competitors_section}}
## Operating principles

1. **{{merge_policy}}**
2. One falsifiable hypothesis per night, frozen BEFORE evaluation.
3. Grade evidence: A = evaluator receipt in this prompt, B = source shown in this prompt, C = inference.
4. ADR convention: {{adr_example}} (sequential).
5. Branch prefix: `{{branch_prefix}}`.
6. Labels: {{labels}}.

## Disciplines
{{discipline_list}}
{{build_section}}{{evaluators_section}}
## What you are, and what the engine does

You are a **single model call**. You cannot run commands, read anything beyond this prompt, open branches or pull requests, or write the ledger. Before calling you the engine has already: checked out the commit named under "Session commit", run the build step and every evaluator listed above (their receipts are under the evidence section), and read the source files most relevant to tonight's deep from that same commit (the `## Source` section, when present).

After you reply, the **engine** extracts your ```dream-patch block, applies it to that commit in isolation, re-runs every REQUIRED evaluator against the candidate, and decides the verdict from those receipts — an ACCEPT whose candidate does not pass is vetoed whatever this report says. The engine then writes the ledger row at `{{ledger_path}}` and, only on an upheld ACCEPT, opens a **draft** pull request on a `{{branch_prefix}}<deep>-<date>` branch for a human to merge.

Never claim to have run, built, tested, published, pushed or merged anything. Report what the receipts and source show, and what you predict the candidate will do.

## Method

### 1 — Orientation
1. Read the recent ledger rows in the evidence. Note streaks, repeated directions and the blockers earlier nights named; do not retry a direction an earlier night showed is blocked.
2. Read tonight's receipts and source for the scans **{{scans}}**. Collect raw observations, citing receipt names and `path:line`.

### 2 — Hypothesis
3. Rank at most five candidate findings by fitness to the deep, novelty, testability against the REQUIRED evaluators, reviewability and smallness of the diff. Pick the highest-scoring one whose change you can write against the source shown.
4. **Freeze the hypothesis** in exactly this template, and do not modify it afterwards:
   > Given <precondition>, when <action>, then <expected>.

### 3 — Candidate
5. Write the smallest change that tests the hypothesis as a unified diff against the source shown. Copy context lines verbatim from it. Never write a hunk inside an elided region or against a file that is not shown (creating a new file is fine). If the source you need was not provided, name the file and line range and answer INCONCLUSIVE with that as the blocker.
6. For each REQUIRED evaluator, predict what it will report on the candidate and why, citing receipt lines.
7. Critic: assume the candidate is subtly wrong and find the flaw. Reward-hack check: a change that weakens a test, benchmark, threshold or gold answer is not a candidate.
8. Evidence-grade every claim (A, B or C as above) and label anything unverifiable.
9. Security: no credentials, personal data or destructive operations in the diff or the report.

### 4 — Verdict
10. Choose the verdict:
    - **ACCEPT**: you emit a candidate diff you expect to pass every REQUIRED evaluator with no regression and no reward-hack. The engine's re-run decides whether it stands.
    - **REJECT**: the evidence falsifies the hypothesis, or the only available change would regress.
    - **INCONCLUSIVE**: the evidence or source is insufficient to write or judge a candidate. Name the blocker.
11. If ACCEPT, emit the candidate as **one git-apply-able unified diff** (paths relative to the repo root, standard `a/` and `b/` prefixes) inside a fenced block delimited exactly:
    ```dream-patch
    <the full diff>
    ```
    Emit exactly one such block. An ACCEPT without it is vetoed by the engine: a finding with no code change is a REJECT or INCONCLUSIVE, stated plainly. The diff must not delete a binary file; the engine refuses such a patch before applying it.
12. Include ONE proposed ledger row in the report, in exactly this shape (the engine fills the issue, PR, witness, prior-night-fate and reviewer columns from its own records — leave them as shown):
    `| {{date_iso}} | {{deep}} | <finding> | NONE | NONE | yes | <VERDICT> | <effect> |  |  |  |  |`
    The **finding** MUST be a concrete, self-contained statement of at most 80 characters saying what tonight established — never `see report` or any bare pointer. The ledger is the only cross-night memory; a row that points elsewhere is a lost night. For an INCONCLUSIVE night, name the blocker itself (for example `perf deep has no evaluator`, `source for src/x.rs:200-400 not provided`) so streak and duplicate-direction signals can read it and stop retrying a dead end.
13. If an ADR is warranted, propose it ({{adr_example}} convention) under **Human action recommended** — you cannot file it.

## Stop conditions
- Every REQUIRED evaluator blocked or silent in the receipts: INCONCLUSIVE, naming the blocker.
- A candidate that fails the security step: REJECT.
- Never fabricate a receipt, a command result, a witness or forge state.

## VERDICT (final line of report)
The very last line of your report MUST be exactly one of:
```
VERDICT: ACCEPT
VERDICT: REJECT
VERDICT: INCONCLUSIVE
```

## FINAL REPORT
End with a structured summary block containing at minimum:
- Date, deep, scans, commit
- Finding, hypothesis, verdict, predicted effect
- Receipts and source files cited; files the candidate changes
- Main lesson, biggest uncertainty, next steps
- Human action recommended
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> DreamConfig {
        DreamConfig::from_json(
            br#"{
              "repo": "example/test",
              "cron": "0 3 * * *",
              "slots": [
                {"deep": "compiler-parity", "scan": ["config-schema", "golden-snapshots"]},
                {"deep": "ledger-signals", "scan": ["witness"]}
              ],
              "buildStep": {"cmd": "cargo build"},
              "evaluatorEntrypoints": {"bench": "cargo test"},
              "competitors": ["Rival Tool"],
              "adrConvention": "4-digit",
              "ledgerPath": "docs/dream-cycle/LEDGER.md",
              "branchPrefix": "dream/",
              "labels": ["dream-cycle"]
            }"#,
        )
        .unwrap()
    }

    fn prompt(cfg: &DreamConfig) -> String {
        compile(
            cfg,
            &cfg.slots[0],
            20260815,
            &[],
            &PromptTemplate::builtin(),
        )
        .unwrap()
    }

    #[test]
    fn prompt_is_deterministic() {
        let cfg = test_config();
        assert_eq!(prompt(&cfg), prompt(&cfg));
    }

    #[test]
    fn prompt_contains_repo_and_deep() {
        let p = prompt(&test_config());
        assert!(p.contains("example/test"));
        assert!(p.contains("compiler-parity"));
        assert!(p.contains("config-schema, golden-snapshots"));
    }

    #[test]
    fn prompt_contains_evaluator() {
        assert!(prompt(&test_config()).contains("cargo test"));
    }

    #[test]
    fn prompt_contains_verdict_instructions() {
        let p = prompt(&test_config());
        assert!(p.contains("VERDICT: ACCEPT"));
        assert!(p.contains("VERDICT: REJECT"));
        assert!(p.contains("VERDICT: INCONCLUSIVE"));
    }

    #[test]
    fn human_review_policy_when_no_auto_merge() {
        assert!(prompt(&test_config()).contains("Human-review-only"));
    }

    #[test]
    fn prompt_assigns_side_effects_to_the_engine() {
        let p = prompt(&test_config());
        assert!(p.contains("single model call"));
        assert!(p.contains("Never claim to have run"));
        for banned in [
            "Publish gist",
            "Create issue",
            "Append one ledger row",
            "Run every evaluator entrypoint",
        ] {
            assert!(
                !p.contains(banned),
                "prompt still asks the model to: {banned}"
            );
        }
        assert!(p.contains(
            "the engine fills the issue, PR, witness, prior-night-fate and reviewer columns"
        ));
    }

    #[test]
    fn proposed_ledger_row_parses_as_a_ledger_row() {
        let p = prompt(&test_config());
        let row = p
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("`| 2026-08-15 | compiler-parity |"))
            .expect("row template present")
            .trim_matches('`');
        assert!(row.split('|').count() >= 12, "{row}");
        assert_eq!(date_iso(20260815), "2026-08-15");
        assert_eq!(date_iso(7), "7");
    }

    #[test]
    fn prompt_forbids_accept_without_patch() {
        let p = prompt(&test_config());
        assert!(p.contains("An ACCEPT without it is vetoed by the engine"));
        assert!(p.contains("Never write a hunk inside an elided region"));
        assert!(p.contains("```dream-patch"));
        assert!(p.contains("draft"));
    }

    #[test]
    fn prompt_forbids_opaque_findings() {
        let p = prompt(&test_config());
        assert!(p.contains("see report"));
        assert!(p.contains("name the blocker itself"));
    }

    #[test]
    fn builtin_template_uses_only_known_placeholders() {
        PromptTemplate::builtin().validate().unwrap();
    }

    #[test]
    fn unknown_and_unterminated_placeholders_are_errors() {
        let cfg = test_config();
        let s = &cfg.slots[0];
        assert_eq!(
            compile(&cfg, s, 1, &[], &PromptTemplate::new("x {{nope}}")),
            Err(TemplateError::UnknownPlaceholder("nope".into()))
        );
        assert_eq!(
            compile(&cfg, s, 1, &[], &PromptTemplate::new("x {{repo")),
            Err(TemplateError::Unterminated)
        );
    }

    #[test]
    fn substitution_is_single_pass() {
        let cfg = DreamConfig::from_json(
            br#"{"repo": "o/r", "slots": [{"deep": "{{repo}}", "scan": []}]}"#,
        )
        .unwrap();
        let out = compile(
            &cfg,
            &cfg.slots[0],
            1,
            &[],
            &PromptTemplate::new("{{deep}}|{{repo}}"),
        )
        .unwrap();
        assert_eq!(out, "{{repo}}|o/r");
    }

    #[test]
    fn bonus_dives_are_numbered() {
        let cfg = test_config();
        let out = compile(
            &cfg,
            &cfg.slots[0],
            1,
            &["first".into(), "second".into()],
            &PromptTemplate::new("{{bonus_section}}"),
        )
        .unwrap();
        assert!(out.contains("1. first\n2. second"));
    }
}
