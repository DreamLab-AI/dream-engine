//! Reading side of the ledger: parsing, verification, contract validation and
//! the deterministic learning signals.
//!
//! This is a port of the behaviour of the reference TypeScript ledger package
//! (`parseLedger`, `verifyLedger`, `validateLedger`, `learningSignals`,
//! `verdictStats`), including its escaped-pipe splitting and its (deliberately
//! lenient) prior-fate reader. Divergences are listed on each function.
//!
//! # Example
//!
//! ```
//! use dream_core::ledger::{empty_ledger, parse_ledger, verify_ledger, LedgerRow, render_row};
//!
//! let row = LedgerRow::unreviewed(
//!     "2026-09-07".into(), "perf".into(), "cut cold start by 40ms".into(), "NONE".into(),
//!     "#12".into(), "yes".into(), "ACCEPT".into(), "bench -40ms".into(), "ab12cd34ef56".into(), "".into(),
//! );
//! let md = format!("{}{}\n", empty_ledger(), render_row(&row));
//! assert!(verify_ledger(&md).ok);
//! assert_eq!(parse_ledger(&md).rows[0].deep, "perf");
//! ```

use std::collections::{BTreeMap, BTreeSet};

use super::{LedgerRow, LEDGER_VERDICTS};

/// Evaluated-column vocabulary. `n/a` belongs to rows that are not nights.
pub const LEDGER_EVALUATED: [&str; 4] = ["yes", "no", "blocked", "n/a"];

/// The width a ledger row is written at.
const WIDTH: usize = 12;
/// The legacy width, still valid on read.
const LEGACY_WIDTH: usize = 10;

/// Split a table line into trimmed cells, honouring `\|` escapes.
///
/// Mirrors the TS `splitRow`: trim the line, drop one leading and one
/// trailing `|`, protect escaped pipes, split on the rest.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.replace("\\|", "\u{0}")
        .split('|')
        .map(|c| c.trim().replace('\u{0}', "|"))
        .collect()
}

fn is_divider(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c: String = c.chars().filter(|ch| !ch.is_whitespace()).collect();
            let c = c.strip_prefix(':').unwrap_or(&c);
            let c = c.strip_suffix(':').unwrap_or(c);
            c.len() >= 3 && c.bytes().all(|b| b == b'-')
        })
}

fn cells_to_row(cells: &[String]) -> LedgerRow {
    let at = |i: usize| cells.get(i).cloned().unwrap_or_default();
    LedgerRow {
        date: at(0),
        deep: at(1),
        finding: at(2),
        issue: at(3),
        pr: at(4),
        evaluated: at(5),
        verdict: at(6),
        effect: at(7),
        witness: at(8),
        prior_fates: at(9),
        reviewer: at(10),
        review_minutes: at(11),
    }
}

/// Parsed rows plus non-fatal warnings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParseResult {
    /// Data rows, in file order.
    pub rows: Vec<LedgerRow>,
    /// Malformed rows and unexpected headers; the rows are still returned.
    pub warnings: Vec<String>,
}

/// Parse a ledger body into rows, skipping blank lines, lines without a pipe,
/// the header and the divider.
///
/// **Divergence from the TS parser:** it warns on any row that is not 10
/// cells wide, so every row this crate writes (12 wide) would warn there. Here
/// 10 (legacy) and 12 are both silent; any other width warns. Missing cells
/// read as empty, extra cells are dropped.
pub fn parse_ledger(markdown: &str) -> ParseResult {
    let mut out = ParseResult::default();
    let mut seen_header = false;
    for line in markdown.lines() {
        if line.trim().is_empty() || !line.contains('|') {
            continue;
        }
        let cells = split_row(line);
        if is_divider(&cells) {
            continue;
        }
        if !seen_header {
            seen_header = true;
            if !cells[0].eq_ignore_ascii_case("date") {
                out.warnings.push(format!(
                    "first table row is not the expected header (got \"{}\")",
                    cells[0]
                ));
            }
            continue;
        }
        if cells.len() != WIDTH && cells.len() != LEGACY_WIDTH {
            out.warnings.push(format!(
                "row has {} columns, expected {WIDTH} (or legacy {LEGACY_WIDTH}): {}",
                cells.len(),
                line.trim()
            ));
        }
        out.rows.push(cells_to_row(&cells));
    }
    out
}

/// Outcome of [`verify_ledger`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    /// True when `errors` is empty.
    pub ok: bool,
    /// Structural errors, `row N: …` (1-based among data rows).
    pub errors: Vec<String>,
    /// Parser warnings.
    pub warnings: Vec<String>,
    /// Number of data rows.
    pub row_count: usize,
}

fn has_date_header(md: &str) -> bool {
    // /\|\s*Date\s*\|/ (case-sensitive, as in the TS verifier).
    let b = md.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'|' {
            let rest = md[i + 1..].trim_start();
            if let Some(after) = rest.strip_prefix("Date") {
                if after.trim_start().starts_with('|') {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Structurally verify a ledger: a `Date` header is present and every row's
/// verdict, evaluated flag and date are in range. Unlike [`validate_ledger`]
/// it checks grandfathered rows too, and it reads empty cells as absent
/// (`r.verdict && …` in the TS), so an empty verdict is not an error here.
pub fn verify_ledger(markdown: &str) -> VerifyResult {
    let mut errors = Vec::new();
    let parsed = parse_ledger(markdown);
    if !has_date_header(markdown) {
        errors.push("ledger is missing the Date header row".to_string());
    }
    for (i, r) in parsed.rows.iter().enumerate() {
        let n = i + 1;
        if !r.verdict.is_empty() && !LEDGER_VERDICTS.contains(&r.verdict.as_str()) {
            errors.push(format!(
                "row {n}: verdict \"{}\" not in {}",
                r.verdict,
                LEDGER_VERDICTS.join("|")
            ));
        }
        if !r.evaluated.is_empty() && !LEDGER_EVALUATED.contains(&r.evaluated.as_str()) {
            errors.push(format!(
                "row {n}: evaluated \"{}\" not in {}",
                r.evaluated,
                LEDGER_EVALUATED.join("|")
            ));
        }
        if !r.date.is_empty() && !super::iso_date(&r.date) {
            errors.push(format!("row {n}: date \"{}\" is not YYYY-MM-DD", r.date));
        }
    }
    VerifyResult {
        ok: errors.is_empty(),
        errors,
        warnings: parsed.warnings,
        row_count: parsed.rows.len(),
    }
}

/// One row-contract violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowViolation {
    /// The row's date cell.
    pub date: String,
    /// The rule broken, named as the TS contract names it.
    pub rule: &'static str,
}

/// The TS `parseRow`: a line starting with `|` that splits into at least 12
/// raw parts (ten cells plus the two outer empties). Returns the first twelve
/// cells at most; narrower legacy rows yield ten.
fn parse_contract_row(line: &str) -> Option<LedgerRow> {
    let t = line.trim();
    if !t.starts_with('|') {
        return None;
    }
    let raw: Vec<String> = t
        .replace("\\|", "\u{0}")
        .split('|')
        .map(|c| c.trim().replace('\u{0}', "|"))
        .collect();
    if raw.len() < 12 {
        return None;
    }
    let inner = &raw[1..raw.len() - 1];
    Some(cells_to_row(inner))
}

/// Validate every row of a ledger body against the row contract, from
/// `enforce_from` (a `YYYY-MM-DD` date; earlier rows are grandfathered).
///
/// Port of the TS `validateLedger`. **Divergence:** violations carry the rule
/// name and the date but not the free-text `detail` the TS adds; the rule set
/// and the rows flagged are the same.
pub fn validate_ledger(markdown: &str, enforce_from: &str) -> Vec<RowViolation> {
    let mut out = Vec::new();
    for line in markdown.lines() {
        if let Some(row) = parse_contract_row(line) {
            let date = row.date.trim().to_string();
            if !super::iso_date(&date) || date.as_str() < enforce_from {
                continue;
            }
            for rule in super::row_violations_from(&row, enforce_from) {
                out.push(RowViolation {
                    date: date.clone(),
                    rule,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Learning signals
// ---------------------------------------------------------------------------

/// Options for [`learning_signals`].
#[derive(Debug, Clone, PartialEq)]
pub struct SignalOptions {
    /// Trailing nights considered for the merge and blocked streaks.
    pub window: usize,
    /// Recent self-scores (0–10), most recent last.
    pub recent_scores: Vec<f64>,
    /// PR numbers known to have merged (for example from a forge lookup).
    pub merged_pr_numbers: BTreeSet<String>,
}

impl Default for SignalOptions {
    fn default() -> Self {
        Self {
            window: 14,
            recent_scores: Vec::new(),
            merged_pr_numbers: BTreeSet::new(),
        }
    }
}

/// Deterministic steering signals derived from the ledger history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningSignals {
    /// No PR in the window is known merged: prefer tiny candidates.
    pub zero_merge_streak: bool,
    /// Normalised finding keys seen at least three times: rotate away.
    pub duplicate_directions: Vec<String>,
    /// The last three scores were all below 5.
    pub low_score_streak: bool,
    /// The last three nights all had `evaluated = blocked`.
    pub blocked_eval_streak: bool,
    /// Nights inside the window.
    pub nights_considered: usize,
}

/// First maximal run of ASCII digits in `s` (`/#?(\d+)/`).
fn pr_number(pr: &str) -> Option<String> {
    let start = pr.find(|c: char| c.is_ascii_digit())?;
    let digits: String = pr[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some(digits)
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Read `#<PR>:<FATE>` tokens out of the "Prior-night fates" column.
///
/// Lenient, as in the TS: case-insensitive, whitespace allowed around the
/// colon, and the fate must end at a word boundary. This is deliberately a
/// different grammar from the strict row contract (`fate_token`), which is why
/// a lenient reader and a strict writer coexist. The latest row wins. The
/// whitespace class is [`char::is_whitespace`]; JS `\s` additionally matches
/// U+FEFF.
pub fn parse_prior_fates(rows: &[LedgerRow]) -> BTreeMap<String, String> {
    let mut fates = BTreeMap::new();
    for r in rows {
        let chars: Vec<char> = r.prior_fates.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] != '#' {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            let ds = j;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j == ds {
                i += 1;
                continue;
            }
            let num: String = chars[ds..j].iter().collect();
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j >= chars.len() || chars[j] != ':' {
                i += 1;
                continue;
            }
            j += 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let rest: String = chars[j..].iter().take(6).collect::<String>().to_uppercase();
            let fate = ["MERGED", "CLOSED", "OPEN", "STALE"]
                .into_iter()
                .find(|f| rest.starts_with(f));
            match fate {
                Some(f) if !chars.get(j + f.len()).copied().is_some_and(is_word) => {
                    fates.insert(num, f.to_string());
                    i = j + f.len();
                }
                _ => i += 1,
            }
        }
    }
    fates
}

/// The TS duplicate-direction key: lowercase, keep `[a-z0-9 ]`, split on
/// whitespace runs **keeping a leading empty token** (JS `split(/\s+/)`), take
/// six tokens, join, trim.
fn direction_key(finding: &str) -> String {
    let cleaned: String = finding
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == ' ')
        .collect();
    let mut tokens: Vec<&str> = Vec::new();
    let mut start = 0;
    let b = cleaned.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b' ' {
            tokens.push(&cleaned[start..i]);
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            start = i;
        } else {
            i += 1;
        }
    }
    tokens.push(&cleaned[start..]);
    tokens
        .into_iter()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Compute the learning signals from parsed rows.
///
/// Merge status comes from `opts.merged_pr_numbers` and from `#N:MERGED`
/// tokens already in the ledger. Duplicate directions are counted over **all**
/// rows (first-seen order); the other signals use the trailing window.
pub fn learning_signals(rows: &[LedgerRow], opts: &SignalOptions) -> LearningSignals {
    let recent = &rows[rows.len().saturating_sub(opts.window)..];

    let mut merged = opts.merged_pr_numbers.clone();
    merged.extend(
        parse_prior_fates(rows)
            .into_iter()
            .filter(|(_, f)| f == "MERGED")
            .map(|(n, _)| n),
    );
    let prs: Vec<String> = recent.iter().filter_map(|r| pr_number(&r.pr)).collect();
    let zero_merge_streak = !prs.is_empty() && prs.iter().all(|n| !merged.contains(n));

    let mut counts: Vec<(String, usize)> = Vec::new();
    for r in rows {
        let key = direction_key(&r.finding);
        if key.is_empty() {
            continue;
        }
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, c)) => *c += 1,
            None => counts.push((key, 1)),
        }
    }
    let duplicate_directions = counts
        .into_iter()
        .filter(|(_, c)| *c >= 3)
        .map(|(k, _)| k)
        .collect();

    let s = &opts.recent_scores;
    let last3 = &s[s.len().saturating_sub(3)..];
    let low_score_streak = last3.len() == 3 && last3.iter().all(|x| *x < 5.0);

    let last3_eval = &recent[recent.len().saturating_sub(3)..];
    let blocked_eval_streak = recent.len() >= 3
        && last3_eval.len() == 3
        && last3_eval.iter().all(|r| r.evaluated == "blocked");

    LearningSignals {
        zero_merge_streak,
        duplicate_directions,
        low_score_streak,
        blocked_eval_streak,
        nights_considered: recent.len(),
    }
}

/// Verdict distribution over a ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VerdictStats {
    /// `ACCEPT` rows.
    pub accept: usize,
    /// `REJECT` rows.
    pub reject: usize,
    /// `INCONCLUSIVE` rows.
    pub inconclusive: usize,
    /// Every other verdict token.
    pub other: usize,
}

/// Count verdicts. Matches the TS `verdictStats` (`ACCEPT`, `REJECT`,
/// `INCONCLUSIVE`, everything else in `other`).
pub fn verdict_stats(rows: &[LedgerRow]) -> VerdictStats {
    let mut s = VerdictStats::default();
    for r in rows {
        match r.verdict.as_str() {
            "ACCEPT" => s.accept += 1,
            "REJECT" => s.reject += 1,
            "INCONCLUSIVE" => s.inconclusive += 1,
            _ => s.other += 1,
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn row(f: impl FnOnce(&mut LedgerRow)) -> LedgerRow {
        let mut r = LedgerRow::unreviewed(
            "2026-08-13".into(),
            "deep".into(),
            "a concrete finding".into(),
            "#1".into(),
            "#2".into(),
            "yes".into(),
            "ACCEPT".into(),
            "tests 85->96".into(),
            "ec2052aa".into(),
            "first night".into(),
        );
        f(&mut r);
        r
    }

    fn append(md: &str, r: &LedgerRow) -> String {
        let base = if md.trim().is_empty() {
            empty_ledger()
        } else {
            md.trim_end().to_string() + "\n"
        };
        format!("{base}{}\n", render_row(r))
    }

    #[test]
    fn append_then_parse_recovers_the_row() {
        let l = append(&empty_ledger(), &row(|_| {}));
        let p = parse_ledger(&l);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.rows[0].verdict, "ACCEPT");
        assert_eq!(p.rows[0].prior_fates, "first night");
    }

    #[test]
    fn escaped_pipe_round_trips_without_shifting_later_cells() {
        let l = append(
            &empty_ledger(),
            &row(|r| r.finding = "found a | b drift".into()),
        );
        let p = parse_ledger(&l);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.rows[0].finding, "found a | b drift");
        assert_eq!(p.rows[0].verdict, "ACCEPT");
        assert!(verify_ledger(&l).ok);
    }

    #[test]
    fn newlines_in_a_cell_collapse_to_one_space() {
        let l = append(
            &empty_ledger(),
            &row(|r| r.finding = "a | b\r\nc\nd".into()),
        );
        assert_eq!(parse_ledger(&l).rows.len(), 1);
        assert_eq!(parse_ledger(&l).rows[0].finding, "a | b c d");
    }

    #[test]
    fn legacy_ten_column_rows_parse_with_empty_review_columns() {
        let l = "| Date | Deep | Finding | Issue | PR | Evaluated? | Verdict | Effect | Witness | Prior-night fates |\n\
                 | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n\
                 | 2026-08-13 | d | f | #1 | #2 | yes | ACCEPT | e | w | |\n";
        let p = parse_ledger(l);
        assert_eq!(p.rows.len(), 1);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.rows[0].reviewer, "");
    }

    #[test]
    fn odd_column_counts_warn_but_still_yield_a_row() {
        let l = "| Date | Deep |\n| --- | --- |\n| 2026-08-13 | d |\n";
        let p = parse_ledger(l);
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.warnings.len(), 1);
    }

    #[test]
    fn verify_accepts_a_valid_ledger_and_counts_rows() {
        let mut l = empty_ledger();
        l = append(&l, &row(|_| {}));
        l = append(
            &l,
            &row(|r| {
                r.date = "2026-08-14".into();
                r.verdict = "REJECT".into();
            }),
        );
        let v = verify_ledger(&l);
        assert!(v.ok, "{:?}", v.errors);
        assert_eq!(v.row_count, 2);
    }

    #[test]
    fn verify_flags_out_of_range_verdict_date_and_evaluated() {
        let v = verify_ledger(&append(
            &empty_ledger(),
            &row(|r| r.verdict = "MAYBE".into()),
        ));
        assert!(!v.ok);
        assert!(v.errors.join(",").contains("verdict"));
        let v = verify_ledger(&append(
            &empty_ledger(),
            &row(|r| {
                r.date = "yesterday".into();
                r.evaluated = "perhaps".into();
            }),
        ));
        let e = v.errors.join(",");
        assert!(e.contains("date") && e.contains("evaluated"), "{e}");
    }

    #[test]
    fn verify_accepts_non_night_verdicts_and_na_evaluated() {
        let mut l = empty_ledger();
        for (d, v, e) in [
            ("2026-09-07", "OPERATOR", "n/a"),
            ("2026-09-08", "HANDOFF", "n/a"),
            ("2026-09-09", "BLOCKED-ENV", "blocked"),
        ] {
            l = append(
                &l,
                &row(|r| {
                    r.date = d.into();
                    r.verdict = v.into();
                    r.evaluated = e.into();
                }),
            );
        }
        let v = verify_ledger(&l);
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(v.row_count, 3);
    }

    #[test]
    fn verify_flags_a_missing_header() {
        let v = verify_ledger("just some text, no table");
        assert!(!v.ok);
        assert!(v.errors.join(",").contains("header"));
    }

    fn many(n: usize, f: impl Fn(usize, &mut LedgerRow)) -> Vec<LedgerRow> {
        (0..n).map(|i| row(|r| f(i, r))).collect()
    }

    #[test]
    fn zero_merge_streak_when_no_pr_is_known_merged() {
        let rows = many(14, |i, r| r.pr = format!("#{}", 100 + i));
        let s = learning_signals(&rows, &SignalOptions::default());
        assert!(s.zero_merge_streak);
        let opts = SignalOptions {
            merged_pr_numbers: ["105".to_string()].into(),
            ..Default::default()
        };
        assert!(!learning_signals(&rows, &opts).zero_merge_streak);
    }

    #[test]
    fn merged_fate_token_in_the_ledger_clears_the_streak() {
        let mut rows = many(13, |i, r| {
            r.pr = format!("#{}", 100 + i);
            r.prior_fates = "no news".into();
        });
        rows.push(row(|r| {
            r.pr = "#113".into();
            r.prior_fates = "checked last week: #105:MERGED, #108:CLOSED".into();
        }));
        assert!(!learning_signals(&rows, &SignalOptions::default()).zero_merge_streak);
    }

    #[test]
    fn closed_open_stale_and_prose_do_not_clear_the_streak() {
        let rows = many(14, |i, r| {
            r.pr = format!("#{}", 100 + i);
            r.prior_fates = format!("#{}:CLOSED", 99 + i);
        });
        assert!(learning_signals(&rows, &SignalOptions::default()).zero_merge_streak);
        let rows = many(14, |i, r| {
            r.pr = format!("#{}", 100 + i);
            r.prior_fates = "PR 105 was merged into main yesterday".into();
        });
        assert!(learning_signals(&rows, &SignalOptions::default()).zero_merge_streak);
    }

    #[test]
    fn prior_fates_are_lenient_and_the_latest_wins() {
        let rows = vec![
            row(|r| r.prior_fates = "#105:OPEN".into()),
            row(|r| r.prior_fates = "#105 : merged".into()),
            row(|r| r.prior_fates = "#7:MERGEDX #8:STALE".into()),
        ];
        let f = parse_prior_fates(&rows);
        assert_eq!(f.get("105").map(String::as_str), Some("MERGED"));
        assert!(!f.contains_key("7"), "\\b must reject MERGEDX");
        assert_eq!(f.get("8").map(String::as_str), Some("STALE"));
    }

    #[test]
    fn duplicate_directions_need_three_repeats_and_use_the_js_six_token_key() {
        let mut rows = many(3, |_, r| {
            r.finding = "improve router calibration loop".into()
        });
        rows.push(row(|r| r.finding = "totally different thing here".into()));
        let s = learning_signals(&rows, &SignalOptions::default());
        assert_eq!(
            s.duplicate_directions,
            vec!["improve router calibration loop"]
        );
        // Punctuation-led findings leave a leading space; JS split(/\s+/) then
        // yields an empty first token that consumes one of the six slots.
        let rows = many(3, |i, r| {
            r.finding = format!("- a b c d e f{i}");
        });
        let s = learning_signals(&rows, &SignalOptions::default());
        assert_eq!(s.duplicate_directions, vec!["a b c d e"]);
    }

    #[test]
    fn low_score_and_blocked_eval_streaks() {
        let rows = many(1, |_, _| {});
        let o = |v: &[f64]| SignalOptions {
            recent_scores: v.to_vec(),
            ..Default::default()
        };
        assert!(learning_signals(&rows, &o(&[7.0, 4.0, 3.0, 2.0])).low_score_streak);
        assert!(!learning_signals(&rows, &o(&[7.0, 4.0, 6.0])).low_score_streak);
        let rows = many(5, |_, r| {
            r.evaluated = "blocked".into();
            r.verdict = "INCONCLUSIVE".into();
        });
        assert!(learning_signals(&rows, &SignalOptions::default()).blocked_eval_streak);
        assert_eq!(
            learning_signals(&rows, &SignalOptions::default()).nights_considered,
            5
        );
    }

    #[test]
    fn verdict_stats_counts_and_buckets_other() {
        let mut rows = vec![];
        for v in [
            "ACCEPT",
            "REJECT",
            "INCONCLUSIVE",
            "INCONCLUSIVE",
            "HANDOFF",
        ] {
            rows.push(row(|r| r.verdict = v.into()));
        }
        let s = verdict_stats(&rows);
        assert_eq!((s.accept, s.reject, s.inconclusive, s.other), (1, 1, 2, 1));
    }

    #[test]
    fn validate_ledger_mirrors_the_typescript_row_contract() {
        let good = render_row(&row(|r| {
            r.date = "2026-09-07".into();
            r.prior_fates = "#1:MERGED #2:OPEN".into();
        }));
        assert!(validate_ledger(&good, CONTRACT_ENFORCE_FROM).is_empty());
        // Escaped pipe keeps alignment.
        let piped = render_row(&row(|r| {
            r.date = "2026-09-07".into();
            r.prior_fates = String::new();
            r.finding = "found a | b drift".into();
        }));
        assert!(validate_ledger(&piped, CONTRACT_ENFORCE_FROM).is_empty());
        // Grandfathered.
        let old = render_row(&row(|r| r.finding = "see report".into()));
        assert!(validate_ledger(&old, CONTRACT_ENFORCE_FROM).is_empty());
        // Header and divider lines are not flagged.
        assert!(validate_ledger(&empty_ledger(), CONTRACT_ENFORCE_FROM).is_empty());
        // Violations carry the date and the rule.
        let bad = render_row(&row(|r| {
            r.date = "2026-09-07".into();
            r.verdict = "ACCEPT".into();
            r.pr = "NONE".into();
            r.prior_fates = String::new();
            r.witness = String::new();
        }));
        let v = validate_ledger(&bad, CONTRACT_ENFORCE_FROM);
        let rules: Vec<_> = v.iter().map(|x| x.rule).collect();
        assert_eq!(rules, vec!["accept-without-pr", "accept-without-witness"]);
        assert!(v.iter().all(|x| x.date == "2026-09-07"));
        // Rows with fewer than ten cells are not rows.
        assert!(validate_ledger("| 2026-09-07 | x |", CONTRACT_ENFORCE_FROM).is_empty());
    }
}
