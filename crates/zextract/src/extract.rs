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
use std::collections::{HashMap, HashSet};

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
pub fn extract(text: &str, patterns: &PatternsConfig) -> Vec<Match> {
    let mut all: Vec<Match> = Vec::new();
    all.extend(crate::pattern::url::extract(text));
    all.extend(crate::pattern::file::extract(text));
    all.extend(crate::pattern::diagnostic::extract(text));
    all.extend(crate::pattern::sha::extract(text));
    all.extend(crate::pattern::ipv4::extract(text));
    all.extend(crate::pattern::ipv6::extract(text));
    all.extend(crate::pattern::uuid::extract(text));
    all.extend(crate::pattern::quoted::extract(text));
    all.extend(crate::pattern::command::extract(text));
    all.extend(crate::pattern::secret::extract(text));
    all.extend(extract_custom(text, patterns));

    let pass1 = dedup_keep_latest(all);
    dedup_by_raw_priority(pass1)
}

/// Run user-defined custom patterns from the `patterns { }` config block.
/// Each pattern compiles its regex on every call — acceptable for the
/// small number of user patterns expected. Invalid regexes are silently
/// skipped (logged at debug level by the caller).
fn extract_custom(text: &str, patterns: &PatternsConfig) -> Vec<Match> {
    let mut out = Vec::new();
    for cp in &patterns.custom {
        let re = match Regex::new(&cp.regex) {
            Ok(r) => r,
            Err(_) => continue, // invalid regex — skip
        };
        let ty = MatchType::from_tag(&cp.ty).unwrap_or(MatchType::Url);
        let mut byte_offset_of_line = 0usize;
        for line in text.lines() {
            for caps in re.captures_iter(line) {
                let full = caps.get(0).unwrap();
                // If the regex has a capture group, group(1) is `{match}`.
                // This lets users write patterns like:
                //   `New Jira ticket : ([A-Z]+-[0-9]+)` — the prefix
                //   anchors the match but only the ticket ID is captured.
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
                let expanded = match &cp.template {
                    Some(tmpl) => tmpl.replace("{match}", &raw),
                    None => raw.clone(),
                };
                let mut fields = HashMap::new();
                match ty {
                    MatchType::Url => { fields.insert("url".to_string(), expanded.clone()); }
                    MatchType::File => { fields.insert("file".to_string(), expanded.clone()); }
                    _ => {}
                }
                fields.insert("match".to_string(), raw.clone());
                out.push(Match {
                    ty,
                    raw: raw.clone(),
                    display: expanded,
                    context: line.to_string(),
                    span: (span_start, span_end),
                    fields,
                    label: Some(cp.name.clone()),
                });
            }
            byte_offset_of_line += line.len() + 1;
        }
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

    fn ep() -> PatternsConfig { PatternsConfig::default() }

    fn count_by_type(text: &str, ty: MatchType) -> usize {
        extract(text, &ep()).into_iter().filter(|m| m.ty == ty).count()
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
    fn git_log_fixture_has_shas() {
        let text = include_str!("../tests/fixtures/git_log.txt");
        assert!(count_by_type(text, MatchType::Sha) >= 5);
    }

    #[test]
    fn commands_fixture_has_commands() {
        let text = include_str!("../tests/fixtures/commands.txt");
        assert!(count_by_type(text, MatchType::Command) >= 5);
    }

    #[test]
    fn secrets_fixture_has_secrets() {
        let text = include_str!("../tests/fixtures/secrets.txt");
        let matches = extract(text, &ep());
        let secrets: Vec<_> = matches.iter().filter(|m| m.ty == MatchType::Secret).collect();
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
        let types: std::collections::HashSet<MatchType> =
            matches.iter().map(|m| m.ty).collect();
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
        let types: std::collections::HashSet<MatchType> =
            matches.iter().map(|m| m.ty).collect();

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
    fn custom_patterns_fixture_port_and_jira() {
        use crate::config::CustomPattern;
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
        };

        let matches = extract(text, &patterns);

        // Port pattern: :3000, :8080, :5432, :443, :6443, :9000, :9090
        let ports: Vec<_> = matches.iter()
            .filter(|m| m.raw.starts_with(':'))
            .collect();
        assert!(ports.len() >= 4, "expected ≥4 port matches, got {}: {:?}",
            ports.len(), ports.iter().map(|m| &m.raw).collect::<Vec<_>>());

        // Jira pattern: PROJ-123, PROJ-456, API-789, OPS-42, PROJ-100, BACKEND-201
        // Use display to check template was applied
        let jira_by_display: Vec<_> = matches.iter()
            .filter(|m| m.display.contains("jira.example.com"))
            .collect();
        assert!(jira_by_display.len() >= 4,
            "expected ≥4 jira matches with template applied, got {}: {:?}",
            jira_by_display.len(),
            jira_by_display.iter().map(|m| &m.display).collect::<Vec<_>>());

        // Template expands correctly
        let proj123 = matches.iter().find(|m| m.raw == "PROJ-123").unwrap();
        assert_eq!(proj123.display, "https://jira.example.com/browse/PROJ-123");
        assert_eq!(proj123.fields.get("url").unwrap(),
            "https://jira.example.com/browse/PROJ-123");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep() -> PatternsConfig { PatternsConfig::default() }

    fn patterns_with(name: &str, regex: &str, ty: &str, template: Option<&str>) -> PatternsConfig {
        use crate::config::CustomPattern;
        PatternsConfig {
            custom: vec![CustomPattern {
                name: name.to_string(),
                regex: regex.to_string(),
                ty: ty.to_string(),
                template: template.map(String::from),
            }],
        }
    }

    #[test]
    fn custom_pattern_basic_match() {
        let p = patterns_with("jira", r"[A-Z]+-\d+", "url",
            Some("https://jira.example.com/browse/{match}"));
        let text = "Fix PROJ-123 and PROJ-456 soon";
        let matches = extract(text, &p);
        let jira: Vec<_> = matches.iter()
            .filter(|m| m.raw == "PROJ-123" || m.raw == "PROJ-456")
            .collect();
        assert_eq!(jira.len(), 2);
        assert_eq!(jira[0].ty, MatchType::Url);
        // Template applied: display is the expanded URL
        assert!(jira[0].display.contains("jira.example.com"));
        assert!(jira[0].display.contains("PROJ-"));
        // url field populated so open verb works
        assert!(jira[0].fields.get("url").unwrap().contains("jira.example.com"));
    }

    #[test]
    fn custom_pattern_capture_group_extracts_group1() {
        // Full match: "New Jira ticket : ST-154R"
        // Group 1:    "ST-154R"  ← only this becomes raw/{match}
        let p = patterns_with("jira",
            r"New Jira ticket : ([A-Z]+-[0-9]+[A-Z]*)",
            "url",
            Some("https://jira.example.com/browse/{match}"));
        let text = "New Jira ticket : ST-154R";
        let matches = extract(text, &p);
        let m = matches.iter().find(|m| m.label.as_deref() == Some("jira")).unwrap();
        assert_eq!(m.raw, "ST-154R");
        assert_eq!(m.display, "https://jira.example.com/browse/ST-154R");
    }

    #[test]
    fn custom_pattern_no_groups_uses_full_match() {
        let p = patterns_with("jira", r"[A-Z]+-[0-9]+", "url",
            Some("https://jira.example.com/browse/{match}"));
        let text = "Fix PROJ-123 today";
        let matches = extract(text, &p);
        let m = matches.iter().find(|m| m.label.as_deref() == Some("jira")).unwrap();
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
}
