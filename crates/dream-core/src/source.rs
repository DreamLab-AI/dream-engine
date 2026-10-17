//! Tree-read: put the source a hypothesis touches in front of the model.
//!
//! The nightly model is a single chat completion ([`crate::seams::Llm`]) with
//! no tools. Handed evaluator receipts but **no source text**, and then asked
//! for a `git apply`-able diff, it answers by narrating an ACCEPT with no
//! ```dream-patch block (gate `CandidateState::NoPatch` → INCONCLUSIVE) or by
//! guessing file contents (`DidNotApply`). Repeated, those nights park a
//! repository on standby.
//!
//! The fix keeps the single-completion design and closes the information gap:
//!
//! 1. **Source plan**: a bounded side-channel call sees tonight's deep, the
//!    required evaluator commands, an excerpt of the evidence pack and an index
//!    of tracked paths, and names at most [`SourceConfig::max_files`] files
//!    (optionally line ranges) it needs. Paths the evidence itself mentions are
//!    added deterministically, so a planner failure still yields useful source.
//! 2. **Validation**: every requested path must be relative, free of `..`,
//!    tracked at the dispatched commit, and outside the secret denylist
//!    ([`is_denied`], extended by a caller-supplied [`Denylist`]).
//! 3. **Tree read**: contents come from `git show <commit>:<path>` at the SAME
//!    commit the candidate worktree is built from, so the text the model sees
//!    is byte-identical to the baseline its diff will be applied to.
//! 4. **Section**: rendered as `## Source (from <commit>)` with per-file
//!    headers and explicit elision markers when a file is clipped.
//! 5. **Repair pass**: if the report still declares ACCEPT without a
//!    ```dream-patch block, one follow-up completion is asked for the diff
//!    alone (or an explicit `NO-PATCH: <reason>`). See [`needs_repair`].
//!
//! Every stage is fail-open: an error yields no section and the night runs as
//! before; the gate's `NoPatch` veto is unchanged. Nothing here reads the
//! process environment: every knob is an explicit parameter.
//!
//! # Example
//!
//! ```
//! use dream_core::source::{validate_path, PathRejection};
//! use std::collections::HashSet;
//!
//! let tracked: HashSet<&str> = ["src/lib.rs"].into_iter().collect();
//! assert_eq!(validate_path("./src/lib.rs", &tracked), Ok("src/lib.rs".to_string()));
//! assert_eq!(validate_path("../etc/passwd", &tracked), Err(PathRejection::Traversal));
//! ```

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::seams::{Llm, LlmError, LlmRequest};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Default cap on files inlined per night.
pub const DEFAULT_MAX_FILES: usize = 12;
/// Default total byte budget for the source section (~15k tokens).
pub const DEFAULT_BUDGET_BYTES: usize = 60_000;
/// Default per-file cap; longer files keep head + tail with a marker.
pub const DEFAULT_FILE_CAP_BYTES: usize = 16_000;
/// Default cap on the path index shown to the source planner.
pub const DEFAULT_INDEX_CAP_CHARS: usize = 24_000;
/// Files larger than this are never offered or read (generated/vendored).
const MAX_READABLE_BYTES: u64 = 1_000_000;
/// Evidence excerpt shown to the source planner (tail of the pack).
const PLANNER_EVIDENCE_CHARS: usize = 12_000;

/// Tree-read knobs. Every value is explicit; nothing is read from the
/// environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceConfig {
    /// When false the stage is skipped entirely.
    pub enabled: bool,
    /// Most files inlined per night.
    pub max_files: usize,
    /// Total bytes of file text in the section.
    pub budget_bytes: usize,
    /// Bytes per file before head+tail clipping.
    pub file_cap_bytes: usize,
    /// Chars of path index shown to the planner.
    pub index_cap_chars: usize,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_files: DEFAULT_MAX_FILES,
            budget_bytes: DEFAULT_BUDGET_BYTES,
            file_cap_bytes: DEFAULT_FILE_CAP_BYTES,
            index_cap_chars: DEFAULT_INDEX_CAP_CHARS,
        }
    }
}

// ---------------------------------------------------------------------------
// Tracked paths at the dispatched commit
// ---------------------------------------------------------------------------

/// One tracked blob at the dispatched commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedFile {
    /// Repo-relative path.
    pub path: String,
    /// Blob size in bytes.
    pub size: u64,
}

/// Every blob tracked at `rev` (`git ls-tree -r -l`). Gitignored files are
/// absent by construction — the same set `git archive` ships to a remote runner.
pub fn tracked_files(repo: &Path, rev: &str) -> Option<Vec<TrackedFile>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-tree", "-r", "-l", "--full-tree", rev])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_ls_tree(&String::from_utf8_lossy(&out.stdout)))
}

/// Parse `git ls-tree -r -l` output: `<mode> <type> <object> <size>\t<path>`.
/// Submodule (`commit`) entries and unparseable lines are skipped.
pub fn parse_ls_tree(text: &str) -> Vec<TrackedFile> {
    text.lines()
        .filter_map(|line| {
            let (meta, path) = line.split_once('\t')?;
            let mut parts = meta.split_whitespace();
            let _mode = parts.next()?;
            if parts.next()? != "blob" {
                return None;
            }
            let _obj = parts.next()?;
            let size = parts.next()?.parse().ok()?;
            Some(TrackedFile {
                path: path.to_string(),
                size,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Path validation
// ---------------------------------------------------------------------------

/// Why a requested path was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathRejection {
    /// Nothing left after trimming.
    Empty,
    /// Absolute, home-relative or drive-lettered.
    Absolute,
    /// Contains a `..` component.
    Traversal,
    /// On the denylist, or a binary extension.
    Denied,
    /// Not tracked at the dispatched commit.
    NotInTree,
}

/// Secret-bearing or credential-looking files are never sent to a provider,
/// whatever the planner asks for. This is the built-in, project-neutral list;
/// extend it with a [`Denylist`].
pub fn is_denied(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    base.starts_with(".env")
        || lower.contains("secret")
        || lower.contains("credential")
        || lower.contains("private_key")
        || lower.contains("privkey")
        || [
            ".pem",
            ".key",
            ".p12",
            ".pfx",
            ".jks",
            ".keystore",
            ".kdbx",
            ".age",
        ]
        .iter()
        .any(|ext| base.ends_with(ext))
        || [
            "id_rsa",
            "id_dsa",
            "id_ecdsa",
            "id_ed25519",
            ".npmrc",
            ".netrc",
            ".pypirc",
            ".git-credentials",
        ]
        .contains(&base)
}

/// Extra paths a host never wants sent to a provider, on top of
/// [`is_denied`].
///
/// An entry ending in `/` denies a directory (at any depth); any other entry
/// denies a file whose whole path or final component equals it. Matching is
/// ASCII case-insensitive.
///
/// ```
/// use dream_core::source::Denylist;
/// let d = Denylist::new(["vault/", "session-key.txt"]);
/// assert!(d.denies("vault/a.txt") && d.denies("x/vault/a.txt"));
/// assert!(d.denies("deep/session-key.txt"));
/// assert!(!d.denies("src/lib.rs"));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Denylist {
    extra: Vec<String>,
}

impl Denylist {
    /// A denylist with the given extra entries.
    pub fn new<I, S>(extra: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            extra: extra
                .into_iter()
                .map(|s| s.into().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }

    /// True when `path` is denied by the built-in list or an extra entry.
    pub fn denies(&self, path: &str) -> bool {
        if is_denied(path) {
            return true;
        }
        let lower = path.to_ascii_lowercase();
        let base = lower.rsplit('/').next().unwrap_or(&lower);
        self.extra.iter().any(|e| match e.strip_suffix('/') {
            Some(dir) => lower.starts_with(e.as_str()) || lower.contains(&format!("/{dir}/")),
            None => lower == *e || base == e,
        })
    }
}

/// Binary or bulky-generated extensions are not worth inlining as text.
fn is_binary_ext(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".png",
        ".jpg",
        ".jpeg",
        ".gif",
        ".webp",
        ".ico",
        ".bmp",
        ".svgz",
        ".pdf",
        ".zip",
        ".gz",
        ".tgz",
        ".xz",
        ".bz2",
        ".7z",
        ".tar",
        ".wasm",
        ".woff",
        ".woff2",
        ".ttf",
        ".otf",
        ".eot",
        ".mp3",
        ".mp4",
        ".mov",
        ".webm",
        ".ogg",
        ".wav",
        ".glb",
        ".gltf",
        ".blend",
        ".bin",
        ".so",
        ".dylib",
        ".dll",
        ".exe",
        ".o",
        ".a",
        ".rlib",
        ".sqlite",
        ".db",
        ".parquet",
        ".onnx",
        ".safetensors",
        ".gguf",
        ".npy",
        ".pt",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// Normalise and validate one requested path against the tracked set.
pub fn validate_path(raw: &str, tracked: &HashSet<&str>) -> Result<String, PathRejection> {
    validate_path_with(raw, tracked, &Denylist::default())
}

/// [`validate_path`] with an extended [`Denylist`].
pub fn validate_path_with(
    raw: &str,
    tracked: &HashSet<&str>,
    deny: &Denylist,
) -> Result<String, PathRejection> {
    let p = raw.trim().trim_matches('`').trim();
    let p = p.strip_prefix("./").unwrap_or(p);
    if p.is_empty() {
        return Err(PathRejection::Empty);
    }
    let bytes = p.as_bytes();
    if p.starts_with('/')
        || p.starts_with('\\')
        || p.starts_with('~')
        || (bytes.len() >= 2 && bytes[1] == b':')
    {
        return Err(PathRejection::Absolute);
    }
    if p.split(['/', '\\']).any(|c| c == "..") {
        return Err(PathRejection::Traversal);
    }
    if deny.denies(p) {
        return Err(PathRejection::Denied);
    }
    if !tracked.contains(p) {
        return Err(PathRejection::NotInTree);
    }
    Ok(p.to_string())
}

// ---------------------------------------------------------------------------
// Planner request / reply
// ---------------------------------------------------------------------------

/// A 1-based inclusive line range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    /// First line, 1-based.
    pub start: usize,
    /// Last line, 1-based, inclusive.
    pub end: usize,
}

impl LineRange {
    /// Parse `"10-80"`, `"10:80"` or `"42"`; `None` when malformed or empty.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (a, b) = match s.split_once(['-', ':']) {
            Some((a, b)) => (a.trim().parse().ok()?, b.trim().parse().ok()?),
            None => {
                let n: usize = s.parse().ok()?;
                (n, n)
            }
        };
        (a >= 1 && b >= a).then_some(Self { start: a, end: b })
    }
}

/// One file the planner asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRequest {
    /// Repo-relative path.
    pub path: String,
    /// The line range requested, if any.
    pub lines: Option<LineRange>,
}

impl SourceRequest {
    /// Parse `path`, `path:10-80` or `path:42` as written in a report's
    /// "Sources wanted next night" list. A trailing `:a-b` that parses as a
    /// [`LineRange`] becomes the range; anything else stays part of the path.
    /// `None` for an empty token.
    pub fn parse(token: &str) -> Option<Self> {
        let t = token.trim().trim_matches('`').trim();
        let t = t.strip_prefix("./").unwrap_or(t);
        if t.is_empty() {
            return None;
        }
        if let Some((head, tail)) = t.rsplit_once(':') {
            if let Some(range) = LineRange::parse(tail) {
                if !head.is_empty() {
                    return Some(Self {
                        path: head.to_string(),
                        lines: Some(range),
                    });
                }
            }
        }
        Some(Self {
            path: t.to_string(),
            lines: None,
        })
    }
}

/// Parse the source planner's reply. Accepts `{"files":[{"path":..,
/// "lines":"a-b"}]}` or `{"files":["path", ..]}`, tolerating fences and prose
/// around the JSON object.
pub fn parse_source_plan(reply: &str) -> Option<Vec<SourceRequest>> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Bare(String),
        Full {
            path: String,
            #[serde(default)]
            lines: Option<String>,
        },
    }
    #[derive(Deserialize)]
    struct Plan {
        files: Vec<Entry>,
    }
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    if end < start {
        return None;
    }
    let plan: Plan = serde_json::from_str(&reply[start..=end]).ok()?;
    Some(
        plan.files
            .into_iter()
            .map(|e| match e {
                Entry::Bare(path) => SourceRequest { path, lines: None },
                Entry::Full { path, lines } => SourceRequest {
                    path,
                    lines: lines.as_deref().and_then(LineRange::parse),
                },
            })
            .collect(),
    )
}

/// Tracked paths the evidence text mentions verbatim (compiler errors, test
/// names, evaluator commands), in first-mention order. Tokens may carry a
/// `:line[:col]` suffix, quotes, or trailing punctuation.
pub fn mentioned_paths(text: &str, tracked: &HashSet<&str>, deny: &Denylist) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || "\"'`()[]{}<>,;=|".contains(c)) {
        let mut tok = raw.trim_end_matches(['.', ':']);
        tok = tok.strip_prefix("./").unwrap_or(tok);
        // Drop a trailing `:12` / `:12:5` location suffix.
        while let Some((head, tail)) = tok.rsplit_once(':') {
            if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
                tok = head;
            } else {
                break;
            }
        }
        if tok.len() < 3 || !tok.contains(['/', '.']) {
            continue;
        }
        if tracked.contains(tok) && !deny.denies(tok) && seen.insert(tok.to_string()) {
            out.push(tok.to_string());
        }
    }
    out
}

/// How many of `max_files` slots requested paths may take: half, at least one.
pub fn requested_slots(max_files: usize) -> usize {
    (max_files / 2).max(1).min(max_files)
}

/// Merge requested paths (first, capped at [`requested_slots`]), planner
/// picks, then evidence-mentioned paths, validating each and stopping at
/// `max_files`. Returns the kept requests and the rejections.
///
/// Requested paths come from the previous night's own "Sources wanted next
/// night" list (or an operator), so they are honoured ahead of the planner —
/// a model asked to pick files cannot be held to a list, and that is how
/// the same file came to be asked for three nights running. The cap keeps a
/// stale request from starving the planner of the files tonight's evidence
/// actually points at. Every requested path still passes the same tree and
/// denylist checks as a planner pick: the report proposes, the tree decides.
pub fn select(
    requested: &[SourceRequest],
    planned: &[SourceRequest],
    mentioned: &[String],
    tracked: &HashSet<&str>,
    max_files: usize,
    deny: &Denylist,
) -> (Vec<SourceRequest>, Vec<(String, PathRejection)>) {
    let mut kept: Vec<SourceRequest> = Vec::new();
    let mut rejected = Vec::new();
    let reserve = requested_slots(max_files);
    let candidates = requested
        .iter()
        .take(reserve)
        .cloned()
        .chain(planned.iter().cloned())
        .chain(mentioned.iter().map(|p| SourceRequest {
            path: p.clone(),
            lines: None,
        }));
    for req in candidates {
        if kept.len() >= max_files {
            break;
        }
        match validate_path_with(&req.path, tracked, deny) {
            Ok(path) => {
                if kept.iter().any(|k| k.path == path) {
                    continue;
                }
                if is_binary_ext(&path) {
                    rejected.push((path, PathRejection::Denied));
                    continue;
                }
                kept.push(SourceRequest {
                    path,
                    lines: req.lines,
                });
            }
            Err(r) => rejected.push((req.path.clone(), r)),
        }
    }
    (kept, rejected)
}

/// The path index the planner chooses from: readable text files only,
/// shallow paths first, clipped to `cap` chars with an explicit remainder.
pub fn index_listing(files: &[TrackedFile], cap: usize, deny: &Denylist) -> String {
    let mut usable: Vec<&TrackedFile> = files
        .iter()
        .filter(|f| {
            f.size <= MAX_READABLE_BYTES && !deny.denies(&f.path) && !is_binary_ext(&f.path)
        })
        .collect();
    usable.sort_by(|a, b| {
        a.path
            .matches('/')
            .count()
            .cmp(&b.path.matches('/').count())
            .then(a.path.cmp(&b.path))
    });
    let mut out = String::new();
    let mut shown = 0usize;
    for f in &usable {
        let line = format!("{} ({}B)\n", f.path, f.size);
        if out.len() + line.len() > cap {
            break;
        }
        out.push_str(&line);
        shown += 1;
    }
    if shown < usable.len() {
        out.push_str(&format!(
            "… {} more tracked paths not listed (you may still name any tracked path)\n",
            usable.len() - shown
        ));
    }
    out
}

/// The source planner's prompt.
pub fn source_plan_prompt(
    deep: &str,
    scans: &str,
    commands: &[String],
    evidence: &str,
    index: &str,
    max_files: usize,
    supplied: &[SourceRequest],
) -> String {
    let supplied_note = if supplied.is_empty() {
        String::new()
    } else {
        let list = supplied
            .iter()
            .map(|r| match r.lines {
                Some(l) => format!("- `{}` lines {}-{}", r.path, l.start, l.end),
                None => format!("- `{}`", r.path),
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\nAlready being read tonight at the previous night's request (do not name these again; spend your picks on what else the change must stay consistent with):\n{list}\n"
        )
    };
    let cmds = if commands.is_empty() {
        "(none)".to_string()
    } else {
        commands
            .iter()
            .map(|c| format!("- `{c}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"You are choosing which source files an overnight research engine should read before proposing ONE small, testable code change for **{deep}** (scans: {scans}).

The engine will run these required evaluator commands against the proposed change:
{cmds}

Tonight's evidence (tail):
```
{evidence}
```

Tracked files at the commit under test:
{index}
{supplied_note}
Name at most {max_files} files whose exact current text you need in order to write a correct unified diff (the files you would change, plus anything a change must stay consistent with — the evaluator scripts, the test that fails, the config it reads). Prefer a line range for large files.

Respond with ONLY a JSON object, no prose:
{{"files":[{{"path":"<repo-relative path>","lines":"<start-end, optional>"}}]}}"#,
        evidence = evidence
    )
}

// ---------------------------------------------------------------------------
// Tree read + rendering
// ---------------------------------------------------------------------------

/// Read `path` at `rev`. `None` for missing, non-UTF-8 or NUL-bearing blobs.
pub fn read_blob(repo: &Path, rev: &str, path: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show", &format!("{rev}:{path}")])
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.contains(&0) {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Lines `range` (1-based, inclusive, clamped) of `text`, plus the file's
/// total line count.
pub fn slice_lines(text: &str, range: LineRange) -> (String, usize) {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 || range.start > total {
        return (String::new(), total);
    }
    let end = range.end.min(total);
    (lines[range.start - 1..end].join("\n"), total)
}

/// Head + tail clip on char boundaries. The marker tells the model text is
/// missing so it never writes a hunk against content it did not see.
pub fn clip_file(s: &str, cap: usize) -> (String, bool) {
    if s.len() <= cap {
        return (s.to_string(), false);
    }
    let head = cap * 2 / 3;
    let tail = cap - head;
    let mut h = head.min(s.len());
    while !s.is_char_boundary(h) {
        h -= 1;
    }
    let mut t = s.len() - tail;
    while !s.is_char_boundary(t) {
        t += 1;
    }
    let elided_lines = s[h..t].lines().count();
    (
        format!(
            "{}\n…[{} bytes / ~{} lines elided by the engine — do NOT write hunks inside this gap; name a line range tomorrow if you need it]…\n{}",
            &s[..h],
            t - h,
            elided_lines,
            &s[t..]
        ),
        true,
    )
}

/// One file as it entered (or failed to enter) the section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileRecord {
    /// Repo-relative path.
    pub path: String,
    /// Bytes of body text inlined.
    pub bytes: usize,
    /// Whether the file was head+tail clipped.
    pub clipped: bool,
    /// The line range requested, if any.
    pub lines: Option<LineRange>,
    /// `inlined` | `unreadable` | `over-budget`.
    pub status: String,
    /// Whether the previous night (or the operator) asked for this file.
    #[serde(default)]
    pub requested: bool,
}

/// Receipt of tonight's tree read, persisted as `source.json`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SourceRecord {
    /// The commit read.
    pub commit: String,
    /// How the planner call ended (`ok`, `ok-fallback`, `unparseable`, `failed: …`).
    pub planner: String,
    /// Every selected file and its fate.
    pub files: Vec<FileRecord>,
    /// Requested paths refused, with the reason.
    pub rejected: Vec<(String, String)>,
    /// Size of the rendered section.
    pub section_bytes: usize,
    /// Requested paths that were inlined tonight (`path` or `path:a-b`).
    #[serde(default)]
    pub requested_honoured: Vec<String>,
    /// Requested paths that were not inlined, with the reason
    /// (`not-in-tree`, `denied`, `over-cap`, `over-budget`, `unreadable`…).
    #[serde(default)]
    pub requested_refused: Vec<(String, String)>,
}

/// A file fetched from the tree, ready to render.
#[derive(Debug, Clone)]
pub struct FetchedFile {
    /// Repo-relative path.
    pub path: String,
    /// The line range requested, if any.
    pub lines: Option<LineRange>,
    /// The file text; `None` when unreadable.
    pub text: Option<String>,
    /// Whether this file was requested rather than planned or mentioned.
    pub requested: bool,
}

/// `path` or `path:a-b` for a request, as the engine writes it in records.
pub fn request_key(path: &str, lines: Option<LineRange>) -> String {
    match lines {
        Some(l) => format!("{path}:{}-{}", l.start, l.end),
        None => path.to_string(),
    }
}

/// Whether `text` carries key material that must never reach a provider.
///
/// Name-based denial ([`is_denied`]) cannot see a secret sitting in an
/// ordinary-looking file (a test fixture, a doc example, a pasted config), and
/// a redactor typically only rewrites paths. So a file holding a PEM
/// private key, a bech32 secret key (`nsec1` + 58 data characters) or a
/// hex secret assigned to a key-looking name is withheld whole — never
/// partially redacted, since a clipped key is still a key.
pub fn has_secret_content(text: &str) -> bool {
    if text.contains("PRIVATE KEY-----") {
        return true;
    }
    let bech32_run = |rest: &str| {
        rest.chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            .count()
    };
    if text
        .match_indices("nsec1")
        .any(|(i, _)| bech32_run(&text[i + 5..]) >= 58)
    {
        return true;
    }
    text.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        let keyish = ["privkey", "private_key", "secret", "seckey", "nsec"]
            .iter()
            .any(|k| lower.contains(k));
        keyish
            && line
                .split(|c: char| !c.is_ascii_hexdigit())
                .any(|run| run.len() == 64)
    })
}

/// Render `## Source (from <commit>)`. Pure: the caller supplies file text.
/// Files carrying key material are withheld whole.
pub fn render_section(
    commit: &str,
    files: &[FetchedFile],
    cfg: &SourceConfig,
    redact: &dyn Fn(&str) -> String,
) -> (String, Vec<FileRecord>) {
    let mut out = format!(
        "\n## Source (from `{commit}`)\n\n\
         These files are read by the engine from commit `{commit}` — the exact tree the \
         evaluators ran against and your diff will be applied to. Write hunks only against \
         text shown here; anything marked elided is not visible to you.\n\n"
    );
    let mut records = Vec::new();
    let mut spent = 0usize;
    // Requested files share at most half the byte budget between them, so a
    // stale or over-broad request cannot crowd out what tonight's planner
    // chose; a requested file beyond that is clipped (head + tail, marked),
    // never silently dropped.
    let requested_ceiling = cfg.budget_bytes / 2;
    let mut spent_requested = 0usize;
    for f in files {
        let Some(text) = &f.text else {
            records.push(FileRecord {
                path: f.path.clone(),
                bytes: 0,
                clipped: false,
                lines: f.lines,
                status: "unreadable".into(),
                requested: f.requested,
            });
            continue;
        };
        if has_secret_content(text) {
            records.push(FileRecord {
                path: f.path.clone(),
                bytes: 0,
                clipped: false,
                lines: f.lines,
                status: "withheld-key-material".into(),
                requested: f.requested,
            });
            continue;
        }
        if spent >= cfg.budget_bytes || (f.requested && spent_requested >= requested_ceiling) {
            records.push(FileRecord {
                path: f.path.clone(),
                bytes: 0,
                clipped: false,
                lines: f.lines,
                status: "over-budget".into(),
                requested: f.requested,
            });
            continue;
        }
        let (body, header) = match f.lines {
            Some(r) => {
                let (slice, total) = slice_lines(text, r);
                let end = r.end.min(total);
                (slice, format!("lines {}-{} of {}", r.start, end, total))
            }
            None => (
                text.clone(),
                format!("{} lines, {} bytes", text.lines().count(), text.len()),
            ),
        };
        let mut cap = cfg.file_cap_bytes.min(cfg.budget_bytes - spent);
        if f.requested {
            cap = cap.min(requested_ceiling - spent_requested);
        }
        let (body, clipped) = clip_file(&body, cap);
        let body = redact(&body);
        spent += body.len();
        if f.requested {
            spent_requested += body.len();
        }
        let tag = if f.requested {
            " — requested by the previous night"
        } else {
            ""
        };
        out.push_str(&format!(
            "### `{}` ({}{})\n```\n{}\n```\n\n",
            f.path, header, tag, body
        ));
        records.push(FileRecord {
            path: f.path.clone(),
            bytes: body.len(),
            clipped,
            lines: f.lines,
            status: "inlined".into(),
            requested: f.requested,
        });
    }
    let omitted: Vec<&str> = records
        .iter()
        .filter(|r| r.status == "over-budget")
        .map(|r| r.path.as_str())
        .collect();
    if !omitted.is_empty() {
        out.push_str(&format!(
            "(Not inlined — source budget reached: {})\n",
            omitted.join(", ")
        ));
    }
    (out, records)
}

/// The closing line of the section when anything was requested: which
/// requests were honoured and which were not, so the model does not ask the
/// operator for a file the engine has already read or already refused.
pub fn requested_note(honoured: &[String], refused: &[(String, String)]) -> String {
    let mut out = String::from("\nRequested by the previous night: ");
    if honoured.is_empty() {
        out.push_str("none honoured");
    } else {
        out.push_str(&format!("honoured {}", honoured.join(", ")));
    }
    if !refused.is_empty() {
        let list = refused
            .iter()
            .map(|(p, why)| format!("{p} ({why})"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("; not inlined {list}"));
    }
    out.push_str(
        ". Do not ask the operator for source files: list what you still need under \
         **Sources wanted next night** and the engine reads it next time.\n",
    );
    out
}

/// Everything [`gather`] needs besides the model. Build it with
/// [`GatherInput::new`] and [`GatherInput::with_requested`]; the struct is
/// `non_exhaustive` so a later field is not a breaking change.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GatherInput<'a> {
    /// The checkout to read from.
    pub repo: &'a Path,
    /// The commit read (and later built from).
    pub commit: &'a str,
    /// Tonight's deep slot.
    pub deep: &'a str,
    /// Tonight's scans, comma-joined.
    pub scans: &'a str,
    /// Required evaluator command lines.
    pub commands: &'a [String],
    /// The evidence pack text.
    pub evidence: &'a str,
    /// Budgets.
    pub cfg: &'a SourceConfig,
    /// Extra denied paths.
    pub deny: &'a Denylist,
    /// Paths the previous night (or the operator) asked to see tonight.
    /// Honoured first, capped at [`requested_slots`] and half the byte budget.
    pub requested: &'a [SourceRequest],
}

impl<'a> GatherInput<'a> {
    /// A gather input with no requested paths.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: &'a Path,
        commit: &'a str,
        deep: &'a str,
        scans: &'a str,
        commands: &'a [String],
        evidence: &'a str,
        cfg: &'a SourceConfig,
        deny: &'a Denylist,
    ) -> Self {
        Self {
            repo,
            commit,
            deep,
            scans,
            commands,
            evidence,
            cfg,
            deny,
            requested: &[],
        }
    }

    /// The same input, honouring `requested` first.
    pub fn with_requested(mut self, requested: &'a [SourceRequest]) -> Self {
        self.requested = requested;
        self
    }
}

/// One completion with an explicit token budget.
async fn ask(m: &dyn Llm, prompt: &str, max_tokens: u32) -> Result<String, LlmError> {
    let req = LlmRequest::new(prompt, max_tokens);
    m.complete(&req).await
}

/// Ask `llm`, then `fallback` if the first call fails, for the planner reply.
async fn plan_with(
    llm: &dyn Llm,
    fallback: Option<&dyn Llm>,
    prompt: &str,
) -> (Vec<SourceRequest>, String) {
    let tokens = |m: &dyn Llm| m.identity().max_tokens.clamp(2048, 4096);
    match ask(llm, prompt, tokens(llm)).await {
        Ok(r) => match parse_source_plan(&r) {
            Some(p) => (p, "ok".to_string()),
            None => (Vec::new(), "unparseable".to_string()),
        },
        Err(e) => match fallback {
            Some(fb) => match ask(fb, prompt, tokens(fb))
                .await
                .ok()
                .and_then(|r| parse_source_plan(&r))
            {
                Some(p) => (p, "ok-fallback".to_string()),
                None => (Vec::new(), format!("failed: {e}")),
            },
            None => (Vec::new(), format!("failed: {e}")),
        },
    }
}

/// Full tree-read stage: plan → validate → read → render. `None` when
/// disabled, when the tree cannot be listed, or when nothing was readable;
/// the night then runs exactly as before.
///
/// `redact` is applied to the evidence shown to the planner and to every
/// inlined file body.
pub async fn gather(
    llm: &dyn Llm,
    fallback: Option<&dyn Llm>,
    input: &GatherInput<'_>,
    redact: &dyn Fn(&str) -> String,
) -> Option<(String, SourceRecord)> {
    let GatherInput {
        repo,
        commit,
        deep,
        scans,
        commands,
        evidence,
        cfg,
        deny,
        requested,
    } = *input;
    if !cfg.enabled {
        return None;
    }
    let files = tracked_files(repo, commit)?;
    let tracked: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let oversize: HashSet<&str> = files
        .iter()
        .filter(|f| f.size > MAX_READABLE_BYTES)
        .map(|f| f.path.as_str())
        .collect();

    // Requested paths are validated before the planner runs, so the planner
    // is told exactly which of them it need not spend picks on.
    let reserve = requested_slots(cfg.max_files);
    let mut honoured_requests: Vec<SourceRequest> = Vec::new();
    let mut requested_refused: Vec<(String, String)> = Vec::new();
    for (i, r) in requested.iter().enumerate() {
        let key = request_key(&r.path, r.lines);
        if i >= reserve {
            requested_refused.push((key, "over-cap".into()));
            continue;
        }
        match validate_path_with(&r.path, &tracked, deny) {
            Ok(path) if oversize.contains(path.as_str()) => {
                requested_refused.push((key, "oversize".into()))
            }
            Ok(path) if is_binary_ext(&path) => requested_refused.push((key, "denied".into())),
            Ok(path) => {
                if !honoured_requests.iter().any(|h| h.path == path) {
                    honoured_requests.push(SourceRequest {
                        path,
                        lines: r.lines,
                    });
                }
            }
            Err(why) => requested_refused.push((key, format!("{why:?}").to_lowercase())),
        }
    }
    let evidence_tail = tail_chars(evidence, PLANNER_EVIDENCE_CHARS);
    let prompt = source_plan_prompt(
        deep,
        scans,
        commands,
        &redact(evidence_tail),
        &index_listing(&files, cfg.index_cap_chars, deny),
        cfg.max_files,
        &honoured_requests,
    );
    let (planned, planner_status) = plan_with(llm, fallback, &prompt).await;
    if planned.is_empty() {
        warn!(status = %planner_status, "source planner gave no files — using evidence-mentioned paths only");
    }
    let mentioned: Vec<String> = mentioned_paths(
        &format!("{}\n{}", commands.join("\n"), evidence),
        &tracked,
        deny,
    )
    .into_iter()
    .filter(|p| !oversize.contains(p.as_str()))
    .collect();
    let (selected, rejected) = select(
        &honoured_requests,
        &planned,
        &mentioned,
        &tracked,
        cfg.max_files,
        deny,
    );
    if selected.is_empty() {
        info!("tree-read selected no files — prompt carries no source section");
        return None;
    }
    let fetched: Vec<FetchedFile> = selected
        .iter()
        .map(|r| FetchedFile {
            path: r.path.clone(),
            lines: r.lines,
            text: if oversize.contains(r.path.as_str()) {
                None
            } else {
                read_blob(repo, commit, &r.path)
            },
            requested: honoured_requests.iter().any(|h| h.path == r.path),
        })
        .collect();
    let (mut section, records) = render_section(commit, &fetched, cfg, redact);
    let inlined = records.iter().filter(|r| r.status == "inlined").count();
    // Every requested path ends in exactly one of the two lists.
    let mut requested_honoured: Vec<String> = Vec::new();
    for rec in records.iter().filter(|r| r.requested) {
        let key = request_key(&rec.path, rec.lines);
        if rec.status == "inlined" {
            requested_honoured.push(key);
        } else {
            requested_refused.push((key, rec.status.clone()));
        }
    }
    if !requested.is_empty() {
        section.push_str(&requested_note(&requested_honoured, &requested_refused));
    }
    info!(
        planner = %planner_status,
        selected = selected.len(),
        inlined,
        rejected = rejected.len(),
        requested = requested.len(),
        requested_honoured = requested_honoured.len(),
        bytes = section.len(),
        "tree-read source section built"
    );
    if inlined == 0 {
        return None;
    }
    let record = SourceRecord {
        commit: commit.to_string(),
        planner: planner_status,
        files: records,
        rejected: rejected
            .into_iter()
            .map(|(p, r)| (p, format!("{r:?}")))
            .collect(),
        section_bytes: section.len(),
        requested_honoured,
        requested_refused,
    };
    Some((section, record))
}

fn tail_chars(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut start = s.len() - n;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

// ---------------------------------------------------------------------------
// Repair pass
// ---------------------------------------------------------------------------

/// Whether the one-shot repair pass should run: the model declared ACCEPT
/// (strictly parsed) but emitted no usable ```dream-patch block.
pub fn needs_repair(claimed_accept: bool, patch_present: bool) -> bool {
    claimed_accept && !patch_present
}

/// The repair prompt: the model's own report plus the same source section,
/// asking for nothing but the diff.
pub fn repair_prompt(report: &str, source_section: Option<&str>) -> String {
    format!(
        "Your report below declares VERDICT: ACCEPT but contains no ```dream-patch block, so the \
         engine has nothing to apply and re-evaluate — the ACCEPT will be vetoed.\n\n\
         Reply with EXACTLY ONE of:\n\
         1. the candidate change as one git-apply-able unified diff (paths relative to the repo \
         root, `a/` and `b/` prefixes, context lines copied verbatim from the source shown) inside\n\
         ```dream-patch\n<diff>\n```\n\
         2. a single line `NO-PATCH: <reason>` if the finding needs no code change or you cannot \
         write the diff from the source available.\n\n\
         No other text.\n{}\n\n---\n\n# Your report\n\n{}",
        source_section.unwrap_or("\n(No source section was available tonight.)\n"),
        report
    )
}

/// What the repair pass produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", content = "detail", rename_all = "kebab-case")]
pub enum RepairOutcome {
    /// A non-empty diff was recovered.
    Patch(String),
    /// The model said there is no patch, with its reason.
    NoPatch(String),
    /// Neither a diff nor a NO-PATCH line.
    Absent,
    /// The completion call itself failed.
    LlmError(String),
}

/// Classify a repair reply.
pub fn parse_repair(reply: &str) -> RepairOutcome {
    if let Some(p) = dream_gate::patch::extract_patch(reply) {
        return RepairOutcome::Patch(p);
    }
    for line in reply.lines() {
        if let Some(reason) = line.trim().strip_prefix("NO-PATCH:") {
            return RepairOutcome::NoPatch(reason.trim().to_string());
        }
    }
    RepairOutcome::Absent
}

/// Receipt of a repair pass, persisted as `repair.json` (missing patch) or
/// `apply-repair.json` (patch that would not apply).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairRecord {
    /// Whether the pass ran.
    pub ran: bool,
    /// What the pass produced.
    pub outcome: RepairOutcome,
    /// Size of the recovered patch, 0 if none.
    pub patch_bytes: usize,
    /// Model that answered.
    pub model: String,
    /// sha256 of the exact prompt sent, so the witness can show what the
    /// model was given. Empty on records written before it existed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt_digest: String,
    /// sha256 of the patch that failed to apply (apply-repair only). A record
    /// is reused on restart only for this exact patch, so a re-run that wrote
    /// a different patch never inherits an older repair.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub failed_patch_digest: String,
}

impl RepairRecord {
    /// Whether this apply-repair record was made for `patch`.
    pub fn repairs(&self, patch: &str) -> bool {
        !self.failed_patch_digest.is_empty()
            && self.failed_patch_digest == dream_gate::manifest::digest(patch.as_bytes())
    }
}

/// Run the repair pass (primary, then fallback provider).
pub async fn repair(
    llm: &dyn Llm,
    fallback: Option<&dyn Llm>,
    report: &str,
    source_section: Option<&str>,
) -> RepairRecord {
    run_repair(llm, fallback, repair_prompt(report, source_section)).await
}

/// `path` and 1-based line of the first hunk git refused, from a
/// `git apply` error (`error: patch failed: <path>:<line>`).
pub fn failing_hunk(apply_error: &str) -> Option<(String, usize)> {
    let rest = apply_error.split("patch failed: ").nth(1)?;
    let spec = rest.split_whitespace().next()?;
    let (path, line) = spec.rsplit_once(':')?;
    Some((path.to_string(), line.parse().ok()?))
}

/// The real base-revision text around the hunk git refused: `context` lines
/// either side of `line`, verbatim, with a header naming the range. None when
/// the file cannot be read at `rev`.
pub fn failing_excerpt(
    repo: &Path,
    rev: &str,
    path: &str,
    line: usize,
    context: usize,
) -> Option<String> {
    let text = read_blob(repo, rev, path)?;
    let start = line.saturating_sub(context).max(1);
    let range = LineRange {
        start,
        end: line + context,
    };
    let (body, total) = slice_lines(&text, range);
    let end = (line + context).min(total);
    Some(format!(
        "`{path}` lines {start}-{end} of {total}, exactly as committed:\n\n````text\n{body}\n````"
    ))
}

/// The apply-repair prompt: the patch the engine could not apply, git's own
/// error, and the real text of the file where it failed. Deliberately carries
/// no evaluator receipts — the repair may fix *where* a change lands, never
/// learn what the gate will measure.
pub fn apply_repair_prompt(
    report: &str,
    patch: &str,
    apply_error: &str,
    excerpt: Option<&str>,
) -> String {
    format!(
        "The ```dream-patch block in your report below did not apply to the commit you were shown. \
         `git apply` said:\n\n```text\n{}\n```\n\n\
         Usual causes: a context or `-` line that is not in the file verbatim (wrong indentation, \
         a line skipped or invented), or a hunk written from memory of code you were not shown.\n\n\
         {}\n\n\
         Reply with EXACTLY ONE of:\n\
         1. the SAME change, corrected so it applies, as one git-apply-able unified diff (`diff --git` \
         headers, `a/` and `b/` prefixes, every context and `-` line copied verbatim from the file) inside\n\
         ```dream-patch\n<diff>\n```\n\
         2. a single line `NO-PATCH: <reason>` if you cannot write it from the text available.\n\n\
         Do not change what the patch does. No other text.\n\n---\n\n# The patch that failed\n\n\
         ```diff\n{}\n```\n\n---\n\n# Your report\n\n{}",
        apply_error.trim(),
        excerpt.unwrap_or("(The engine could not read the file at the failing hunk.)"),
        patch.trim_end(),
        report
    )
}

/// Run the apply-repair pass once (primary, then fallback provider).
pub async fn apply_repair(
    llm: &dyn Llm,
    fallback: Option<&dyn Llm>,
    failed_patch: &str,
    prompt: String,
) -> RepairRecord {
    let mut record = run_repair(llm, fallback, prompt).await;
    record.failed_patch_digest = dream_gate::manifest::digest(failed_patch.as_bytes());
    record
}

async fn run_repair(llm: &dyn Llm, fallback: Option<&dyn Llm>, prompt: String) -> RepairRecord {
    let prompt_digest = dream_gate::manifest::digest(prompt.as_bytes());
    let (outcome, model) = match ask(llm, &prompt, llm.identity().max_tokens).await {
        Ok(r) => (parse_repair(&r), llm.identity().model),
        Err(e) => match fallback {
            Some(fb) => match ask(fb, &prompt, fb.identity().max_tokens).await {
                Ok(r) => (parse_repair(&r), fb.identity().model),
                Err(e2) => (
                    RepairOutcome::LlmError(format!("primary: {e}; fallback: {e2}")),
                    fb.identity().model,
                ),
            },
            None => (RepairOutcome::LlmError(e.to_string()), llm.identity().model),
        },
    };
    let patch_bytes = match &outcome {
        RepairOutcome::Patch(p) => p.len(),
        _ => 0,
    };
    RepairRecord {
        ran: true,
        outcome,
        patch_bytes,
        model,
        prompt_digest,
        failed_patch_digest: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_material_is_detected() {
        let nsec = format!("nsec1{}", "q".repeat(58));
        assert!(has_secret_content(&format!("fixture = \"{nsec}\"")));
        assert!(!has_secret_content("the nsec1 prefix marks a secret key"));
        assert!(has_secret_content("-----BEGIN EC PRIVATE KEY-----\nabc"));
        assert!(has_secret_content(&format!(
            "APP_PRIVKEY_HEX={}",
            "ab".repeat(32)
        )));
        // A 64-hex event id or pubkey on a line with no key-looking name is fine.
        assert!(!has_secret_content(&format!(
            "event id {}",
            "ab".repeat(32)
        )));
        assert!(!has_secret_content("fn secret_path() -> &str { \"x\" }"));
    }

    #[test]
    fn key_material_files_are_withheld_whole() {
        let nsec = format!("nsec1{}", "z".repeat(58));
        let files = vec![
            FetchedFile {
                path: "a.rs".into(),
                lines: None,
                text: Some("fn a() {}".into()),
                requested: false,
            },
            FetchedFile {
                path: "fixture.rs".into(),
                lines: None,
                text: Some(format!("const K: &str = \"{nsec}\";")),
                requested: false,
            },
        ];
        let (section, records) =
            render_section("abc", &files, &SourceConfig::default(), &|s: &str| {
                s.to_string()
            });
        assert!(!section.contains(&nsec));
        assert!(!section.contains("fixture.rs"));
        assert_eq!(records[1].status, "withheld-key-material");
        assert_eq!(records[0].status, "inlined");
    }

    fn tracked<'a>(paths: &'a [&'a str]) -> HashSet<&'a str> {
        paths.iter().copied().collect()
    }

    #[test]
    fn parses_ls_tree_blobs_and_skips_submodules() {
        let text = "100644 blob abc123     120\tsrc/lib.rs\n\
                    160000 commit def456       -\tvendor/sub\n\
                    100755 blob 0a1b2c    9000\tscripts/check.sh\n\
                    garbage line\n";
        let files = parse_ls_tree(text);
        assert_eq!(
            files,
            vec![
                TrackedFile {
                    path: "src/lib.rs".into(),
                    size: 120
                },
                TrackedFile {
                    path: "scripts/check.sh".into(),
                    size: 9000
                },
            ]
        );
    }

    #[test]
    fn validation_refuses_traversal_absolute_denied_and_untracked() {
        let t = tracked(&[
            "src/lib.rs",
            ".env.local",
            "config/secrets.toml",
            "certs/server.pem",
        ]);
        assert_eq!(validate_path("src/lib.rs", &t), Ok("src/lib.rs".into()));
        assert_eq!(validate_path("./src/lib.rs", &t), Ok("src/lib.rs".into()));
        assert_eq!(validate_path("`src/lib.rs`", &t), Ok("src/lib.rs".into()));
        assert_eq!(validate_path("", &t), Err(PathRejection::Empty));
        assert_eq!(
            validate_path("/etc/passwd", &t),
            Err(PathRejection::Absolute)
        );
        assert_eq!(validate_path("~/x", &t), Err(PathRejection::Absolute));
        assert_eq!(validate_path("C:\\x", &t), Err(PathRejection::Absolute));
        assert_eq!(
            validate_path("src/../../etc/passwd", &t),
            Err(PathRejection::Traversal)
        );
        assert_eq!(validate_path("..\\x", &t), Err(PathRejection::Traversal));
        assert_eq!(validate_path(".env.local", &t), Err(PathRejection::Denied));
        assert_eq!(
            validate_path("config/secrets.toml", &t),
            Err(PathRejection::Denied)
        );
        assert_eq!(
            validate_path("certs/server.pem", &t),
            Err(PathRejection::Denied)
        );
        assert_eq!(
            validate_path("src/missing.rs", &t),
            Err(PathRejection::NotInTree)
        );
    }

    #[test]
    fn denylist_covers_key_material() {
        for p in [
            ".env",
            "deploy/.env.production",
            "keys/id_ed25519",
            "a/b/.npmrc",
            "x/credentials.json",
            "tls/server.key",
            "vault.kdbx",
        ] {
            assert!(is_denied(p), "{p} must be denied");
        }
        for p in ["src/keyboard.rs", "docs/environment.md", "src/lib.rs"] {
            assert!(!is_denied(p), "{p} must be allowed");
        }
    }

    #[test]
    fn host_denylist_extends_the_built_in_one() {
        let d = Denylist::new(["session-key.txt", "private/"]);
        assert!(!is_denied("x/session-key.txt"), "not built in");
        assert!(d.denies("x/session-key.txt"));
        assert!(d.denies("private/a.md") && d.denies("pkg/Private/a.md"));
        assert!(!d.denies("privateer/a.md"));
        assert!(d.denies(".env"), "built-ins still apply");
        let t = tracked(&["private/a.md", "src/lib.rs"]);
        assert_eq!(
            validate_path_with("private/a.md", &t, &d),
            Err(PathRejection::Denied)
        );
        assert_eq!(validate_path("private/a.md", &t), Ok("private/a.md".into()));
    }

    #[test]
    fn plan_parsing_accepts_both_shapes_and_fences() {
        let reply = "Sure.\n```json\n{\"files\":[{\"path\":\"src/a.rs\",\"lines\":\"10-40\"},\"src/b.rs\",{\"path\":\"src/c.rs\",\"lines\":\"bad\"}]}\n```";
        let plan = parse_source_plan(reply).unwrap();
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].lines, Some(LineRange { start: 10, end: 40 }));
        assert_eq!(
            plan[1],
            SourceRequest {
                path: "src/b.rs".into(),
                lines: None
            }
        );
        assert_eq!(plan[2].lines, None);
        assert!(parse_source_plan("no json here").is_none());
        assert!(parse_source_plan("} {").is_none());
    }

    #[test]
    fn line_ranges_parse_and_reject_nonsense() {
        assert_eq!(
            LineRange::parse("5-9"),
            Some(LineRange { start: 5, end: 9 })
        );
        assert_eq!(
            LineRange::parse("5:9"),
            Some(LineRange { start: 5, end: 9 })
        );
        assert_eq!(
            LineRange::parse("42"),
            Some(LineRange { start: 42, end: 42 })
        );
        assert_eq!(LineRange::parse("0-3"), None);
        assert_eq!(LineRange::parse("9-5"), None);
        assert_eq!(LineRange::parse("x"), None);
    }

    #[test]
    fn mentioned_paths_strip_locations_and_ignore_untracked() {
        let t = tracked(&["src/main.rs", "scripts/check.sh", "package.json", ".env"]);
        let text =
            "error[E0425]: at src/main.rs:12:5\n run `bash scripts/check.sh`; see (package.json). \
                    also src/main.rs again and ./.env and nowhere/else.rs";
        assert_eq!(
            mentioned_paths(text, &t, &Denylist::default()),
            vec![
                "src/main.rs".to_string(),
                "scripts/check.sh".into(),
                "package.json".into()
            ]
        );
    }

    #[test]
    fn select_prefers_planner_dedups_and_caps() {
        let t = tracked(&["a.rs", "b.rs", "c.rs", "d.png", ".env"]);
        let planned = vec![
            SourceRequest {
                path: "b.rs".into(),
                lines: None,
            },
            SourceRequest {
                path: "../x".into(),
                lines: None,
            },
            SourceRequest {
                path: ".env".into(),
                lines: None,
            },
            SourceRequest {
                path: "d.png".into(),
                lines: None,
            },
        ];
        let mentioned = vec!["b.rs".to_string(), "a.rs".into(), "c.rs".into()];
        let (kept, rejected) = select(&[], &planned, &mentioned, &t, 2, &Denylist::default());
        assert_eq!(
            kept.iter().map(|k| k.path.as_str()).collect::<Vec<_>>(),
            vec!["b.rs", "a.rs"]
        );
        assert!(rejected.contains(&("../x".into(), PathRejection::Traversal)));
        assert!(rejected.contains(&(".env".into(), PathRejection::Denied)));
        assert!(rejected.contains(&("d.png".into(), PathRejection::Denied)));
    }

    #[test]
    fn index_is_shallow_first_filtered_and_capped() {
        let files = vec![
            TrackedFile {
                path: "src/deep/x.rs".into(),
                size: 10,
            },
            TrackedFile {
                path: "README.md".into(),
                size: 10,
            },
            TrackedFile {
                path: "img/logo.png".into(),
                size: 10,
            },
            TrackedFile {
                path: ".env".into(),
                size: 10,
            },
            TrackedFile {
                path: "huge.json".into(),
                size: 5_000_000,
            },
            TrackedFile {
                path: "src/lib.rs".into(),
                size: 10,
            },
        ];
        let full = index_listing(&files, 10_000, &Denylist::default());
        let order: Vec<&str> = full.lines().map(|l| l.split(' ').next().unwrap()).collect();
        assert_eq!(order, vec!["README.md", "src/lib.rs", "src/deep/x.rs"]);
        let clipped = index_listing(&files, 20, &Denylist::default());
        assert!(clipped.contains("more tracked paths not listed"));
    }

    #[test]
    fn slicing_and_clipping_mark_what_is_missing() {
        let text = (1..=10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            slice_lines(&text, LineRange { start: 3, end: 4 }),
            ("line3\nline4".into(), 10)
        );
        assert_eq!(
            slice_lines(&text, LineRange { start: 9, end: 99 }).0,
            "line9\nline10"
        );
        assert_eq!(slice_lines(&text, LineRange { start: 20, end: 30 }).0, "");
        let big = "é".repeat(5_000);
        let (c, clipped) = clip_file(&big, 1_000);
        assert!(clipped);
        assert!(c.contains("elided by the engine"));
        assert!(c.len() < big.len());
        assert_eq!(clip_file("tiny", 100), ("tiny".into(), false));
    }

    #[test]
    fn section_formatting_budget_and_unreadable() {
        let cfg = SourceConfig {
            budget_bytes: 40,
            file_cap_bytes: 30,
            ..SourceConfig::default()
        };
        let files = vec![
            FetchedFile {
                path: "a.rs".into(),
                lines: None,
                text: Some("fn a() {}\n".into()),
                requested: false,
            },
            FetchedFile {
                path: "gone.rs".into(),
                lines: None,
                text: None,
                requested: false,
            },
            FetchedFile {
                path: "b.rs".into(),
                lines: Some(LineRange { start: 2, end: 2 }),
                text: Some("x\ny\nz".into()),
                requested: false,
            },
            FetchedFile {
                path: "c.rs".into(),
                lines: None,
                text: Some("c".repeat(100)),
                requested: false,
            },
            FetchedFile {
                path: "d.rs".into(),
                lines: None,
                text: Some("d".into()),
                requested: false,
            },
        ];
        let redact = |s: &str| s.replace("fn", "FN");
        let (section, records) = render_section("abc123", &files, &cfg, &redact);
        assert!(section.starts_with("\n## Source (from `abc123`)"));
        assert!(section.contains("### `a.rs` (1 lines, 10 bytes)\n```\nFN a() {}\n"));
        assert!(section.contains("### `b.rs` (lines 2-2 of 3)\n```\ny\n```"));
        assert_eq!(records[1].status, "unreadable");
        assert_eq!(records[3].status, "inlined");
        assert!(records[3].clipped);
        assert_eq!(records[4].status, "over-budget");
        assert!(section.contains("source budget reached: d.rs"));
    }

    #[test]
    fn repair_runs_only_for_accept_without_patch() {
        assert!(needs_repair(true, false));
        assert!(!needs_repair(true, true));
        assert!(!needs_repair(false, false));
        assert!(!needs_repair(false, true));
    }

    #[test]
    fn repair_reply_classification() {
        let diff = "```dream-patch\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n```";
        assert!(matches!(parse_repair(diff), RepairOutcome::Patch(p) if p.contains("+b")));
        assert_eq!(
            parse_repair("NO-PATCH: pure measurement"),
            RepairOutcome::NoPatch("pure measurement".into())
        );
        assert_eq!(parse_repair("I think it works"), RepairOutcome::Absent);
        let prompt = repair_prompt("VERDICT: ACCEPT", Some("## Source (from `r`)"));
        assert!(prompt.contains("```dream-patch"));
        assert!(prompt.contains("NO-PATCH:"));
        assert!(prompt.contains("## Source (from `r`)"));
    }

    #[test]
    fn failing_hunk_is_read_from_git_apply_errors() {
        let e =
            "apply --recount: git apply failed: error: patch failed: scripts/gen.mjs:18\nerror: x";
        assert_eq!(failing_hunk(e), Some(("scripts/gen.mjs".to_string(), 18)));
        let e = "git apply failed: error: patch failed: packages/cli/src/tui.ts:49\nerror: packages/cli/src/tui.ts: patch does not apply";
        assert_eq!(
            failing_hunk(e),
            Some(("packages/cli/src/tui.ts".to_string(), 49))
        );
        assert_eq!(
            failing_hunk("error: corrupt patch at /tmp/x.patch:14"),
            None
        );
    }

    #[test]
    fn apply_repair_prompt_carries_the_error_and_file_but_no_receipts() {
        let prompt = apply_repair_prompt(
            "# report\nVERDICT: ACCEPT",
            "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-x\n+y",
            "error: patch failed: f:1",
            Some("`f` lines 1-3 of 3, exactly as committed:\n\n````text\na\nb\nc\n````"),
        );
        assert!(prompt.contains("error: patch failed: f:1"));
        assert!(prompt.contains("````text\na\nb\nc\n````"));
        assert!(prompt.contains("+y"));
        assert!(prompt.contains("```dream-patch"));
        assert!(prompt.contains("NO-PATCH:"));
        assert!(!prompt.to_lowercase().contains("receipt"));
        assert!(!prompt.to_lowercase().contains("evaluator"));
    }

    #[test]
    fn failing_excerpt_is_verbatim_and_clamped() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path();
        let git = |a: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(a)
                .output()
                .unwrap()
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        let body: String = (1..=10).map(|i| format!("    line {i}\n")).collect();
        std::fs::write(repo.join("f.txt"), &body).unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "c"]);
        let ex = failing_excerpt(repo, "HEAD", "f.txt", 2, 3).unwrap();
        assert!(ex.contains("lines 1-5 of 10"), "{ex}");
        assert!(
            ex.contains("    line 1\n    line 2"),
            "indentation kept: {ex}"
        );
        assert!(!ex.contains("line 6"));
        assert!(failing_excerpt(repo, "HEAD", "missing.txt", 1, 3).is_none());
    }

    #[test]
    fn repair_record_round_trips_for_restart_reuse() {
        let r = RepairRecord {
            ran: true,
            outcome: RepairOutcome::Patch("--- a/f\n+++ b/f\n".into()),
            patch_bytes: 16,
            model: "m".into(),
            prompt_digest: "abc".into(),
            failed_patch_digest: dream_gate::manifest::digest(b"old patch"),
        };
        let back: RepairRecord = serde_json::from_slice(&serde_json::to_vec(&r).unwrap()).unwrap();
        assert_eq!(back.outcome, r.outcome);
        assert_eq!(back.prompt_digest, "abc");
        // Reused only for the patch it repaired: a same-day re-run with a new
        // patch must not inherit it, nor may a record with no digest.
        assert!(back.repairs("old patch"));
        assert!(!back.repairs("new patch"));
        let legacy = RepairRecord {
            failed_patch_digest: String::new(),
            ..back
        };
        assert!(!legacy.repairs("old patch"));
    }

    #[test]
    fn reads_blobs_at_a_commit_not_the_worktree() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path();
        let git = |a: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(a)
                .output()
                .unwrap()
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("f.txt"), "committed\n").unwrap();
        std::fs::write(repo.join("bin.dat"), [0u8, 1, 2]).unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "c"]);
        let rev = String::from_utf8(git(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        std::fs::write(repo.join("f.txt"), "dirty worktree\n").unwrap();
        assert_eq!(
            read_blob(repo, &rev, "f.txt").as_deref(),
            Some("committed\n")
        );
        assert_eq!(read_blob(repo, &rev, "bin.dat"), None);
        assert_eq!(read_blob(repo, &rev, "missing"), None);
        let files = tracked_files(repo, &rev).unwrap();
        assert!(files.iter().any(|f| f.path == "f.txt" && f.size == 10));
    }

    // ---- async stages against a fake model and a real repository ----

    use crate::seams::{BoxFut, Llm, LlmError, LlmRequest};
    use dream_gate::manifest::ModelIdentity;
    use std::sync::Mutex;

    struct Fake {
        replies: Mutex<Vec<Result<String, LlmError>>>,
        seen: Mutex<Vec<(String, u32)>>,
    }
    impl Fake {
        fn new(replies: Vec<Result<String, LlmError>>) -> Self {
            Self {
                replies: Mutex::new(replies),
                seen: Mutex::new(vec![]),
            }
        }
    }
    impl Llm for Fake {
        fn complete<'a>(&'a self, req: &'a LlmRequest) -> BoxFut<'a, Result<String, LlmError>> {
            self.seen
                .lock()
                .unwrap()
                .push((req.prompt.clone(), req.max_tokens));
            let r = self.replies.lock().unwrap().remove(0);
            Box::pin(async move { r })
        }
        fn identity(&self) -> ModelIdentity {
            ModelIdentity {
                provider: "fake".into(),
                model: "fake-1".into(),
                max_tokens: 1000,
                fallback: None,
            }
        }
    }

    fn repo_with(files: &[(&str, &str)]) -> (tempfile::TempDir, String) {
        let d = tempfile::tempdir().unwrap();
        let g = |a: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(d.path())
                .args(a)
                .output()
                .unwrap()
        };
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        for (p, c) in files {
            let full = d.path().join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, c).unwrap();
        }
        g(&["add", "-A"]);
        g(&["commit", "-qm", "c"]);
        let rev = String::from_utf8(g(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        (d, rev)
    }

    #[test]
    fn gather_inlines_planned_files_and_clamps_the_planner_budget() {
        let (d, rev) = repo_with(&[("src/a.rs", "fn a() {}\n"), ("session-key.txt", "x")]);
        let fake = Fake::new(vec![Ok(
            r#"{"files":[{"path":"src/a.rs"},"session-key.txt","../x"]}"#.into(),
        )]);
        let deny = Denylist::new(["session-key.txt"]);
        let cmds: Vec<String> = vec![];
        let cfg = SourceConfig::default();
        let input = GatherInput::new(d.path(), &rev, "d", "s", &cmds, "", &cfg, &deny);
        let (section, rec) =
            futures::executor::block_on(gather(&fake, None, &input, &|s: &str| s.to_string()))
                .expect("a section");
        assert!(section.contains("fn a() {}"));
        assert!(!section.contains("session-key"));
        assert_eq!(rec.planner, "ok");
        assert_eq!(rec.rejected.len(), 2, "{:?}", rec.rejected);
        assert_eq!(fake.seen.lock().unwrap()[0].1, 2048, "clamped up from 1000");
    }

    #[test]
    fn gather_falls_back_and_then_degrades_to_evidence_mentions() {
        let (d, rev) = repo_with(&[("src/a.rs", "fn a() {}\n"), ("src/b.rs", "fn b() {}\n")]);
        let primary = Fake::new(vec![Err(LlmError::Empty)]);
        let fb = Fake::new(vec![Ok(r#"{"files":["src/b.rs"]}"#.into())]);
        let cmds: Vec<String> = vec![];
        let deny = Denylist::default();
        let cfg = SourceConfig::default();
        let input = GatherInput::new(
            d.path(),
            &rev,
            "d",
            "s",
            &cmds,
            "error at src/a.rs:3",
            &cfg,
            &deny,
        );
        let (section, rec) =
            futures::executor::block_on(gather(&primary, Some(&fb), &input, &|s: &str| {
                s.to_string()
            }))
            .unwrap();
        assert_eq!(rec.planner, "ok-fallback");
        assert!(section.contains("fn b()") && section.contains("fn a()"));
        // Both fail: the evidence-mentioned file still arrives.
        let primary = Fake::new(vec![Err(LlmError::Empty)]);
        let (section, rec) =
            futures::executor::block_on(gather(&primary, None, &input, &|s: &str| s.to_string()))
                .unwrap();
        assert!(rec.planner.starts_with("failed"));
        assert!(section.contains("fn a()") && !section.contains("fn b()"));
        // Disabled: nothing, and no model call.
        let off = SourceConfig {
            enabled: false,
            ..SourceConfig::default()
        };
        let silent = Fake::new(vec![]);
        let mut input = input;
        input.cfg = &off;
        assert!(futures::executor::block_on(
            gather(&silent, None, &input, &|s: &str| s.to_string())
        )
        .is_none());
    }

    #[test]
    fn repair_uses_the_fallback_and_records_the_model_that_answered() {
        let primary = Fake::new(vec![Err(LlmError::Api("HTTP 500".into()))]);
        let fb = Fake::new(vec![Ok(
            "```dream-patch\ndiff --git a/x b/x\n+new\n```".into()
        )]);
        let rec = futures::executor::block_on(repair(&primary, Some(&fb), "report", None));
        assert!(matches!(rec.outcome, RepairOutcome::Patch(_)));
        assert_eq!(rec.model, "fake-1");
        assert!(!rec.prompt_digest.is_empty());
        let none = Fake::new(vec![Err(LlmError::Empty)]);
        let rec = futures::executor::block_on(repair(&none, None, "report", None));
        assert!(matches!(rec.outcome, RepairOutcome::LlmError(_)));
        let ap = Fake::new(vec![Ok("NO-PATCH: cannot".into())]);
        let rec = futures::executor::block_on(apply_repair(&ap, None, "old", "p".into()));
        assert_eq!(rec.outcome, RepairOutcome::NoPatch("cannot".into()));
        assert!(rec.repairs("old") && !rec.repairs("other"));
    }

    #[test]
    fn source_request_parse_splits_a_trailing_line_range() {
        assert_eq!(
            SourceRequest::parse("`src/a.rs:10-80`"),
            Some(SourceRequest {
                path: "src/a.rs".into(),
                lines: Some(LineRange { start: 10, end: 80 }),
            })
        );
        assert_eq!(
            SourceRequest::parse("./conf/x:y.toml"),
            Some(SourceRequest {
                path: "conf/x:y.toml".into(),
                lines: None,
            })
        );
        assert_eq!(SourceRequest::parse("  "), None);
    }

    #[test]
    fn select_honours_requested_first_but_reserves_only_half_the_slots() {
        let t = tracked(&["r1.rs", "r2.rs", "r3.rs", "p1.rs", "m1.rs"]);
        let req = |p: &str| SourceRequest {
            path: p.into(),
            lines: None,
        };
        let requested = vec![req("r1.rs"), req("r2.rs"), req("r3.rs")];
        let planned = vec![req("p1.rs")];
        let mentioned = vec!["m1.rs".to_string()];
        let (kept, _) = select(
            &requested,
            &planned,
            &mentioned,
            &t,
            4,
            &Denylist::default(),
        );
        let paths: Vec<&str> = kept.iter().map(|k| k.path.as_str()).collect();
        // Two requested (half of four), then the planner, then the mention; r3 waits.
        assert_eq!(paths, vec!["r1.rs", "r2.rs", "p1.rs", "m1.rs"]);
        assert_eq!(requested_slots(1), 1);
        assert_eq!(requested_slots(12), 6);
    }

    #[test]
    fn render_caps_requested_files_at_half_the_byte_budget() {
        let cfg = SourceConfig {
            budget_bytes: 2_000,
            file_cap_bytes: 2_000,
            ..SourceConfig::default()
        };
        let files = vec![
            FetchedFile {
                path: "req.rs".into(),
                lines: None,
                text: Some("x".repeat(1_800)),
                requested: true,
            },
            FetchedFile {
                path: "plan.rs".into(),
                lines: None,
                text: Some("y".repeat(400)),
                requested: false,
            },
        ];
        let (section, records) = render_section("c", &files, &cfg, &|s: &str| s.to_string());
        assert!(records[0].clipped, "requested file is clipped to its half");
        // At most half the budget of the file's own text survives; the rest is
        // the elision marker, which counts against the budget like any text.
        let body = section.split("```").nth(1).unwrap_or("");
        let kept_x = body.matches("xxxx").count() * 4;
        assert!(kept_x <= 1_000, "{kept_x}");
        assert_eq!(records[1].status, "inlined");
        assert!(!records[1].requested && records[0].requested);
        assert!(section.contains("requested by the previous night"));
    }

    #[test]
    fn planner_prompt_lists_already_supplied_requests() {
        let supplied = vec![SourceRequest {
            path: "src/a.rs".into(),
            lines: Some(LineRange { start: 1, end: 9 }),
        }];
        let p = source_plan_prompt("d", "s", &[], "", "", 12, &supplied);
        assert!(p.contains("Already being read tonight"));
        assert!(p.contains("`src/a.rs` lines 1-9"));
        let p0 = source_plan_prompt("d", "s", &[], "", "", 12, &[]);
        assert!(!p0.contains("Already being read tonight"));
    }

    #[test]
    fn gather_honours_requested_paths_and_records_refusals() {
        let (d, rev) = repo_with(&[("src/a.rs", "fn a() {}\n"), ("src/b.rs", "fn b() {}\n")]);
        // The planner names nothing: without requests the night has no section.
        let fake = Fake::new(vec![Ok(r#"{"files":[]}"#.into())]);
        let cmds: Vec<String> = vec![];
        let cfg = SourceConfig::default();
        let deny = Denylist::default();
        let requested = vec![
            SourceRequest {
                path: "src/b.rs".into(),
                lines: None,
            },
            SourceRequest {
                path: "nowhere.rs".into(),
                lines: None,
            },
            SourceRequest {
                path: "../etc/passwd".into(),
                lines: None,
            },
        ];
        let input = GatherInput::new(d.path(), &rev, "d", "s", &cmds, "", &cfg, &deny)
            .with_requested(&requested);
        let (section, rec) =
            futures::executor::block_on(gather(&fake, None, &input, &|s: &str| s.to_string()))
                .expect("a section from the request alone");
        assert!(section.contains("fn b()") && !section.contains("fn a()"));
        assert!(section.contains("Requested by the previous night: honoured src/b.rs"));
        assert!(section.contains("Sources wanted next night"));
        assert_eq!(rec.requested_honoured, vec!["src/b.rs".to_string()]);
        assert_eq!(
            rec.requested_refused.len(),
            2,
            "{:?}",
            rec.requested_refused
        );
        assert!(rec.files[0].requested);
        // The planner was told what it need not pick.
        assert!(fake.seen.lock().unwrap()[0].0.contains("`src/b.rs`"));
    }
}
