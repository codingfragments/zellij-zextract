//! Git SHA pattern: 7-40 hex chars at word boundaries.
//!
//! Heuristic: require at least one a-f letter to avoid pure-numeric noise.
//! ANSI escape codes are stripped before scanning so `git log --color` output
//! works — `\x1b[33m` ends with `m` (word char), which breaks `\b` if present.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};

fn ansi_re() -> &'static Regex {
    static ANSI_RE: OnceLock<Regex> = OnceLock::new();
    // Covers common SGR sequences (color, bold, reset) and cursor-movement codes.
    ANSI_RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[mKJHfABCDsu]").expect("ansi regex compiles"))
}

fn sha_regex() -> &'static Regex {
    static SHA_RE: OnceLock<Regex> = OnceLock::new();
    SHA_RE.get_or_init(|| Regex::new(r"\b[0-9a-fA-F]{7,40}\b").expect("sha regex compiles"))
}

/// Strip ANSI escape sequences from `text`. Returns a Cow to avoid allocating
/// when no escape is present.
fn strip_ansi(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\x1b') {
        ansi_re().replace_all(text, "").into_owned().into()
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

pub fn extract(text: &str) -> Vec<Match> {
    // Spans are in terms of the stripped text. Dedup is span-order-based
    // within this type only, so coordinate consistency is preserved.
    let stripped = strip_ansi(text);
    let re = sha_regex();
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in stripped.lines() {
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

    // ── helpers ────────────────────────────────────────────────────────────

    fn raw_set(text: &str) -> std::collections::HashSet<String> {
        extract(text).into_iter().map(|m| m.raw).collect()
    }

    // ── existing behaviour preserved ───────────────────────────────────────

    #[test]
    fn short_sha() {
        let m = extract("commit abc1234 by alice");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "abc1234");
        assert_eq!(m[0].fields["sha"], "abc1234");
    }

    #[test]
    fn no_match_for_41_char_hex_run() {
        // 41 contiguous hex chars — exceeds SHA-1 length, should not match.
        let s = "a".repeat(40) + "b"; // 41 hex chars, all alpha so passes the a-f check
        assert_eq!(s.len(), 41);
        let m = extract(&format!("prefix {s} suffix"));
        assert!(m.is_empty(), "41-char hex run should not match: {m:?}");
    }

    #[test]
    fn exactly_40_chars() {
        let m = extract("commit abc1234def5678fedcba0987654321abcd");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "abc1234def5678fedcba0987654321abcd");
        // 34 chars — all within range
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

    // ── git log --oneline (plain) ──────────────────────────────────────────

    #[test]
    fn oneline_hash_at_line_start() {
        let m = extract("f2d1431 docs: backfill CHANGELOG for 0.2.0 through 0.3.1");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431");
    }

    #[test]
    fn oneline_multiple_commits() {
        let input = "\
f2d1431 docs: backfill CHANGELOG for 0.2.0 through 0.3.1
d0ef973 chore: release v0.3.1
dc97c4b Merge pull request #22 from codingfragments/feature/auto-resize-min-size
eb210b7 feat: auto-grow float to 95% when render area below minimum
ce2d85d docs: add demo GIF to README";
        let got = raw_set(input);
        assert!(got.contains("f2d1431"), "f2d1431 missing");
        assert!(got.contains("d0ef973"), "d0ef973 missing");
        assert!(got.contains("dc97c4b"), "dc97c4b missing");
        assert!(got.contains("eb210b7"), "eb210b7 missing");
        assert!(got.contains("ce2d85d"), "ce2d85d missing");
        assert_eq!(got.len(), 5);
    }

    // ── git log --oneline --color=always ──────────────────────────────────

    #[test]
    fn oneline_with_color_hash() {
        // \x1b[33m…\x1b[m wraps the abbreviated hash; the `m` before the hash
        // char is a word char that breaks \b without ANSI stripping.
        let input = "\x1b[33mf2d1431\x1b[m docs: backfill CHANGELOG for 0.2.0 through 0.3.1";
        let m = extract(input);
        assert_eq!(m.len(), 1, "should find exactly one sha, got: {m:?}");
        assert_eq!(m[0].raw, "f2d1431");
    }

    #[test]
    fn oneline_color_multiple_commits() {
        let input = "\
\x1b[33mf2d1431\x1b[m docs: backfill CHANGELOG for 0.2.0 through 0.3.1\n\
\x1b[33md0ef973\x1b[m chore: release v0.3.1\n\
\x1b[33mdc97c4b\x1b[m Merge pull request #22 from codingfragments/feature/auto-resize-min-size\n\
\x1b[33meb210b7\x1b[m feat: auto-grow float to 95% when render area below minimum\n\
\x1b[33mce2d85d\x1b[m docs: add demo GIF to README";
        let got = raw_set(input);
        for sha in &["f2d1431", "d0ef973", "dc97c4b", "eb210b7", "ce2d85d"] {
            assert!(got.contains(*sha), "{sha} missing from {got:?}");
        }
        assert_eq!(got.len(), 5);
    }

    // ── git log (full format) ──────────────────────────────────────────────

    #[test]
    fn full_log_commit_line_plain() {
        let input = "commit f2d1431aedae0d23a36c71a4882fa97a76271960";
        let m = extract(input);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431aedae0d23a36c71a4882fa97a76271960");
    }

    #[test]
    fn full_log_commit_line_colored() {
        // Color wraps "commit <hash>" together; the space still provides a word boundary.
        let input = "\x1b[33mcommit f2d1431aedae0d23a36c71a4882fa97a76271960\x1b[m";
        let m = extract(input);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431aedae0d23a36c71a4882fa97a76271960");
    }

    #[test]
    fn full_log_merge_line() {
        let input = "Merge: abc1234 def5678";
        let got = raw_set(input);
        assert!(got.contains("abc1234"), "first merge parent missing");
        assert!(got.contains("def5678"), "second merge parent missing");
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn full_log_block() {
        let input = "\
commit f2d1431aedae0d23a36c71a4882fa97a76271960\n\
Author: Stefan Marx <stefan@example.com>\n\
Date:   Mon Jun 1 00:51:06 2026 +0200\n\
\n\
    docs: backfill CHANGELOG\n";
        let m = extract(input);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431aedae0d23a36c71a4882fa97a76271960");
    }

    // ── git log --oneline --graph ──────────────────────────────────────────

    #[test]
    fn graph_simple_star() {
        // "* <hash> message" — `*` and space are non-word, so \b works.
        let m = extract("* f2d1431 docs: backfill CHANGELOG");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431");
    }

    #[test]
    fn graph_with_pipes() {
        let m = extract("| * f2d1431 docs: backfill CHANGELOG");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431");
    }

    #[test]
    fn graph_colored() {
        // Graph output with colors: graph chars may also be colored.
        let input =
            "\x1b[31m|\x1b[m \x1b[31m*\x1b[m \x1b[33mf2d1431\x1b[m docs: backfill CHANGELOG";
        let m = extract(input);
        assert_eq!(m.len(), 1, "got: {m:?}");
        assert_eq!(m[0].raw, "f2d1431");
    }

    // ── inline / prose references ─────────────────────────────────────────

    #[test]
    fn inline_reference() {
        let m = extract("see commit abc1234f for details");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "abc1234f");
    }

    #[test]
    fn no_match_in_pure_text() {
        // "feature" contains no hex-only chars beyond f — but it's mixed alpha
        // and has no letter in a-f... wait, 'e', 'a' are in a-f. But "feature"
        // is 7 chars and all alpha — passes the hex char check. It should NOT
        // match because 't', 'u', 'r' are not hex chars: \b[0-9a-fA-F]{7,40}\b
        // requires ALL chars to be hex.
        let m = extract("feature branch pushed");
        assert!(m.is_empty(), "non-hex word should not match: {m:?}");
    }

    #[test]
    fn no_match_for_over_40_hex_chars() {
        // 41 hex chars — should not match as it exceeds a valid SHA length.
        let m = extract("abc1234def5678fedcba0987654321abc12345678");
        assert!(m.is_empty(), "41-char hex run should not match");
    }
}
