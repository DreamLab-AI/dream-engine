//! Pure parsing and repair of model-written unified diffs.
//!
//! Nothing here touches git or the filesystem: the caller supplies file text
//! through a closure and does the applying. For the security check on which
//! paths a diff touches, see [`crate::guard`]; the helpers here are
//! deliberately *lenient* repairs and must never be used for that purpose.

/// Whether a unified diff deletes a binary file. Candidates may not: a
/// deleted binary cannot be reviewed from the diff, and git records only
/// "Binary files … differ" for it, so the loss is invisible in review.
pub fn deletes_binary(patch: &str) -> bool {
    patch.split("diff --git ").skip(1).any(|file| {
        file.contains("deleted file mode")
            && (file.contains("Binary files") || file.contains("GIT binary patch"))
    })
}

/// Pull the candidate patch out of the first ```dream-patch fenced block.
/// Returns None when absent or empty (a finding with no code change).
///
/// The block ends at the first line that is *only* a fence (trailing
/// whitespace allowed). A bare "```" anywhere else is diff content: every diff
/// line carries a one-character prefix, so a Rust doc-test fence (`/// ````)
/// or a Markdown code fence edited by the patch appears as ` /// ```` or
/// `+```` and never as a fence line. Ending at the first "```" substring cut
/// such patches mid-hunk ("corrupt patch at :14", 2026-10-08).
pub fn extract_patch(report: &str) -> Option<String> {
    const TAG: &str = "```dream-patch";
    let start = report.find(TAG)? + TAG.len();
    let rest = &report[start..];
    // Skip the remainder of the opening fence line.
    let body_start = rest.find('\n').map(|i| i + 1)?;
    let rest = &rest[body_start..];
    let mut end = None;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "```" {
            end = Some(offset);
            break;
        }
        offset += line.len();
    }
    let body = &rest[..end?];
    if body.trim().is_empty() {
        None
    } else {
        Some(body.to_string())
    }
}

/// Mechanical repairs for model-written diffs that `git apply` refuses even
/// though every hunk's text is in the base tree. Each repair was found by
/// replaying vetoed nights (2026-10-08..10) against their frozen commits:
///
/// * **Missing `diff --git` headers.** Without them `git apply --recount`
///   reads the next file's `--- a/…` header as a deleted line of the previous
///   hunk (2026-10-09). A header is synthesised for every
///   `---`/`+++` pair that lacks one.
/// * **Trailing blank separator lines** at the end of a hunk are dropped:
///   in `--recount` mode git counts them as context the file may not have.
/// * **Asymmetric context.** A hunk with leading context but none trailing
///   is anchored by git to end-of-file, so it fails anywhere mid-file
///   (2026-10-09). When the hunk's old lines occur exactly once
///   in the base file and not at its end, up to three following lines of the
///   real file are appended as trailing context.
///
/// Only headers and context lines change: every `+`/`-` line is kept
/// byte-for-byte, so the normalised diff makes exactly the model's change.
/// A path that appears in more than one section is left alone (its second
/// section would be anchored against a file the first already changed).
/// `read` returns the base-revision text of a path. Returns the normalised
/// patch and a note per repair made; no notes means nothing changed.
pub fn normalise_patch(
    patch: &str,
    read: impl Fn(&str) -> Option<String>,
) -> (String, Vec<String>) {
    let lines: Vec<&str> = patch.lines().collect();
    let mut notes: Vec<String> = Vec::new();
    let header_path = |l: &str, prefix: &str| -> Option<String> {
        let raw = l.strip_prefix(prefix)?;
        // GNU diff appends a tab and a timestamp.
        let raw = raw.split('\t').next().unwrap_or(raw).trim();
        if raw == "/dev/null" {
            return Some("/dev/null".into());
        }
        let raw = raw
            .strip_prefix("a/")
            .or_else(|| raw.strip_prefix("b/"))
            .unwrap_or(raw);
        Some(raw.to_string())
    };
    // Paths per section, to refuse normalising a path touched twice.
    let mut seen: Vec<String> = Vec::new();
    let mut repeated = false;
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("--- ") && lines.get(i + 1).is_some_and(|n| n.starts_with("+++ ")) {
            let a = header_path(l, "--- ").unwrap_or_default();
            let b = header_path(lines[i + 1], "+++ ").unwrap_or_default();
            let name = if b == "/dev/null" { a } else { b };
            if seen.contains(&name) {
                repeated = true;
            }
            seen.push(name);
        }
    }
    if repeated {
        return (patch.to_string(), Vec::new());
    }

    let is_file_header = |i: usize| {
        lines[i].starts_with("--- ") && lines.get(i + 1).is_some_and(|n| n.starts_with("+++ "))
    };
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    let mut base: Option<Vec<String>> = None;
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if is_file_header(i) {
            let a = header_path(l, "--- ").unwrap_or_default();
            let b = header_path(lines[i + 1], "+++ ").unwrap_or_default();
            let name = if b == "/dev/null" {
                a.clone()
            } else {
                b.clone()
            };
            let has_git_header = out
                .iter()
                .rev()
                .take(4)
                .any(|p| p.starts_with("diff --git "));
            if !has_git_header {
                out.push(format!("diff --git a/{name} b/{name}"));
                if a == "/dev/null" {
                    out.push("new file mode 100644".into());
                }
                if b == "/dev/null" {
                    out.push("deleted file mode 100644".into());
                }
                notes.push(format!("header synthesised for {name}"));
            }
            base = if a == "/dev/null" {
                None
            } else {
                read(&name).map(|t| t.lines().map(str::to_string).collect())
            };
            current = Some(name);
            out.push(l.to_string());
            out.push(lines[i + 1].to_string());
            i += 2;
            continue;
        }
        if l.starts_with("@@") {
            let mut body: Vec<String> = Vec::new();
            let mut j = i + 1;
            while j < lines.len()
                && !lines[j].starts_with("@@")
                && !lines[j].starts_with("diff --git ")
                && !is_file_header(j)
            {
                body.push(lines[j].to_string());
                j += 1;
            }
            let mut dropped = 0;
            while body.last().is_some_and(|b| b.is_empty()) {
                body.pop();
                dropped += 1;
            }
            if dropped > 0 {
                notes.push(format!(
                    "{} trailing blank line(s) dropped from a hunk in {}",
                    dropped,
                    current.as_deref().unwrap_or("?")
                ));
            }
            if let Some(file) = base.as_ref() {
                if let Some(extra) = trailing_context(&body, file) {
                    notes.push(format!(
                        "{} line(s) of trailing context added to a hunk in {}",
                        extra.len(),
                        current.as_deref().unwrap_or("?")
                    ));
                    body.extend(extra.into_iter().map(|x| format!(" {x}")));
                }
            }
            out.push(l.to_string());
            out.extend(body);
            i = j;
            continue;
        }
        out.push(l.to_string());
        i += 1;
    }
    let mut text = out.join("\n");
    text.push('\n');
    (text, notes)
}

/// The real lines to append to a hunk with leading context but no trailing
/// context, or None when no padding is needed or it cannot be done safely.
fn trailing_context(body: &[String], file: &[String]) -> Option<Vec<String>> {
    if body.iter().any(|b| b.starts_with('\\')) {
        return None; // "\ No newline at end of file": the hunk is at EOF.
    }
    let ctx = |b: &String| b.starts_with(' ') || b.is_empty();
    let lead = body.iter().take_while(|b| ctx(b)).count();
    let trail = body.iter().rev().take_while(|b| ctx(b)).count();
    if lead == 0 || trail > 0 || lead == body.len() {
        return None;
    }
    let old: Vec<&str> = body
        .iter()
        .filter(|b| ctx(b) || b.starts_with('-'))
        .map(|b| if b.is_empty() { "" } else { &b[1..] })
        .collect();
    if old.is_empty() || old.len() > file.len() {
        return None;
    }
    let hits: Vec<usize> = (0..=file.len() - old.len())
        .filter(|&s| {
            file[s..s + old.len()]
                .iter()
                .map(String::as_str)
                .eq(old.iter().copied())
        })
        .collect();
    let [start] = hits[..] else { return None };
    let end = start + old.len();
    if end >= file.len() {
        return None; // genuinely at end of file: git's anchoring is right.
    }
    let want = lead.min(3);
    Some(file[end..(end + want).min(file.len())].to_vec())
}

/// Accept git blob bytes as text: `None` when they contain a NUL byte or are
/// not valid UTF-8 (binary content).
///
/// This is the pure half of reading a blob; the caller runs `git show` (or
/// equivalent) and hands the bytes over.
///
/// ```
/// use dream_gate::patch::blob_text;
/// assert_eq!(blob_text(b"hi\n".to_vec()).as_deref(), Some("hi\n"));
/// assert_eq!(blob_text(vec![0, 1, 2]), None);
/// ```
pub fn blob_text(bytes: Vec<u8>) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_deletion_is_detected_per_file() {
        let del_bin = "diff --git a/x.png b/x.png\ndeleted file mode 100644\nindex 1..0\nBinary files a/x.png and /dev/null differ\n";
        let del_text = "diff --git a/x.txt b/x.txt\ndeleted file mode 100644\n--- a/x.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n";
        let mod_bin = "diff --git a/y.png b/y.png\nindex 1..2 100644\nBinary files a/y.png and b/y.png differ\n";
        assert!(deletes_binary(del_bin));
        assert!(!deletes_binary(del_text));
        assert!(!deletes_binary(mod_bin));
        assert!(deletes_binary(&format!("{del_text}{del_bin}")));
        assert!(!deletes_binary(&format!("{del_text}{mod_bin}")));
    }

    #[test]
    fn extract_patch_pulls_the_fenced_diff() {
        let report = "blah\n\n```dream-patch\ndiff --git a/x b/x\n+new\n```\n\ntail";
        let p = extract_patch(report).unwrap();
        assert!(p.starts_with("diff --git a/x b/x"));
        assert!(p.contains("+new"));
        assert!(!p.contains("```"));
    }

    #[test]
    fn extract_patch_none_when_absent_or_empty() {
        assert!(extract_patch("no patch here").is_none());
        assert!(extract_patch("```dream-patch\n\n```").is_none());
        assert!(extract_patch("```dream-patch\n   \n```").is_none());
    }

    #[test]
    fn a_fence_inside_the_diff_does_not_end_the_block() {
        // 2026-10-08: the hunk edits a Rust doc test, so the diff
        // carries ` /// ```` lines. Ending at the first "```" substring cut it
        // at line 14 ("corrupt patch").
        let diff = "--- a/lib.rs\n+++ b/lib.rs\n@@ -1,3 +1,4 @@\n /// ```\n /// x();\n+/// y();\n /// ```\n";
        let report = format!("text\n```dream-patch\n{diff}```\nafter\n```\nother block\n```\n");
        assert_eq!(extract_patch(&report).as_deref(), Some(diff));
        // A closing fence with trailing whitespace or CRLF still closes.
        let crlf = format!("```dream-patch\r\n{diff}```  \r\nrest");
        assert_eq!(extract_patch(&crlf).as_deref(), Some(diff));
        // No closing fence at all: no patch rather than a guess.
        assert!(extract_patch("```dream-patch\n--- a/x\n+++ b/x\n").is_none());
    }

    #[test]
    fn normalising_never_changes_an_added_or_removed_line() {
        let base = |_: &str| Some("a\nb\nc\nd\ne\n".to_string());
        let patch = "--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n a\n b\n-c\n+  C with  spaces\t\n\n";
        let (out, notes) = normalise_patch(patch, base);
        assert!(!notes.is_empty());
        let changes = |t: &str| -> Vec<String> {
            t.lines()
                .filter(|l| {
                    (l.starts_with('+') && !l.starts_with("+++"))
                        || (l.starts_with('-') && !l.starts_with("---"))
                })
                .map(str::to_string)
                .collect()
        };
        assert_eq!(changes(&out), changes(patch));
        assert!(out.contains(" d\n"), "{out}");
    }

    #[test]
    fn normalising_leaves_eof_hunks_and_repeated_paths_alone() {
        let base = |_: &str| Some("a\nb\nc\n".to_string());
        // Old block ends at EOF: git's end-anchoring is correct, no padding.
        let at_eof = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n a\n b\n-c\n+C\n";
        assert!(normalise_patch(at_eof, base).1.is_empty());
        // Same path in two sections: untouched.
        let twice =
            "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+A\n--- a/f\n+++ b/f\n@@ -3 +3 @@\n-c\n+C\n";
        let (out, notes) = normalise_patch(twice, base);
        assert!(notes.is_empty());
        assert_eq!(out, twice);
    }

    #[test]
    fn blob_text_refuses_binary_and_invalid_utf8() {
        assert_eq!(blob_text(b"a\n".to_vec()).as_deref(), Some("a\n"));
        assert_eq!(blob_text(b"a\0b".to_vec()), None);
        assert_eq!(blob_text(vec![0xff, 0xfe]), None);
    }
}
