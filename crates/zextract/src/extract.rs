//! Pattern-based extraction over captured scrollback text.
//!
//! Phase 1 implements URL extraction only. Phase 3 expands the pattern
//! set; the `Match` struct already has the per-type field map so future
//! patterns slot in without reshaping.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub ty: MatchType,
    pub raw: String,
    pub display: String,
    pub context: String,
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

fn url_regex() -> &'static Regex {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    URL_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:https?|ftp|file|git|ssh)://[^\s<>'`\[\](){}]+")
            .expect("url regex compiles")
    })
}

/// Trim trailing punctuation that's commonly adjacent to URLs in prose
/// but not part of them. Keep the original `raw` intact for downstream
/// consumers; this only affects what the picker shows / what we copy.
fn trim_trailing_punct(s: &str) -> &str {
    s.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>'))
}

/// Extract all URLs from `text`. De-dupes by `(type, raw)` keeping the
/// last occurrence (and its surrounding context line) per spec.
pub fn extract(text: &str) -> Vec<Match> {
    let re = url_regex();
    let mut by_raw: HashMap<String, Match> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for line in text.lines() {
        for m in re.find_iter(line) {
            let raw = trim_trailing_punct(m.as_str()).to_string();
            if raw.is_empty() {
                continue;
            }
            let mut fields = HashMap::new();
            fields.insert("url".to_string(), raw.clone());
            if let Some(scheme_end) = raw.find("://") {
                let scheme = &raw[..scheme_end];
                fields.insert("scheme".to_string(), scheme.to_string());
                let after_scheme = &raw[scheme_end + 3..];
                let host_end = after_scheme.find(|c: char| matches!(c, '/' | '?' | '#'))
                    .unwrap_or(after_scheme.len());
                let host = &after_scheme[..host_end];
                fields.insert("host".to_string(), host.to_string());
            }
            let entry = Match {
                ty: MatchType::Url,
                raw: raw.clone(),
                display: raw.clone(),
                context: line.to_string(),
                fields,
            };
            if !by_raw.contains_key(&raw) {
                order.push(raw.clone());
            }
            by_raw.insert(raw, entry);
        }
    }

    // Preserve recency: latest-seen first.
    order.reverse();
    order.into_iter().filter_map(|k| by_raw.remove(&k)).collect()
}

/// Take the last `n` lines of a scrollback capture. Phase 1 hardcodes
/// the default cap; config-driven grab mode lands in Phase 7.
pub fn take_recent(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_basic_https_url() {
        let matches = extract("see https://example.com/foo for details");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].raw, "https://example.com/foo");
        assert_eq!(matches[0].fields["scheme"], "https");
        assert_eq!(matches[0].fields["host"], "example.com");
    }

    #[test]
    fn trims_trailing_punctuation() {
        let matches = extract("more at https://example.com/foo.");
        assert_eq!(matches[0].raw, "https://example.com/foo");
    }

    #[test]
    fn dedupes_keeping_latest() {
        let text = "first https://a.example.com/x\nlast https://a.example.com/x";
        let matches = extract(text);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].context, "last https://a.example.com/x");
    }

    #[test]
    fn recency_order_latest_first() {
        let text = "first https://a.example.com\nsecond https://b.example.com";
        let matches = extract(text);
        assert_eq!(matches[0].raw, "https://b.example.com");
        assert_eq!(matches[1].raw, "https://a.example.com");
    }

    #[test]
    fn handles_multiple_schemes() {
        let matches = extract("a http://x.example.com b git://y.example.com c ssh://z.example.com");
        let raws: Vec<_> = matches.iter().map(|m| m.raw.as_str()).collect();
        assert!(raws.contains(&"http://x.example.com"));
        assert!(raws.contains(&"git://y.example.com"));
        assert!(raws.contains(&"ssh://z.example.com"));
    }

    #[test]
    fn take_recent_caps_to_n_lines() {
        let text = (1..=200).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
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
