//! Extraction coordinator. Dispatches the captured scrollback text to
//! each pattern module, combines results, and resolves overlap per Q25:
//!
//!   - Cross-type overlap: emit all matches.
//!   - Same-type dedup: keep only the latest occurrence per `(type, raw)`.
//!   - Within-pattern leftmost-longest: handled inside each pattern.
//!
//! Output order is **latest-first** (most recent occurrence in the
//! scrollback ranks ahead of older ones).

use std::collections::{HashMap, HashSet};

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
}

/// Run all patterns against `text` and return the combined, deduped,
/// recency-ordered matches.
pub fn extract(text: &str) -> Vec<Match> {
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

    dedup_keep_latest(all)
}

/// Take the last `n` lines of a scrollback capture. Phase 1 hardcoded
/// the default cap to RECENT_LINES; Phase 7 will wire this to a
/// config-driven grab mode.
pub fn take_recent(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Dedup by `(type, raw)` keeping the latest occurrence (largest
/// `span.0`). Returns matches in latest-first order.
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

#[cfg(test)]
mod fixture_tests {
    //! Integration coverage: read each fixture file and assert minimum
    //! counts per type. Lighter than full snapshot diffing (which we'll
    //! revisit when insta plays nicely with the test environment) but
    //! catches the regression cases that matter — a pattern silently
    //! ceasing to match its fixture is exactly what these tests find.
    use super::*;

    fn count_by_type(text: &str, ty: MatchType) -> usize {
        extract(text).into_iter().filter(|m| m.ty == ty).count()
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
        let matches = extract(text);
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
        let matches = extract(text);
        let types: std::collections::HashSet<MatchType> =
            matches.iter().map(|m| m.ty).collect();
        // Realworld transcript should exercise at least 5 different types.
        assert!(types.len() >= 5, "got types: {types:?}");
    }

    #[test]
    fn adversarial_fixture_rejects_near_misses() {
        let text = include_str!("../tests/fixtures/adversarial.txt");
        let matches = extract(text);
        // No SHA from "12345678" (pure-numeric).
        assert!(!matches
            .iter()
            .any(|m| m.ty == MatchType::Sha && m.raw == "12345678"));
        // No IPv4 from "999.1.1.1".
        assert!(!matches
            .iter()
            .any(|m| m.ty == MatchType::Ipv4 && m.raw.starts_with("999.")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_keeping_latest() {
        let text = "first https://a.example.com/x\nlast https://a.example.com/x";
        let m = extract(text);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].context, "last https://a.example.com/x");
    }

    #[test]
    fn recency_order_latest_first() {
        let text = "first https://a.example.com\nsecond https://b.example.com";
        let m = extract(text);
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
