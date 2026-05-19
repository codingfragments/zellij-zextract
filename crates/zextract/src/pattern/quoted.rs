//! Quoted strings: `"..."`, `'...'`, `` `...` ``.
//!
//! Captures `{content}` (the inner string), `{quote}` (the delimiter
//! used). v1 does not handle escaped quotes inside content — the
//! capture stops at the first matching delimiter. For action templates
//! like "copy unquoted" we surface `{content}` directly.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};

fn quoted_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Three alternations, each captures the inner content (group 1/2/3).
        Regex::new(r#""([^"]*)"|'([^']*)'|`([^`]*)`"#).expect("quoted regex compiles")
    })
}

pub fn extract(text: &str) -> Vec<Match> {
    let re = quoted_regex();
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in text.lines() {
        for caps in re.captures_iter(line) {
            let full = caps.get(0).unwrap();
            let (content, quote_ch) = if let Some(c) = caps.get(1) {
                (c.as_str(), '"')
            } else if let Some(c) = caps.get(2) {
                (c.as_str(), '\'')
            } else if let Some(c) = caps.get(3) {
                (c.as_str(), '`')
            } else {
                continue;
            };
            // Skip empty strings — too noisy.
            if content.is_empty() {
                continue;
            }
            // Skip very-short single-char strings unless they look intentional.
            if content.len() < 2 {
                continue;
            }
            let raw = full.as_str().to_string();
            let span_start = byte_offset_of_line + full.start();
            let span_end = span_start + raw.len();
            let mut fields = HashMap::new();
            fields.insert("content".to_string(), content.to_string());
            fields.insert("quote".to_string(), quote_ch.to_string());
            out.push(Match {
                ty: MatchType::QuotedString,
                raw,
                display: content.to_string(),
                context: line.to_string(),
                label: None,
                source_pane_id: None,
                span: (span_start, span_end),
                fields,
            });
        }
        byte_offset_of_line += line.len() + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_quoted() {
        let m = extract(r#"error: "permission denied" reported"#);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["content"], "permission denied");
        assert_eq!(m[0].fields["quote"], "\"");
        assert_eq!(m[0].display, "permission denied");
    }

    #[test]
    fn single_quoted() {
        let m = extract("config key 'database.url' present");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["content"], "database.url");
        assert_eq!(m[0].fields["quote"], "'");
    }

    #[test]
    fn backtick_quoted() {
        let m = extract("run `cargo build --release` to build");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["content"], "cargo build --release");
        assert_eq!(m[0].fields["quote"], "`");
    }

    #[test]
    fn multiple_in_line() {
        let m = extract(r#"got "alpha" and "beta" today"#);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn skips_empty() {
        let m = extract(r#"empty: "" no content"#);
        assert!(m.is_empty());
    }
}
