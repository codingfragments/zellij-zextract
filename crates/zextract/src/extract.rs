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
}

impl MatchType {
    pub fn tag(self) -> &'static str {
        match self {
            MatchType::Url => "url",
        }
    }
}

/// Run all patterns against `text` and return the combined, deduped,
/// recency-ordered matches.
pub fn extract(text: &str) -> Vec<Match> {
    let mut all: Vec<Match> = Vec::new();
    all.extend(crate::pattern::url::extract(text));
    // future patterns (file, sha, diagnostic, ...) appended here.

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
