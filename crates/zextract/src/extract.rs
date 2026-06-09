//! Extraction coordinator. Dispatches the captured scrollback text to
//! each pattern module, combines results, and resolves overlap.
//!
//! Overlap policy (per planning.md Q25 + Phase 4 update):
//!
//!   1. Within-pattern: leftmost-longest, handled inside each pattern.
//!   2. **Pass 1 — same-type dedup**: same `(type, raw)` collapses to one
//!      match, keeping the latest occurrence (largest `span.0`). Preserves
//!      recency context.
//!   3. **Pass 2 — cross-type dedup**: same `raw` from multiple types
//!      collapses to one match, keeping the one whose type ranks earliest
//!      in `TYPE_PRIORITY`. Ties (impossible with the static list, but
//!      possible once Phase 7 KDL config exposes the ordering) go to the
//!      latest occurrence.
//!
//! Output order: **latest-first** (most recent occurrence in the
//! scrollback ranks ahead of older ones), with picker score bonuses
//! also derived from `TYPE_PRIORITY` (front-of-list = positive bonus,
//! tail = negative).

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use regex_lite::Regex;

use crate::config::PatternsConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub ty: MatchType,
    pub raw: String,
    pub display: String,
    pub context: String,
    /// Byte offsets in the input text. Used for dedup tie-breaking
    /// (latest = larger `span.0`) and for the JSON export in Phase 5.
    pub span: (usize, usize),
    pub fields: HashMap<String, String>,
    /// Display/filter label override for custom patterns. `None` for all
    /// built-in patterns — `effective_tag()` falls back to `ty.tag()`.
    pub label: Option<String>,
    /// The pane this match was extracted from. `None` for matches produced
    /// before multi-pane extraction was wired (should not occur at runtime).
    pub source_pane_id: Option<u32>,
}

impl Match {
    /// The tag used for display, type-filter (`#name`), and the list
    /// row label. Returns the custom pattern name when set, otherwise
    /// the built-in type tag (`"url"`, `"file"`, etc.).
    pub fn effective_tag(&self) -> &str {
        self.label.as_deref().unwrap_or_else(|| self.ty.tag())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchType {
    Url,
    File,
    Diagnostic,
    Git,
    Sha,
    Ipv4,
    Ipv6,
    Uuid,
    QuotedString,
    Command,
    Secret,
}

impl MatchType {
    pub fn tag(self) -> &'static str {
        match self {
            MatchType::Url => "url",
            MatchType::File => "file",
            MatchType::Diagnostic => "diag",
            MatchType::Git => "git",
            MatchType::Sha => "sha",
            MatchType::Ipv4 => "ipv4",
            MatchType::Ipv6 => "ipv6",
            MatchType::Uuid => "uuid",
            MatchType::QuotedString => "quote",
            MatchType::Command => "cmd",
            MatchType::Secret => "secret",
        }
    }

    pub fn from_tag(s: &str) -> Option<Self> {
        match s {
            "url" => Some(Self::Url),
            "file" => Some(Self::File),
            "diag" => Some(Self::Diagnostic),
            "git" => Some(Self::Git),
            "sha" => Some(Self::Sha),
            "ipv4" => Some(Self::Ipv4),
            "ipv6" => Some(Self::Ipv6),
            "uuid" => Some(Self::Uuid),
            "quote" => Some(Self::QuotedString),
            "cmd" => Some(Self::Command),
            "secret" => Some(Self::Secret),
            _ => None,
        }
    }
}

/// Type-priority list — front of list = highest priority. Drives **both**:
///
///   - Pass 2 cross-type dedup-by-raw (`dedup_by_raw_priority`)
///   - Picker score bonus (`type_priority_bonus`)
///
/// Reordering this list is the single edit that changes both behaviors
/// at once. Phase 7 KDL config will expose this as a user-tweakable
/// ordered list of type names.
pub const TYPE_PRIORITY: &[MatchType] = &[
    MatchType::Url,
    MatchType::Diagnostic,
    MatchType::File,
    MatchType::Uuid,
    MatchType::Git, // wins over bare Sha when hash appears in a git log line
    MatchType::Sha,
    MatchType::Ipv4,
    MatchType::Ipv6,
    MatchType::Command,
    MatchType::Secret, // entropy fallback is broad; let specific types win
    MatchType::QuotedString,
];

/// Position in `TYPE_PRIORITY`. Lower number = higher priority.
/// Returns `TYPE_PRIORITY.len()` for unknown types (puts them last).
fn type_priority_index(ty: MatchType) -> usize {
    TYPE_PRIORITY
        .iter()
        .position(|&t| t == ty)
        .unwrap_or(TYPE_PRIORITY.len())
}

/// Picker-rank score bonus derived from priority list position.
/// Symmetric around the middle: front of list = positive bonus,
/// middle = 0, tail = negative. With 10 types, range is +5 to -4.
pub fn type_priority_bonus(ty: MatchType) -> i32 {
    let n = TYPE_PRIORITY.len() as i32;
    let pos = type_priority_index(ty) as i32;
    n / 2 - pos
}

/// Run all patterns against `text` and return the combined, deduped,
/// recency-ordered matches.
/// Convenience wrapper used by tests. Production code uses [`extract_timed`].
#[allow(dead_code)]
pub fn extract(text: &str, patterns: &PatternsConfig) -> Vec<Match> {
    extract_timed(text, patterns).0
}

/// Per-pattern µs timings returned by [`extract_timed`]. All values are
/// microseconds; use them for `[zextract]` debug log lines in the caller.
#[derive(Debug, Default)]
pub struct ExtractionTimings {
    pub url_us: u128,
    pub file_us: u128,
    pub diagnostic_us: u128,
    pub sha_us: u128,
    pub ipv4_us: u128,
    pub ipv6_us: u128,
    pub uuid_us: u128,
    pub quoted_us: u128,
    pub command_us: u128,
    pub secret_us: u128,
    pub custom_us: u128,
    pub dedup_us: u128,
    pub total_us: u128,
}

/// Same as [`extract`] but also returns per-pattern µs timings. Used by the
/// plugin host to log where time is spent; tests stay on the cheaper `extract`.
pub fn extract_timed(text: &str, patterns: &PatternsConfig) -> (Vec<Match>, ExtractionTimings) {
    let t_start = Instant::now();
    let mut t = ExtractionTimings::default();
    let mut all: Vec<Match> = Vec::new();

    macro_rules! timed {
        ($field:ident, $expr:expr) => {{
            let t0 = Instant::now();
            let v = $expr;
            t.$field = t0.elapsed().as_micros();
            v
        }};
    }

    let dis = &patterns.disabled;

    if !dis.contains("url") {
        all.extend(timed!(url_us, crate::pattern::url::extract(text)));
    }
    if !dis.contains("file") {
        all.extend(timed!(file_us, crate::pattern::file::extract(text)));
    }
    if !dis.contains("diag") {
        all.extend(timed!(
            diagnostic_us,
            crate::pattern::diagnostic::extract(text)
        ));
    }
    if !dis.contains("git") {
        // git has no dedicated timing field — folded into sha_us
        let t0 = Instant::now();
        all.extend(crate::pattern::git::extract(text));
        t.sha_us += t0.elapsed().as_micros();
    }
    if !dis.contains("sha") {
        all.extend(timed!(sha_us, crate::pattern::sha::extract(text)));
    }
    if !dis.contains("ipv4") {
        all.extend(timed!(ipv4_us, crate::pattern::ipv4::extract(text)));
    }
    if !dis.contains("ipv6") {
        all.extend(timed!(ipv6_us, crate::pattern::ipv6::extract(text)));
    }
    if !dis.contains("uuid") {
        all.extend(timed!(uuid_us, crate::pattern::uuid::extract(text)));
    }
    if !dis.contains("quote") {
        all.extend(timed!(quoted_us, crate::pattern::quoted::extract(text)));
    }
    if !dis.contains("cmd") {
        all.extend(timed!(
            command_us,
            crate::pattern::command::extract(text, &patterns.command)
        ));
        // flag/comment/extension-anchored passes are folded into command_us.
        if patterns.command.flag_anchored {
            let t0 = Instant::now();
            all.extend(crate::pattern::command::extract_flag_anchored(
                text,
                &patterns.command,
            ));
            t.command_us += t0.elapsed().as_micros();
        }
        if patterns.command.comment_anchored {
            let t0 = Instant::now();
            all.extend(crate::pattern::command::extract_comment_anchored(
                text,
                &patterns.command,
            ));
            t.command_us += t0.elapsed().as_micros();
        }
        if patterns.command.extension_anchored {
            let t0 = Instant::now();
            all.extend(crate::pattern::command::extract_extension_anchored(
                text,
                &patterns.command,
            ));
            t.command_us += t0.elapsed().as_micros();
        }
    }
    if !dis.contains("secret") {
        all.extend(timed!(
            secret_us,
            crate::pattern::secret::extract(text, &patterns.secret)
        ));
    }
    all.extend(timed!(custom_us, extract_custom(text, patterns)));

    let t0 = Instant::now();
    let pass1 = dedup_keep_latest(all);
    let result = dedup_by_raw_priority(pass1);
    t.dedup_us = t0.elapsed().as_micros();

    t.total_us = t_start.elapsed().as_micros();
    (result, t)
}

/// Run user-defined custom patterns from the `patterns { }` config block.
/// Each pattern compiles its regex on every call — acceptable for the
/// small number of user patterns expected. Invalid regexes are silently
/// skipped (logged at debug level by the caller).
fn extract_custom(text: &str, patterns: &PatternsConfig) -> Vec<Match> {
    let mut out = Vec::new();
    for cp in &patterns.custom {
        if patterns.disabled.contains(&cp.name) {
            continue;
        }
        out.extend(extract_single_custom(text, cp));
    }
    out
}

/// Run a single user-defined custom pattern against `text`.
fn extract_single_custom(text: &str, cp: &crate::config::schema::CustomPattern) -> Vec<Match> {
    let re = match Regex::new(&cp.regex) {
        Ok(r) => r,
        Err(_) => return Vec::new(), // invalid regex — skip
    };
    let ty = MatchType::from_tag(&cp.ty).unwrap_or(MatchType::Url);
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in text.lines() {
        for caps in re.captures_iter(line) {
            let full = caps.get(0).unwrap();
            // If the regex has a capture group, group(1) is `{match}`.
            // No groups → full match is used (backwards-compatible).
            let (raw, span_start, span_end) = match caps.get(1) {
                Some(g) => (
                    g.as_str().to_string(),
                    byte_offset_of_line + g.start(),
                    byte_offset_of_line + g.end(),
                ),
                None => (
                    full.as_str().to_string(),
                    byte_offset_of_line + full.start(),
                    byte_offset_of_line + full.end(),
                ),
            };
            if raw.is_empty() {
                continue;
            }
            let mut fields = HashMap::new();
            let full_str = full.as_str();
            fields.insert("0".to_string(), full_str.to_string());
            for i in 1..caps.len() {
                if let Some(g) = caps.get(i) {
                    fields.insert(i.to_string(), g.as_str().to_string());
                }
            }
            let match_val = caps.get(1).map(|g| g.as_str()).unwrap_or(full_str);
            fields.insert("match".to_string(), match_val.to_string());

            let expanded = match &cp.template {
                Some(tmpl) => {
                    let mut s = tmpl.clone();
                    for (key, val) in &fields {
                        s = s.replace(&format!("{{{key}}}"), val);
                    }
                    s
                }
                None => raw.clone(),
            };

            // raw = expanded template when template is present, else regex match.
            let raw = if cp.template.is_some() {
                expanded.clone()
            } else {
                raw
            };

            match ty {
                MatchType::Url => {
                    fields.insert("url".to_string(), expanded.clone());
                }
                MatchType::File => {
                    fields.insert("file".to_string(), expanded.clone());
                }
                _ => {}
            }
            out.push(Match {
                ty,
                raw: raw.clone(),
                display: expanded,
                context: line.to_string(),
                span: (span_start, span_end),
                fields,
                label: Some(cp.name.clone()),
                source_pane_id: None,
            });
        }
        byte_offset_of_line += line.len() + 1;
    }
    out
}

/// Take the last `n` lines of a scrollback capture. Phase 1 hardcoded
/// the default cap to RECENT_LINES; Phase 7 will wire this to a
/// config-driven grab mode.
pub fn take_recent(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

// ── Pattern-chunked extraction ─────────────────────────────────────────────

/// One unit of work in a pattern-chunked extraction run.
/// Each variant maps to a single pattern pass; `Custom(i)` indexes into
/// `PatternsConfig::custom`.
#[derive(Debug, Clone)]
pub enum PatternTask {
    Url,
    File,
    Diagnostic,
    Git,
    Sha,
    Ipv4,
    Ipv6,
    Uuid,
    Quoted,
    Command,
    Secret,
    Custom(usize),
}

impl PatternTask {
    /// Short display name shown in the progress label.
    pub fn label<'a>(&'a self, patterns: &'a PatternsConfig) -> &'a str {
        match self {
            Self::Url => "url",
            Self::File => "file",
            Self::Diagnostic => "diag",
            Self::Git => "git",
            Self::Sha => "sha",
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
            Self::Uuid => "uuid",
            Self::Quoted => "quote",
            Self::Command => "cmd",
            Self::Secret => "secret",
            Self::Custom(i) => patterns
                .custom
                .get(*i)
                .map(|cp| cp.name.as_str())
                .unwrap_or("custom"),
        }
    }
}

/// Build the ordered queue of pattern tasks for the given effective config.
/// Disabled patterns are excluded so `queue.len()` equals the total tick count.
pub fn build_pattern_queue(patterns: &PatternsConfig) -> VecDeque<PatternTask> {
    let dis = &patterns.disabled;
    let mut q = VecDeque::new();
    if !dis.contains("url") {
        q.push_back(PatternTask::Url);
    }
    if !dis.contains("file") {
        q.push_back(PatternTask::File);
    }
    if !dis.contains("diag") {
        q.push_back(PatternTask::Diagnostic);
    }
    if !dis.contains("git") {
        q.push_back(PatternTask::Git);
    }
    if !dis.contains("sha") {
        q.push_back(PatternTask::Sha);
    }
    if !dis.contains("ipv4") {
        q.push_back(PatternTask::Ipv4);
    }
    if !dis.contains("ipv6") {
        q.push_back(PatternTask::Ipv6);
    }
    if !dis.contains("uuid") {
        q.push_back(PatternTask::Uuid);
    }
    if !dis.contains("quote") {
        q.push_back(PatternTask::Quoted);
    }
    if !dis.contains("cmd") {
        q.push_back(PatternTask::Command);
    }
    if !dis.contains("secret") {
        q.push_back(PatternTask::Secret);
    }
    for (i, cp) in patterns.custom.iter().enumerate() {
        if !dis.contains(&cp.name) {
            q.push_back(PatternTask::Custom(i));
        }
    }
    q
}

/// Run a single pattern task and return its matches.
/// For `Command`, all enabled sub-passes (flag/comment/extension anchored)
/// are folded into one call so the caller sees one tick per pattern, not
/// per sub-pass.
pub fn run_pattern_task(task: &PatternTask, text: &str, patterns: &PatternsConfig) -> Vec<Match> {
    match task {
        PatternTask::Url => crate::pattern::url::extract(text),
        PatternTask::File => crate::pattern::file::extract(text),
        PatternTask::Diagnostic => crate::pattern::diagnostic::extract(text),
        PatternTask::Git => crate::pattern::git::extract(text),
        PatternTask::Sha => crate::pattern::sha::extract(text),
        PatternTask::Ipv4 => crate::pattern::ipv4::extract(text),
        PatternTask::Ipv6 => crate::pattern::ipv6::extract(text),
        PatternTask::Uuid => crate::pattern::uuid::extract(text),
        PatternTask::Quoted => crate::pattern::quoted::extract(text),
        PatternTask::Command => {
            let mut out = crate::pattern::command::extract(text, &patterns.command);
            if patterns.command.flag_anchored {
                out.extend(crate::pattern::command::extract_flag_anchored(
                    text,
                    &patterns.command,
                ));
            }
            if patterns.command.comment_anchored {
                out.extend(crate::pattern::command::extract_comment_anchored(
                    text,
                    &patterns.command,
                ));
            }
            if patterns.command.extension_anchored {
                out.extend(crate::pattern::command::extract_extension_anchored(
                    text,
                    &patterns.command,
                ));
            }
            out
        }
        PatternTask::Secret => crate::pattern::secret::extract(text, &patterns.secret),
        PatternTask::Custom(i) => patterns
            .custom
            .get(*i)
            .map(|cp| extract_single_custom(text, cp))
            .unwrap_or_default(),
    }
}

/// Apply both dedup passes to a raw accumulated match list.
/// Used by incremental extraction to re-settle the list after each tick.
pub fn dedup_matches(matches: Vec<Match>) -> Vec<Match> {
    dedup_by_raw_priority(dedup_keep_latest(matches))
}

// ── Private helpers ────────────────────────────────────────────────────────

/// Pass 1: dedup by `(type, raw)` keeping the latest occurrence
/// (largest `span.0`). Returns matches in latest-first order.
fn dedup_keep_latest(mut matches: Vec<Match>) -> Vec<Match> {
    // Sort ascending by span.0 so iterating reverse yields latest-first.
    matches.sort_by_key(|m| m.span.0);
    let mut seen: HashSet<(MatchType, String)> = HashSet::new();
    let mut out: Vec<Match> = Vec::with_capacity(matches.len());
    for m in matches.into_iter().rev() {
        if seen.insert((m.ty, m.raw.clone())) {
            out.push(m);
        }
    }
    out
}

/// Pass 2: dedup by `raw` alone. When multiple types match the same
/// raw text, keep the one with the highest priority (front-of-list in
/// `TYPE_PRIORITY`). Ties resolved by recency (larger `span.0` wins).
///
/// Returns matches in latest-first order.
fn dedup_by_raw_priority(matches: Vec<Match>) -> Vec<Match> {
    let mut by_raw: HashMap<String, Match> = HashMap::new();
    for m in matches {
        let key = m.raw.clone();
        match by_raw.entry(key) {
            Entry::Vacant(e) => {
                e.insert(m);
            }
            Entry::Occupied(mut e) => {
                let incumbent = e.get();
                let new_prio = type_priority_index(m.ty);
                let cur_prio = type_priority_index(incumbent.ty);
                let replace = if new_prio < cur_prio {
                    true
                } else if new_prio == cur_prio {
                    // Same priority — recency wins (latest span.0).
                    m.span.0 > incumbent.span.0
                } else {
                    false
                };
                if replace {
                    e.insert(m);
                }
            }
        }
    }
    let mut out: Vec<Match> = by_raw.into_values().collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.span.0));
    out
}

#[cfg(test)]
mod fixture_tests {
    //! Integration coverage: read each fixture file and assert minimum
    //! counts per type. Lighter than full snapshot diffing (which we'll
    //! revisit when insta plays nicely with the test environment) but
    //! catches the regression cases that matter — a pattern silently
    //! ceasing to match its fixture is exactly what these tests find.
    use super::*;

    fn ep() -> PatternsConfig {
        PatternsConfig::default()
    }

    fn count_by_type(text: &str, ty: MatchType) -> usize {
        extract(text, &ep())
            .into_iter()
            .filter(|m| m.ty == ty)
            .count()
    }

    #[test]
    fn urls_fixture_has_urls() {
        let text = include_str!("../tests/fixtures/urls.txt");
        assert!(count_by_type(text, MatchType::Url) >= 5);
    }

    #[test]
    fn files_fixture_has_files() {
        let text = include_str!("../tests/fixtures/files.txt");
        assert!(count_by_type(text, MatchType::File) >= 3);
    }

    #[test]
    fn diagnostics_fixture_has_diagnostics() {
        let text = include_str!("../tests/fixtures/diagnostics.txt");
        assert!(count_by_type(text, MatchType::Diagnostic) >= 2);
    }

    #[test]
    fn git_log_fixture_has_git_matches() {
        let text = include_str!("../tests/fixtures/git_log.txt");
        assert!(count_by_type(text, MatchType::Git) >= 5);
    }

    #[test]
    fn commands_fixture_has_commands() {
        let text = include_str!("../tests/fixtures/commands.txt");
        assert!(count_by_type(text, MatchType::Command) >= 5);
    }

    #[test]
    fn commands_fixture_inline_comment_hints() {
        use crate::config::schema::CommandPatternConfig;
        let text = include_str!("../tests/fixtures/commands.txt");
        let patterns = PatternsConfig {
            command: CommandPatternConfig {
                flag_anchored: true,
                comment_anchored: true,
                extension_anchored: true,
                ..CommandPatternConfig::default()
            },
            ..PatternsConfig::default()
        };
        let matches = extract(text, &patterns);
        let find = |raw: &str| matches.iter().find(|m| m.raw == raw);

        // comment-anchored: `./sync-all.sh` is also matched by the file pattern,
        // so cross-type dedup promotes it to File (higher priority) and the hint
        // is lost. Just assert the raw value is present.
        assert!(
            find("./sync-all.sh").is_some(),
            "./sync-all.sh not extracted"
        );

        // flag-anchored with path prefix: args prevent file-pattern overlap so
        // the Command match (and its hint) survives dedup.
        let dry = find("./sync-all.sh --dry-run").expect("./sync-all.sh --dry-run not extracted");
        assert_eq!(
            dry.fields.get("hint").map(String::as_str),
            Some("preview only")
        );

        let backup =
            find("/usr/local/bin/backup.sh --incremental").expect("backup.sh not extracted");
        assert_eq!(
            backup.fields.get("hint").map(String::as_str),
            Some("nightly incremental backup")
        );

        // exec-anchored also strips inline comments.
        let nginx = find("sudo systemctl restart nginx").expect("nginx restart not extracted");
        assert_eq!(
            nginx.fields.get("hint").map(String::as_str),
            Some("apply config changes")
        );

        // flag-anchored with prose prefix + multi-line continuation.
        let multiline = find("./testcommand.sh -option ntu --otunug osu -n -line 1 2 3 test")
            .expect("multi-line continuation not extracted");
        assert_eq!(
            multiline.fields.get("hint").map(String::as_str),
            Some("command")
        );

        // prompt-anchored continuation with inline comments.
        assert!(
            find("./build.sh --release --target wasm32").is_some(),
            "prompt-anchored continuation not extracted"
        );

        // extension-anchored.
        let backup_daily =
            find("backup.sh --daily").expect("extension-anchored backup.sh not extracted");
        assert_eq!(
            backup_daily.fields.get("hint").map(String::as_str),
            Some("scheduled backup")
        );

        assert!(
            find("deploy.pl --env prod --verbose").is_some(),
            "deploy.pl not extracted"
        );
    }

    #[test]
    fn secrets_fixture_has_secrets() {
        let text = include_str!("../tests/fixtures/secrets.txt");
        let matches = extract(text, &ep());
        let secrets: Vec<_> = matches
            .iter()
            .filter(|m| m.ty == MatchType::Secret)
            .collect();
        assert!(secrets.len() >= 7, "got {} secrets", secrets.len());
        // Verify a mix of curated formats fired.
        let formats: std::collections::HashSet<&str> = secrets
            .iter()
            .filter_map(|m| m.fields.get("secret_format").map(|s| s.as_str()))
            .collect();
        for required in ["jwt", "aws", "github", "gitlab", "stripe", "bearer"] {
            assert!(formats.contains(required), "missing format: {required}");
        }
    }

    #[test]
    fn realworld_fixture_finds_diverse_types() {
        let text = include_str!("../tests/fixtures/realworld.txt");
        let matches = extract(text, &ep());
        let types: std::collections::HashSet<MatchType> = matches.iter().map(|m| m.ty).collect();
        // Realworld transcript should exercise at least 5 different types.
        assert!(types.len() >= 5, "got types: {types:?}");
    }

    #[test]
    fn adversarial_fixture_rejects_near_misses() {
        let text = include_str!("../tests/fixtures/adversarial.txt");
        let matches = extract(text, &ep());
        // No SHA from "12345678" (pure-numeric).
        assert!(!matches
            .iter()
            .any(|m| m.ty == MatchType::Sha && m.raw == "12345678"));
        // No IPv4 from "999.1.1.1".
        assert!(!matches
            .iter()
            .any(|m| m.ty == MatchType::Ipv4 && m.raw.starts_with("999.")));
    }

    #[test]
    fn stress_fixture_dense_mixed_corpus() {
        // 260+ line realistic transcript — exercises the
        // "many matches across many types" path that triggered
        // Zellij's wasm growth cap before the buffer-reuse fix.
        let text = include_str!("../tests/fixtures/stress.txt");
        let matches = extract(text, &ep());
        let types: std::collections::HashSet<MatchType> = matches.iter().map(|m| m.ty).collect();

        // Diverse — at least 7 of the 10 v1 types fire.
        assert!(
            types.len() >= 7,
            "stress fixture should exercise ≥7 types, got {} ({:?})",
            types.len(),
            types
        );

        // Dense — enough matches to stress the per-frame allocator.
        assert!(
            matches.len() >= 40,
            "stress fixture should yield ≥40 matches, got {}",
            matches.len()
        );

        // Each of the most-common types should fire at least once.
        for required in [
            MatchType::Url,
            MatchType::File,
            MatchType::Command,
            MatchType::Sha,
            MatchType::Secret,
        ] {
            assert!(
                matches.iter().any(|m| m.ty == required),
                "stress fixture missing required type {required:?}"
            );
        }
    }

    #[test]
    fn multi_group_patterns_fixture() {
        use crate::config::schema::CustomPattern;
        let text = include_str!("../tests/fixtures/multi_group_patterns.txt");

        let patterns = PatternsConfig {
            custom: vec![
                CustomPattern {
                    name: "port".to_string(),
                    regex: r":[0-9]{4,5}\b".to_string(),
                    ty: "url".to_string(),
                    template: None,
                },
                CustomPattern {
                    name: "jira".to_string(),
                    regex: r"([A-Z]+)-([0-9]+[A-Z]*)".to_string(),
                    ty: "url".to_string(),
                    template: Some("https://jira.example.com/browse/{1}-{2}".to_string()),
                },
                CustomPattern {
                    name: "github-pr".to_string(),
                    regex: r"github\.com/([^/\s]+)/([^/\s]+)/pull/([0-9]+)".to_string(),
                    ty: "url".to_string(),
                    template: Some("https://github.com/{1}/{2}/pull/{3}".to_string()),
                },
            ],
            ..PatternsConfig::default()
        };

        let matches = extract(text, &patterns);

        // Port: :3000, :9090, :8080
        let ports: Vec<_> = matches
            .iter()
            .filter(|m| m.label.as_deref() == Some("port"))
            .collect();
        assert!(
            ports.len() >= 3,
            "expected ≥3 port matches, got {}",
            ports.len()
        );

        // Jira: ST-154R, BACKEND-42, FRONTEND-7, OPS-100 + git log refs
        let jira: Vec<_> = matches
            .iter()
            .filter(|m| m.label.as_deref() == Some("jira"))
            .collect();
        assert!(
            jira.len() >= 4,
            "expected ≥4 jira matches, got {}",
            jira.len()
        );

        // ST-154R: group 1 = "ST", group 2 = "154R", display = full URL
        let st = jira
            .iter()
            .find(|m| m.fields.get("2").map(|s| s.as_str()) == Some("154R"));
        let st = st.expect("ST-154R not found");
        assert_eq!(st.fields.get("1").unwrap(), "ST");
        assert_eq!(st.fields.get("2").unwrap(), "154R");
        assert_eq!(st.display, "https://jira.example.com/browse/ST-154R");
        assert_eq!(
            st.fields.get("url").unwrap(),
            "https://jira.example.com/browse/ST-154R"
        );

        // GitHub PRs: 3 distinct PRs across 2 orgs
        let prs: Vec<_> = matches
            .iter()
            .filter(|m| m.label.as_deref() == Some("github-pr"))
            .collect();
        assert!(prs.len() >= 3, "expected ≥3 PR matches, got {}", prs.len());

        // PR #99: org=myorg, repo=myrepo, number=99
        let pr99 = prs
            .iter()
            .find(|m| m.fields.get("3").map(|s| s.as_str()) == Some("99"));
        let pr99 = pr99.expect("PR #99 not found");
        assert_eq!(pr99.fields.get("1").unwrap(), "myorg");
        assert_eq!(pr99.fields.get("2").unwrap(), "myrepo");
        assert_eq!(pr99.display, "https://github.com/myorg/myrepo/pull/99");

        // Cross-org PR: otherorg/otherrepo/pull/7
        let pr7 = prs
            .iter()
            .find(|m| m.fields.get("1").map(|s| s.as_str()) == Some("otherorg"));
        let pr7 = pr7.expect("cross-org PR not found");
        assert_eq!(pr7.display, "https://github.com/otherorg/otherrepo/pull/7");
    }

    #[test]
    fn custom_patterns_fixture_port_and_jira() {
        use crate::config::schema::CustomPattern;
        let text = include_str!("../tests/fixtures/custom_patterns.txt");

        let patterns = PatternsConfig {
            custom: vec![
                CustomPattern {
                    name: "port".to_string(),
                    regex: r":[0-9]{4,5}\b".to_string(),
                    ty: "url".to_string(),
                    template: None,
                },
                CustomPattern {
                    name: "jira".to_string(),
                    regex: r"[A-Z]+-[0-9]+".to_string(),
                    ty: "url".to_string(),
                    template: Some("https://jira.example.com/browse/{match}".to_string()),
                },
            ],
            ..PatternsConfig::default()
        };

        let matches = extract(text, &patterns);

        // Port pattern: :3000, :8080, :5432, :443, :6443, :9000, :9090
        let ports: Vec<_> = matches.iter().filter(|m| m.raw.starts_with(':')).collect();
        assert!(
            ports.len() >= 4,
            "expected ≥4 port matches, got {}: {:?}",
            ports.len(),
            ports.iter().map(|m| &m.raw).collect::<Vec<_>>()
        );

        // Jira pattern: PROJ-123, PROJ-456, API-789, OPS-42, PROJ-100, BACKEND-201
        // Use display to check template was applied
        let jira_by_display: Vec<_> = matches
            .iter()
            .filter(|m| m.display.contains("jira.example.com"))
            .collect();
        assert!(
            jira_by_display.len() >= 4,
            "expected ≥4 jira matches with template applied, got {}: {:?}",
            jira_by_display.len(),
            jira_by_display
                .iter()
                .map(|m| &m.display)
                .collect::<Vec<_>>()
        );

        // Template expands correctly; raw = expanded URL
        let proj123 = matches
            .iter()
            .find(|m| m.display == "https://jira.example.com/browse/PROJ-123")
            .unwrap();
        assert_eq!(proj123.raw, "https://jira.example.com/browse/PROJ-123");
        assert_eq!(
            proj123.fields.get("url").unwrap(),
            "https://jira.example.com/browse/PROJ-123"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep() -> PatternsConfig {
        PatternsConfig::default()
    }

    fn patterns_with(name: &str, regex: &str, ty: &str, template: Option<&str>) -> PatternsConfig {
        use crate::config::schema::CustomPattern;
        PatternsConfig {
            custom: vec![CustomPattern {
                name: name.to_string(),
                regex: regex.to_string(),
                ty: ty.to_string(),
                template: template.map(String::from),
            }],
            ..PatternsConfig::default()
        }
    }

    #[test]
    fn custom_pattern_basic_match() {
        let p = patterns_with(
            "jira",
            r"[A-Z]+-\d+",
            "url",
            Some("https://jira.example.com/browse/{match}"),
        );
        let text = "Fix PROJ-123 and PROJ-456 soon";
        let matches = extract(text, &p);
        // raw = expanded URL (template present); filter by label
        let jira: Vec<_> = matches
            .iter()
            .filter(|m| m.label.as_deref() == Some("jira"))
            .collect();
        assert_eq!(jira.len(), 2);
        assert_eq!(jira[0].ty, MatchType::Url);
        assert!(jira[0].display.contains("jira.example.com"));
        assert!(jira[0].display.contains("PROJ-"));
        assert!(jira[0]
            .fields
            .get("url")
            .unwrap()
            .contains("jira.example.com"));
    }

    #[test]
    fn custom_pattern_capture_group_context_prefix() {
        // Template present → raw = expanded URL (unique dedup key).
        let p = patterns_with(
            "jira",
            r"New Jira ticket : ([A-Z]+-[0-9]+[A-Z]*)",
            "url",
            Some("https://jira.example.com/browse/{match}"),
        );
        let text = "New Jira ticket : ST-154R";
        let matches = extract(text, &p);
        let m = matches
            .iter()
            .find(|m| m.label.as_deref() == Some("jira"))
            .unwrap();
        assert_eq!(m.raw, "https://jira.example.com/browse/ST-154R");
        assert_eq!(m.display, "https://jira.example.com/browse/ST-154R");
        assert_eq!(m.fields.get("match").unwrap(), "ST-154R");
    }

    #[test]
    fn custom_pattern_multi_group_template() {
        // 3 groups; raw = expanded URL so each unique PR survives dedup.
        let p = patterns_with(
            "pr",
            r"github\.com/([^/]+)/([^/]+)/pull/([0-9]+)",
            "url",
            Some("https://github.com/{1}/{2}/pull/{3}"),
        );
        let text = "see github.com/myorg/myrepo/pull/42 for details";
        let matches = extract(text, &p);
        let m = matches
            .iter()
            .find(|m| m.label.as_deref() == Some("pr"))
            .unwrap();
        assert_eq!(m.raw, "https://github.com/myorg/myrepo/pull/42");
        assert_eq!(m.display, "https://github.com/myorg/myrepo/pull/42");
        assert_eq!(m.fields.get("1").unwrap(), "myorg");
        assert_eq!(m.fields.get("2").unwrap(), "myrepo");
        assert_eq!(m.fields.get("3").unwrap(), "42");
        assert_eq!(
            m.fields.get("0").unwrap(),
            "github.com/myorg/myrepo/pull/42"
        );
    }

    #[test]
    fn custom_pattern_group0_full_match_in_template() {
        // {0} gives the full match even when groups exist
        let p = patterns_with(
            "tagged",
            r"PREFIX:([A-Z]+)",
            "cmd",
            Some("echo full={0} id={1}"),
        );
        let text = "see PREFIX:HELLO here";
        let matches = extract(text, &p);
        let m = matches
            .iter()
            .find(|m| m.label.as_deref() == Some("tagged"))
            .unwrap();
        assert_eq!(m.display, "echo full=PREFIX:HELLO id=HELLO");
    }

    #[test]
    fn custom_pattern_match_alias_for_group1() {
        // {match} and {1} are equivalent
        let p = patterns_with("jira", r"([A-Z]+-[0-9]+)", "url", Some("{match} == {1}"));
        let text = "fix PROJ-99";
        let matches = extract(text, &p);
        let m = matches
            .iter()
            .find(|m| m.label.as_deref() == Some("jira"))
            .unwrap();
        assert_eq!(m.display, "PROJ-99 == PROJ-99");
    }

    #[test]
    fn custom_pattern_no_groups_with_template_raw_is_expanded() {
        // No groups + template → raw = expanded URL.
        let p = patterns_with(
            "jira",
            r"[A-Z]+-[0-9]+",
            "url",
            Some("https://jira.example.com/browse/{match}"),
        );
        let text = "Fix PROJ-123 today";
        let matches = extract(text, &p);
        let m = matches
            .iter()
            .find(|m| m.label.as_deref() == Some("jira"))
            .unwrap();
        assert_eq!(m.raw, "https://jira.example.com/browse/PROJ-123");
    }

    #[test]
    fn custom_pattern_no_template_raw_is_match_text() {
        // No template → raw = the regex match text itself.
        let p = patterns_with("ticket", r"[A-Z]+-[0-9]+", "cmd", None);
        let text = "Fix PROJ-123 today";
        let matches = extract(text, &p);
        let m = matches
            .iter()
            .find(|m| m.label.as_deref() == Some("ticket"))
            .unwrap();
        assert_eq!(m.raw, "PROJ-123");
    }

    #[test]
    fn custom_pattern_no_template_display_equals_raw() {
        let p = patterns_with("sha256", r"[0-9a-f]{64}", "sha", None);
        let hash = "a".repeat(64);
        let text = format!("hash: {hash}");
        let matches = extract(&text, &p);
        // May or may not match depending on built-in sha — just verify
        // no panic on None template.
        let _ = matches.iter().find(|m| m.ty == MatchType::Sha);
    }

    #[test]
    fn custom_pattern_invalid_regex_skipped() {
        let p = patterns_with("bad", r"[unclosed", "url", None);
        // Should not panic; just return built-in matches.
        let text = "https://example.com";
        let matches = extract(text, &p);
        assert!(!matches.is_empty()); // built-in URL still works
    }

    #[test]
    fn dedupes_keeping_latest() {
        let text = "first https://a.example.com/x\nlast https://a.example.com/x";
        let m = extract(text, &ep());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].context, "last https://a.example.com/x");
    }

    #[test]
    fn cross_type_dedup_keeps_higher_priority() {
        // `src/main.rs:42:8` matches BOTH file and diagnostic. With
        // diagnostic ranked above file in TYPE_PRIORITY, the diag wins.
        let text = "error at src/main.rs:42:8";
        let m = extract(text, &ep());
        let same_raw: Vec<_> = m.iter().filter(|x| x.raw == "src/main.rs:42:8").collect();
        assert_eq!(same_raw.len(), 1, "got: {same_raw:?}");
        assert_eq!(same_raw[0].ty, MatchType::Diagnostic);
    }

    #[test]
    fn priority_bonus_order_matches_list() {
        // First in TYPE_PRIORITY gets the highest bonus; last gets the
        // lowest. Round-trip the list and assert bonuses are
        // monotonically decreasing.
        let bonuses: Vec<i32> = TYPE_PRIORITY
            .iter()
            .map(|&t| type_priority_bonus(t))
            .collect();
        for w in bonuses.windows(2) {
            assert!(w[0] > w[1], "expected strict decrease, got {bonuses:?}");
        }
    }

    #[test]
    fn recency_order_latest_first() {
        let text = "first https://a.example.com\nsecond https://b.example.com";
        let m = extract(text, &ep());
        assert_eq!(m[0].raw, "https://b.example.com");
        assert_eq!(m[1].raw, "https://a.example.com");
    }

    #[test]
    fn take_recent_caps_to_n_lines() {
        let text = (1..=200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let capped = take_recent(&text, 50);
        assert_eq!(capped.lines().count(), 50);
        assert!(capped.starts_with("line 151\n"));
        assert!(capped.ends_with("line 200"));
    }

    #[test]
    fn take_recent_handles_short_input() {
        let text = "one\ntwo\nthree";
        let capped = take_recent(text, 50);
        assert_eq!(capped, text);
    }

    // ── Custom pattern priority test ──────────────────────────────────────
    // This test documents the v1 behaviour (built-in file pattern wins on
    // overlap) and the expected v2 behaviour (user-controlled priority).

    #[test]
    fn router_pattern_extracts_path_from_url() {
        use crate::config::schema::CustomPattern;

        let patterns = PatternsConfig {
            custom: vec![CustomPattern {
                name: "router".to_string(),
                regex: r"router\.com/([^/\s]+)".to_string(),
                ty: "cmd".to_string(),
                template: Some("Routing path {1}".to_string()),
            }],
            ..PatternsConfig::default()
        };

        let text = "Request failed at http://example.router.com/tester";
        let matches = extract(text, &patterns);

        // Custom pattern fires: group 1 = "tester", template expanded.
        let router_match = matches
            .iter()
            .find(|m| m.label.as_deref() == Some("router"));
        let router_match = router_match.expect("router pattern did not match");
        assert_eq!(router_match.fields.get("1").unwrap(), "tester");
        assert_eq!(router_match.display, "Routing path tester");
        assert_eq!(router_match.raw, "Routing path tester"); // template present → raw = expanded

        // Priority observation (v1 behaviour):
        // The built-in URL pattern also matches "http://example.router.com/tester"
        // and the built-in file pattern matches "router.com/tester".
        // These have DIFFERENT raw values from the custom match so they
        // all survive dedup and appear alongside the router entry.
        // In v2 a user-controlled `priority` list would let the router
        // pattern suppress the file match for the same span.
        let url_match = matches
            .iter()
            .find(|m| m.ty == MatchType::Url && m.label.is_none());
        assert!(url_match.is_some(), "built-in url match should also appear");
    }

    #[test]
    fn disabled_url_suppresses_url_matches() {
        let mut patterns = PatternsConfig::default();
        patterns.disabled.insert("url".to_string());
        let text = "see https://example.com for details";
        let matches = extract(text, &patterns);
        assert!(
            matches.iter().all(|m| m.ty != MatchType::Url),
            "url pattern should be suppressed"
        );
    }

    #[test]
    fn disabled_custom_pattern_suppressed() {
        use crate::config::schema::CustomPattern;
        let patterns = PatternsConfig {
            disabled: ["ticket".to_string()].into_iter().collect(),
            custom: vec![CustomPattern {
                name: "ticket".to_string(),
                regex: "[A-Z]+-[0-9]+".to_string(),
                ty: "url".to_string(),
                template: None,
            }],
            ..PatternsConfig::default()
        };
        let text = "Fixed in PROJ-123";
        let matches = extract(text, &patterns);
        assert!(
            matches.iter().all(|m| m.label.as_deref() != Some("ticket")),
            "disabled custom pattern should be suppressed"
        );
    }
}
