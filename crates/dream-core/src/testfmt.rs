//! Per-test outcomes from evaluator output.
//!
//! An evaluator normally yields one receipt (pass or fail). When it is
//! declared with a [`TestFormat`], this module reads the individual tests out
//! of its output into [`dream_gate::pertest::TestOutcomes`], so the gate can
//! see what one exit code hides: a test that used to pass and now fails, or a
//! *new* failure in an already-red suite.
//!
//! | format | reads | notes |
//! |---|---|---|
//! | [`TestFormat::Libtest`] | `cargo test` text (`test name ... ok`) | pretty format only; use `2>&1` so the `Running` banners that label each target are in the same stream |
//! | [`TestFormat::Junit`] | JUnit XML | `quick-xml`; DOCTYPE entities are never expanded |
//! | [`TestFormat::VitestJson`] | Vitest/Jest `--reporter=json` | pass the work directory so file paths become repo-relative |
//!
//! With **no** format an evaluator counts as one case; nothing here runs.
//! Cases that were skipped or ignored are not cases. A format that finds no
//! tests is an error ([`TestFmtError::NoTests`]), never an empty success, and
//! [`outcomes`] then returns `None` rather than fabricate evidence: the
//! receipt alone judges that evaluator, exactly as before.
//!
//! **Truncation:** an evaluator piped through `| tail -N` loses the head of its
//! output, and with it most test lines. Declare a format only on evaluators
//! whose full output reaches this module (and give the pipe `pipefail`).
//! Truncation shows up as baseline tests *missing* from the candidate run, a
//! harness fault, not a loss.
//!
//! # Example
//!
//! ```
//! use dream_core::testfmt::parse_libtest;
//! use dream_gate::pertest::TestStatus;
//!
//! let out = "running 2 tests\ntest math::adds ... ok\ntest math::subs ... FAILED\n\nfailures:\n\n---- math::subs stdout ----\nthread 'math::subs' panicked at src/m.rs:3:5:\nassertion failed: 1 == 2\n\nfailures:\n    math::subs\n";
//! let cases = parse_libtest(out)?;
//! assert_eq!(cases[0].status, TestStatus::Passed);
//! assert_eq!(cases[1].status, TestStatus::failed("assertion failed: 1 == 2"));
//! # Ok::<(), dream_core::testfmt::TestFmtError>(())
//! ```

use std::collections::HashMap;

use dream_gate::config::TestFormat;
use dream_gate::outcome::ExecOutcome;
use dream_gate::pertest::{TestCase, TestOutcomes, TestStatus};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Deserialize;

/// Why no per-test outcomes could be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TestFmtError {
    /// The output parsed but contained no test cases (a compile error, a
    /// terse run, an empty report).
    #[error("no test cases found in the output")]
    NoTests,
    /// The output is not valid in the declared format.
    #[error("malformed {0}")]
    Malformed(String),
}

/// Longest cause kept per failed test.
const CAUSE_CAP: usize = 2000;

fn clip(mut s: String) -> String {
    if s.len() > CAUSE_CAP {
        let mut n = CAUSE_CAP;
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
    }
    s
}

/// Make names unique and deterministic: the second `x` becomes `x~2`.
fn dedup(cases: &mut [TestCase]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for c in cases.iter_mut() {
        let n = seen.entry(c.name.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            c.name = format!("{}~{}", c.name, n);
        }
    }
}

// ---------------------------------------------------------------------------
// libtest
// ---------------------------------------------------------------------------

/// `Running unittests src/lib.rs (…)` → `unittests src/lib.rs`;
/// `Doc-tests name` → `Doc-tests name`.
fn libtest_label(line: &str) -> Option<String> {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("Running ") {
        let label = rest.rsplit_once(" (").map_or(rest, |(l, _)| l);
        return Some(label.trim().to_string());
    }
    t.strip_prefix("Doc-tests ")
        .map(|c| format!("Doc-tests {}", c.trim()))
}

/// Drop a trailing ` (line N)` from a doc-test name.
fn strip_doc_line(name: &str) -> &str {
    if let Some(head) = name.strip_suffix(')') {
        if let Some((h, tail)) = head.rsplit_once(" (line ") {
            if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
                return h;
            }
        }
    }
    name
}

/// The panic message of one `---- name stdout ----` block.
fn panic_cause(block: &[&str]) -> String {
    for (i, l) in block.iter().enumerate() {
        let Some(at) = l.find("panicked at ") else {
            continue;
        };
        let after = &l[at + "panicked at ".len()..];
        // Old format: panicked at 'msg', file:line:col
        if let Some(rest) = after.strip_prefix('\'') {
            if let Some((msg, _)) = rest.rsplit_once("', ") {
                return clip(msg.to_string());
            }
        }
        // New format: location line, then the message lines.
        let msg: Vec<&str> = block[i + 1..]
            .iter()
            .map(|m| m.trim())
            .take_while(|m| {
                !m.is_empty()
                    && !m.starts_with("stack backtrace")
                    && !m.starts_with("note: run with")
            })
            .collect();
        if !msg.is_empty() {
            return clip(msg.join(" "));
        }
        return clip(after.to_string());
    }
    "test failed".to_string()
}

fn parse_libtest_section(label: Option<&str>, lines: &[&str], out: &mut Vec<TestCase>) {
    // Failure blocks first, keyed by normalised test name.
    let mut causes: HashMap<String, String> = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if let Some(name) = l
            .strip_prefix("---- ")
            .and_then(|r| r.strip_suffix(" stdout ----"))
        {
            let start = i + 1;
            let mut end = start;
            while end < lines.len() && !lines[end].starts_with("---- ") && lines[end] != "failures:"
            {
                end += 1;
            }
            causes.insert(
                strip_doc_line(name).to_string(),
                panic_cause(&lines[start..end]),
            );
            i = end;
        } else {
            i += 1;
        }
    }
    for l in lines {
        let Some(rest) = l.strip_prefix("test ") else {
            continue;
        };
        let Some((name, status)) = rest.rsplit_once(" ... ") else {
            continue;
        };
        let name = name.strip_suffix(" - should panic").unwrap_or(name);
        let name = strip_doc_line(name);
        let status = status.trim();
        let st = if status == "ok" {
            TestStatus::Passed
        } else if status == "FAILED" {
            TestStatus::failed(
                causes
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| "test failed".to_string()),
            )
        } else {
            // ignored, ignored with a reason, bench: …
            continue;
        };
        let full = match label {
            Some(lb) => format!("{lb}::{name}"),
            None => name.to_string(),
        };
        out.push(TestCase::new(full, st));
    }
}

/// Parse `cargo test` text output.
///
/// Each target's tests are named `"<target>::<test>"`, where the target is the
/// text of its `Running …` or `Doc-tests …` banner without the build-hash path
/// (so names are stable between a baseline and a candidate tree). Tests with
/// no preceding banner keep bare names. A doc-test's `(line N)` is dropped so a
/// shifted line does not rename it. `should_panic` tests that panic are passes;
/// ignored tests are not cases. A failed test's cause is its panic message.
pub fn parse_libtest(text: &str) -> Result<Vec<TestCase>, TestFmtError> {
    let all: Vec<&str> = text.lines().collect();
    let mut cases = Vec::new();
    let mut label: Option<String> = None;
    let mut start = 0;
    for (i, l) in all.iter().enumerate() {
        if let Some(next) = libtest_label(l) {
            parse_libtest_section(label.as_deref(), &all[start..i], &mut cases);
            label = Some(next);
            start = i + 1;
        }
    }
    parse_libtest_section(label.as_deref(), &all[start..], &mut cases);
    if cases.is_empty() {
        return Err(TestFmtError::NoTests);
    }
    dedup(&mut cases);
    Ok(cases)
}

// ---------------------------------------------------------------------------
// vitest / jest json
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VtReport {
    #[serde(default)]
    test_results: Vec<VtFile>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VtFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    assertion_results: Vec<VtTest>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VtTest {
    #[serde(default)]
    full_name: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    failure_messages: Vec<String>,
}

fn first_line(s: &str) -> String {
    clip(
        s.lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string(),
    )
}

fn relative<'a>(path: &'a str, root: Option<&str>) -> &'a str {
    match root {
        Some(r) => path
            .strip_prefix(r.trim_end_matches('/'))
            .map(|p| p.trim_start_matches('/'))
            .unwrap_or(path),
        None => path,
    }
}

/// Parse Vitest (or Jest) `--reporter=json` output.
///
/// A test is named `"<file>::<full name>"`. `root` is the directory the
/// evaluator ran in: it is stripped from file paths so baseline and candidate
/// worktrees name tests identically. Skipped, pending and todo tests are not
/// cases. A test file that failed to load (no assertions, status `failed`) is
/// one failed case named `"<file>::(suite)"`. Text before the first `{`
/// (a banner) is ignored.
pub fn parse_vitest_json(text: &str, root: Option<&str>) -> Result<Vec<TestCase>, TestFmtError> {
    let start = text
        .find('{')
        .ok_or_else(|| TestFmtError::Malformed("vitest json: no JSON object".into()))?;
    let end = text.rfind('}').unwrap_or(start);
    let report: VtReport = serde_json::from_str(&text[start..=end])
        .map_err(|e| TestFmtError::Malformed(format!("vitest json: {e}")))?;
    let mut cases = Vec::new();
    for f in &report.test_results {
        let file = relative(&f.name, root);
        if f.assertion_results.is_empty() && f.status == "failed" {
            cases.push(TestCase::new(
                format!("{file}::(suite)"),
                TestStatus::failed(first_line(&f.message)),
            ));
            continue;
        }
        for t in &f.assertion_results {
            let name = if t.full_name.is_empty() {
                &t.title
            } else {
                &t.full_name
            };
            let st = match t.status.as_str() {
                "passed" => TestStatus::Passed,
                "failed" => TestStatus::failed(first_line(
                    t.failure_messages.first().map_or("", String::as_str),
                )),
                _ => continue,
            };
            cases.push(TestCase::new(format!("{file}::{name}"), st));
        }
    }
    if cases.is_empty() {
        return Err(TestFmtError::NoTests);
    }
    dedup(&mut cases);
    Ok(cases)
}

// ---------------------------------------------------------------------------
// junit xml
// ---------------------------------------------------------------------------

fn attr(e: &quick_xml::events::BytesStart<'_>, key: &str) -> String {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key.as_bytes())
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
        .unwrap_or_default()
}

/// Parse JUnit XML (`<testsuites>` or a bare `<testsuite>`, nested suites
/// allowed).
///
/// A case is named `"<classname>::<name>"` (just `<name>` when the class is
/// empty). A `<failure>` or `<error>` child makes it failed, with the
/// element's `message` attribute (else the first non-blank line of its text,
/// CDATA included) as cause; `<skipped>` removes it. DOCTYPE declarations and
/// entities are never expanded or fetched.
pub fn parse_junit(xml: &str) -> Result<Vec<TestCase>, TestFmtError> {
    let bad = |e: &dyn std::fmt::Display| TestFmtError::Malformed(format!("junit xml: {e}"));
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut cases: Vec<TestCase> = Vec::new();
    // Current case: (name, failed cause, skipped); and the failure element's
    // text buffer when inside one.
    let mut cur: Option<(String, Option<String>, bool)> = None;
    let mut in_fail: Option<(String, String)> = None; // (message attr, text)
    let mut depth_open = 0usize;
    loop {
        match reader.read_event().map_err(|e| bad(&e))? {
            Event::Start(e) => {
                depth_open += 1;
                match e.name().as_ref() {
                    b"testcase" => {
                        let class = attr(&e, "classname");
                        let name = attr(&e, "name");
                        let full = if class.is_empty() {
                            name
                        } else {
                            format!("{class}::{name}")
                        };
                        cur = Some((full, None, false));
                    }
                    b"failure" | b"error" if cur.is_some() => {
                        in_fail = Some((attr(&e, "message"), String::new()));
                    }
                    b"skipped" => {
                        if let Some(c) = cur.as_mut() {
                            c.2 = true;
                        }
                    }
                    _ => {}
                }
            }
            Event::Empty(e) => match e.name().as_ref() {
                b"testcase" => {
                    let class = attr(&e, "classname");
                    let name = attr(&e, "name");
                    let full = if class.is_empty() {
                        name
                    } else {
                        format!("{class}::{name}")
                    };
                    cases.push(TestCase::new(full, TestStatus::Passed));
                }
                b"failure" | b"error" => {
                    if let Some(c) = cur.as_mut() {
                        let m = attr(&e, "message");
                        c.1 = Some(if m.is_empty() {
                            "test failed".into()
                        } else {
                            clip(m)
                        });
                    }
                }
                b"skipped" => {
                    if let Some(c) = cur.as_mut() {
                        c.2 = true;
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                if let Some((_, buf)) = in_fail.as_mut() {
                    buf.push_str(&t.unescape().map_err(|e| bad(&e))?);
                }
            }
            Event::CData(t) => {
                if let Some((_, buf)) = in_fail.as_mut() {
                    buf.push_str(&String::from_utf8_lossy(&t.into_inner()));
                }
            }
            Event::End(e) => {
                depth_open = depth_open.saturating_sub(1);
                match e.name().as_ref() {
                    b"failure" | b"error" => {
                        if let (Some((msg, text)), Some(c)) = (in_fail.take(), cur.as_mut()) {
                            let cause = if msg.trim().is_empty() {
                                let l = first_line(&text);
                                if l.is_empty() {
                                    "test failed".to_string()
                                } else {
                                    l
                                }
                            } else {
                                clip(msg)
                            };
                            c.1 = Some(cause);
                        }
                    }
                    b"testcase" => {
                        if let Some((name, fail, skipped)) = cur.take() {
                            if !skipped {
                                let st = match fail {
                                    Some(cause) => TestStatus::failed(cause),
                                    None => TestStatus::Passed,
                                };
                                cases.push(TestCase::new(name, st));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth_open != 0 || cur.is_some() {
        return Err(TestFmtError::Malformed(
            "junit xml: unclosed element".into(),
        ));
    }
    if cases.is_empty() {
        return Err(TestFmtError::NoTests);
    }
    dedup(&mut cases);
    Ok(cases)
}

// ---------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------

/// Parse one execution's output in `format`. `root` is the directory the
/// evaluator ran in (used by [`TestFormat::VitestJson`]).
///
/// For libtest, stderr (where cargo prints the `Running` banners when the
/// streams are not merged) is placed before stdout; prefer `2>&1`. For the
/// report formats stdout is used unless only stderr holds a report.
pub fn parse_exec(
    format: TestFormat,
    exec: &ExecOutcome,
    root: Option<&str>,
) -> Result<Vec<TestCase>, TestFmtError> {
    match format {
        TestFormat::Libtest => {
            let text = if exec.stderr.is_empty() {
                exec.stdout.clone()
            } else {
                format!("{}\n{}", exec.stderr, exec.stdout)
            };
            parse_libtest(&text)
        }
        TestFormat::VitestJson => {
            let text = if exec.stdout.contains('{') {
                &exec.stdout
            } else {
                &exec.stderr
            };
            parse_vitest_json(text, root)
        }
        TestFormat::Junit => {
            let text = if exec.stdout.contains('<') {
                &exec.stdout
            } else {
                &exec.stderr
            };
            parse_junit(text)
        }
    }
}

/// Build the gate's per-test evidence for one evaluator from its baseline and
/// candidate executions.
///
/// Returns `None` — never fabricated evidence — when either run failed at
/// the transport level, timed out, or cannot be parsed in `format`; the
/// evaluator's receipt then judges it alone, as if no format were declared.
pub fn outcomes(
    format: TestFormat,
    baseline: &ExecOutcome,
    candidate: &ExecOutcome,
    baseline_root: Option<&str>,
    candidate_root: Option<&str>,
) -> Option<TestOutcomes> {
    let usable = |e: &ExecOutcome| e.transport_error.is_none() && !e.timed_out;
    if !usable(baseline) || !usable(candidate) {
        return None;
    }
    let b = parse_exec(format, baseline, baseline_root).ok()?;
    let c = parse_exec(format, candidate, candidate_root).ok()?;
    Some(TestOutcomes {
        baseline: b,
        candidate: c,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dream_gate::config::TestFormat;
    use dream_gate::outcome::ExecOutcome;
    use dream_gate::pertest::{TestCase, TestStatus};

    const LIBTEST_FAIL: &str = include_str!("../tests/fixtures/testfmt/libtest-fail.txt");
    const LIBTEST_BACKTRACE: &str =
        include_str!("../tests/fixtures/testfmt/libtest-fail-backtrace.txt");
    const LIBTEST_PASS: &str = include_str!("../tests/fixtures/testfmt/libtest-pass.txt");
    const VITEST_JSON: &str = include_str!("../tests/fixtures/testfmt/vitest.json");
    const JUNIT: &str = include_str!("../tests/fixtures/testfmt/junit-vitest.xml");

    fn get<'a>(cases: &'a [TestCase], name: &str) -> &'a TestCase {
        cases
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no case {name:?} in {:?}", names(cases)))
    }
    fn names(cases: &[TestCase]) -> Vec<&str> {
        cases.iter().map(|c| c.name.as_str()).collect()
    }
    fn exec(stdout: &str) -> ExecOutcome {
        ExecOutcome {
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            duration_ms: 1,
            transport_error: None,
            timed_out: false,
            started_at: "2026-10-15T00:00:00Z".into(),
        }
    }

    #[test]
    fn libtest_real_failing_run_across_unit_integration_and_doc_targets() {
        let c = parse_libtest(LIBTEST_FAIL).unwrap();
        assert_eq!(c.len(), 5, "ignored tests are not cases: {:?}", names(&c));
        let u = "unittests src/lib.rs::inner::tests::";
        assert_eq!(get(&c, &format!("{u}doubles")).status, TestStatus::Passed);
        assert_eq!(
            get(&c, &format!("{u}panics_as_expected")).status,
            TestStatus::Passed,
            "`- should panic ... ok` is a pass"
        );
        match &get(&c, &format!("{u}doubles_wrongly")).status {
            TestStatus::Failed { cause } => {
                assert!(cause.contains("two doubled should be five"), "{cause}");
                assert!(
                    cause.contains("left: 4") && cause.contains("right: 5"),
                    "{cause}"
                );
                assert!(!cause.contains("RUST_BACKTRACE"), "{cause}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            get(&c, "tests/integration.rs::integration_adds").status,
            TestStatus::Passed
        );
        assert_eq!(
            get(&c, "Doc-tests sample::src/lib.rs - add").status,
            TestStatus::Passed,
            "the `(line N)` suffix is dropped so names survive a shifted line"
        );
    }

    #[test]
    fn libtest_backtrace_noise_does_not_leak_into_the_cause() {
        let c = parse_libtest(LIBTEST_BACKTRACE).unwrap();
        let f = c
            .iter()
            .find(|c| c.name.ends_with("doubles_wrongly"))
            .unwrap();
        let TestStatus::Failed { cause } = &f.status else {
            panic!()
        };
        assert!(cause.contains("two doubled should be five"));
        assert!(
            !cause.contains("rust_begin_unwind") && !cause.contains("stack backtrace"),
            "{cause}"
        );
    }

    #[test]
    fn libtest_passing_run_and_stable_names_between_runs() {
        let c = parse_libtest(LIBTEST_PASS).unwrap();
        assert!(c.iter().all(|c| c.status == TestStatus::Passed));
        // `--skip` removed one test; the survivors keep the same names as in
        // the failing run, so baseline/candidate pairing works.
        let fail = parse_libtest(LIBTEST_FAIL).unwrap();
        assert!(c.iter().all(|c| fail.iter().any(|f| f.name == c.name)));
    }

    #[test]
    fn libtest_without_running_lines_has_bare_names_and_old_panic_format() {
        let text = "running 2 tests\ntest a::ok_one ... ok\ntest a::bad ... FAILED\n\nfailures:\n\n---- a::bad stdout ----\nthread 'a::bad' panicked at 'index out of bounds', src/x.rs:9:5\n\nfailures:\n    a::bad\n\ntest result: FAILED. 1 passed; 1 failed;\n";
        let c = parse_libtest(text).unwrap();
        assert_eq!(names(&c), vec!["a::ok_one", "a::bad"]);
        assert_eq!(c[1].status, TestStatus::failed("index out of bounds"));
    }

    #[test]
    fn libtest_duplicate_names_get_a_stable_suffix() {
        let text = "running 2 tests\ntest dup ... ok\ntest dup ... ok\n";
        assert_eq!(names(&parse_libtest(text).unwrap()), vec!["dup", "dup~2"]);
    }

    #[test]
    fn libtest_no_tests_is_an_error_not_an_empty_success() {
        for text in [
            "",
            "error[E0432]: unresolved import\nerror: could not compile\n",
        ] {
            assert_eq!(parse_libtest(text), Err(TestFmtError::NoTests));
        }
        // A terse run prints dots, which carry no names.
        assert_eq!(
            parse_libtest("running 3 tests\n...\n\ntest result: ok. 3 passed\n"),
            Err(TestFmtError::NoTests)
        );
    }

    #[test]
    fn vitest_json_real_run() {
        let c = parse_vitest_json(VITEST_JSON, Some("/work/vt")).unwrap();
        assert_eq!(c.len(), 3, "skipped is not a case: {:?}", names(&c));
        assert_eq!(
            get(&c, "sample.test.js::math adds").status,
            TestStatus::Passed
        );
        assert_eq!(
            get(&c, "sample.test.js::strings upper-cases").status,
            TestStatus::Passed
        );
        match &get(&c, "sample.test.js::math subtracts wrongly").status {
            TestStatus::Failed { cause } => {
                assert_eq!(
                    cause,
                    "AssertionError: expected 2 to be 5 // Object.is equality"
                );
            }
            other => panic!("{other:?}"),
        }
        // Without a root the absolute path is kept as is.
        let abs = parse_vitest_json(VITEST_JSON, None).unwrap();
        assert!(
            abs[0].name.starts_with("/work/vt/sample.test.js::"),
            "{}",
            abs[0].name
        );
    }

    #[test]
    fn vitest_json_suite_that_failed_to_load_is_a_failed_case() {
        let j = r#"{"testResults":[{"name":"/r/a.test.ts","status":"failed","message":"Error: Cannot find module './x'","assertionResults":[]}]}"#;
        let c = parse_vitest_json(j, Some("/r")).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "a.test.ts::(suite)");
        assert_eq!(
            c[0].status,
            TestStatus::failed("Error: Cannot find module './x'")
        );
    }

    #[test]
    fn vitest_json_garbage_and_empty_are_errors() {
        assert!(matches!(
            parse_vitest_json("not json", None),
            Err(TestFmtError::Malformed(_))
        ));
        assert_eq!(
            parse_vitest_json(r#"{"testResults":[]}"#, None),
            Err(TestFmtError::NoTests)
        );
        // Noise before the JSON object (a reporter banner) is tolerated.
        let noisy = format!("RUN v2.1.9 /r\n{VITEST_JSON}\n");
        assert_eq!(
            parse_vitest_json(&noisy, Some("/work/vt")).unwrap().len(),
            3
        );
    }

    #[test]
    fn junit_real_vitest_run() {
        let c = parse_junit(JUNIT).unwrap();
        assert_eq!(c.len(), 3, "skipped is not a case: {:?}", names(&c));
        assert_eq!(
            get(&c, "sample.test.js::math > adds").status,
            TestStatus::Passed
        );
        match &get(&c, "sample.test.js::math > subtracts wrongly").status {
            TestStatus::Failed { cause } => {
                assert_eq!(cause, "expected 2 to be 5 // Object.is equality");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn junit_surefire_shapes_cdata_error_nesting_and_no_wrapper() {
        let xml = r#"<?xml version="1.0"?>
<testsuite name="s" tests="3">
  <testcase classname="com.acme.A" name="ok" time="0.1"/>
  <testcase classname="com.acme.A" name="boom"><error type="java.lang.NPE">trace line 1
trace line 2</error></testcase>
  <testsuite name="inner">
    <testcase classname="com.acme.B" name="bad &amp; ugly"><failure><![CDATA[expected <1> but was <2>]]></failure></testcase>
    <testcase classname="" name="no-class"/>
  </testsuite>
</testsuite>"#;
        let c = parse_junit(xml).unwrap();
        assert_eq!(c.len(), 4);
        assert_eq!(get(&c, "com.acme.A::ok").status, TestStatus::Passed);
        assert_eq!(
            get(&c, "com.acme.A::boom").status,
            TestStatus::failed("trace line 1")
        );
        assert_eq!(
            get(&c, "com.acme.B::bad & ugly").status,
            TestStatus::failed("expected <1> but was <2>")
        );
        assert_eq!(get(&c, "no-class").status, TestStatus::Passed);
    }

    #[test]
    fn junit_malformed_and_empty_are_errors_and_entities_are_not_expanded() {
        assert!(matches!(
            parse_junit("<testsuite><testcase name='a'>"),
            Err(TestFmtError::Malformed(_))
        ));
        assert_eq!(parse_junit("<testsuites/>"), Err(TestFmtError::NoTests));
        // A DOCTYPE with an external entity is inert: nothing is fetched or
        // expanded, and the case still parses.
        let xxe = r#"<?xml version="1.0"?><!DOCTYPE d [<!ENTITY x SYSTEM "file:///etc/passwd">]><testsuite><testcase name="a&x;b"/></testsuite>"#;
        match parse_junit(xxe) {
            Ok(c) => assert!(!c[0].name.contains("root:"), "{}", c[0].name),
            Err(TestFmtError::Malformed(_)) => {}
            Err(e) => panic!("{e:?}"),
        }
    }

    #[test]
    fn outcomes_pair_baseline_and_candidate_or_decline() {
        let base = exec(LIBTEST_PASS);
        let cand = exec(LIBTEST_FAIL);
        let o = outcomes(TestFormat::Libtest, &base, &cand, None, None).unwrap();
        assert_eq!(o.baseline.len(), 4);
        assert_eq!(o.candidate.len(), 5);
        // A transport failure, a timeout or unparseable output declines to
        // fabricate evidence; the receipt alone then judges the evaluator.
        let blocked = ExecOutcome::blocked("no route");
        assert!(outcomes(TestFormat::Libtest, &base, &blocked, None, None).is_none());
        let mut timed = exec(LIBTEST_FAIL);
        timed.timed_out = true;
        assert!(outcomes(TestFormat::Libtest, &base, &timed, None, None).is_none());
        assert!(outcomes(
            TestFormat::Libtest,
            &base,
            &exec("compile error"),
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn stderr_is_appended_when_stdout_lacks_the_target_banners() {
        let mut e = exec("running 1 test\ntest t ... ok\n");
        e.stderr = "     Running unittests src/lib.rs (target/debug/deps/x-1)\n".into();
        let c = parse_exec(TestFormat::Libtest, &e, None).unwrap();
        assert_eq!(c.len(), 1);
    }
}
