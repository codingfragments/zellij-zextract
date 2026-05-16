//! Git SHA pattern: 7-40 hex chars at word boundaries.
//!
//! Heuristic per planning.md: require at least one a-f letter in the
//! match to avoid pure-numeric noise. Phase 7 will optionally tighten
//! this with context cues ("commit ", "git log" preceding).

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};

fn sha_regex() -> &'static Regex {
    static SHA_RE: OnceLock<Regex> = OnceLock::new();
    SHA_RE.get_or_init(|| Regex::new(r"\b[0-9a-fA-F]{7,40}\b").expect("sha regex compiles"))
}

pub fn extract(text: &str) -> Vec<Match> {
    let re = sha_regex();
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in text.lines() {
        for m in re.find_iter(line) {
            let raw = m.as_str().to_string();
            // Require at least one a-f letter — pure-numeric runs (line numbers,
            // timestamps) would otherwise show up as "SHAs".
            if !raw.chars().any(|c| matches!(c, 'a'..='f' | 'A'..='F')) {
                continue;
            }
            let span_start = byte_offset_of_line + m.start();
            let span_end = span_start + raw.len();
            let mut fields = HashMap::new();
            fields.insert("sha".to_string(), raw.clone());
            out.push(Match {
                ty: MatchType::Sha,
                raw: raw.clone(),
                display: raw,
                context: line.to_string(),
                label: None, span: (span_start, span_end),
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
    fn short_sha() {
        let m = extract("commit abc1234 by alice");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "abc1234");
        assert_eq!(m[0].fields["sha"], "abc1234");
    }

    #[test]
    fn full_sha() {
        let m = extract("git show abc1234def5678fedcba0987654321 0fedcba1234");
        let raws: Vec<_> = m.iter().map(|x| x.raw.as_str()).collect();
        assert!(raws.contains(&"abc1234def5678fedcba0987654321"));
    }

    #[test]
    fn skips_pure_numeric_runs() {
        let m = extract("the value 12345678 appears");
        assert!(m.is_empty());
    }

    #[test]
    fn skips_too_short() {
        let m = extract("ab12 too short");
        assert!(m.is_empty());
    }
}
