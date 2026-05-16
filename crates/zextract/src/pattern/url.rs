//! URL pattern: `(https?|ftp|file|git|ssh)://...` style URIs.
//! Captures `{url}`, `{scheme}`, `{host}` for action templates.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};
use crate::pattern::trim_trailing_punct;

fn url_regex() -> &'static Regex {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    URL_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:https?|ftp|file|git|ssh)://[^\s<>'`\[\](){}]+")
            .expect("url regex compiles")
    })
}

pub fn extract(text: &str) -> Vec<Match> {
    let re = url_regex();
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in text.lines() {
        for m in re.find_iter(line) {
            let raw_unt = m.as_str();
            let raw = trim_trailing_punct(raw_unt).to_string();
            if raw.is_empty() {
                continue;
            }
            let span_start = byte_offset_of_line + m.start();
            let span_end = span_start + raw.len();

            let mut fields = HashMap::new();
            fields.insert("url".to_string(), raw.clone());
            if let Some(scheme_end) = raw.find("://") {
                fields.insert("scheme".to_string(), raw[..scheme_end].to_string());
                let after = &raw[scheme_end + 3..];
                let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
                fields.insert("host".to_string(), after[..host_end].to_string());
            }
            out.push(Match {
                ty: MatchType::Url,
                raw: raw.clone(),
                display: raw,
                context: line.to_string(),
                label: None,
                span: (span_start, span_end),
                fields,
            });
        }
        byte_offset_of_line += line.len() + 1; // +1 for the '\n' we split on
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_basic_https_url() {
        let m = extract("see https://example.com/foo for details");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "https://example.com/foo");
        assert_eq!(m[0].fields["scheme"], "https");
        assert_eq!(m[0].fields["host"], "example.com");
    }

    #[test]
    fn trims_trailing_punctuation() {
        let m = extract("more at https://example.com/foo.");
        assert_eq!(m[0].raw, "https://example.com/foo");
    }

    #[test]
    fn handles_multiple_schemes() {
        let m = extract("a http://x.example.com b git://y.example.com c ssh://z.example.com");
        let raws: Vec<_> = m.iter().map(|x| x.raw.as_str()).collect();
        assert!(raws.contains(&"http://x.example.com"));
        assert!(raws.contains(&"git://y.example.com"));
        assert!(raws.contains(&"ssh://z.example.com"));
    }

    #[test]
    fn populates_span() {
        let m = extract("see https://example.com here");
        assert_eq!(m[0].span, (4, 23));
    }
}
