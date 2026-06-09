//! Git commit pattern: git log --oneline and git log full-format lines.
//!
//! `raw`     = the commit hash (what Insert/Copy operate on).
//! `display` = the oneline representation (hash + subject), for list display.
//! Default action: Insert.
//!
//! Handles `--color` output via ANSI stripping, and `--graph` output
//! via a prefix that permits `|`, `*`, `/`, `\`, and space before the hash.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};

fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[mKJHfABCDsu]").expect("ansi regex"))
}

fn strip_ansi(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\x1b') {
        ansi_re().replace_all(text, "").into_owned().into()
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// `git log --oneline [--graph]`: optional graph prefix, then hash, then subject.
fn oneline_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[\|\*\/\\\s]*([0-9a-fA-F]{7,40})\s+(\S.*)$").expect("git oneline regex")
    })
}

/// `git log` full-format commit header line.
fn commit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^commit ([0-9a-fA-F]{40})\b").expect("git commit regex")
    })
}

fn has_hex_letter(s: &str) -> bool {
    s.chars().any(|c| matches!(c, 'a'..='f' | 'A'..='F'))
}

pub fn extract(text: &str) -> Vec<Match> {
    let stripped = strip_ansi(text);
    let lines: Vec<&str> = stripped.lines().collect();
    let mut out = Vec::new();
    let mut byte_offset = 0usize;

    for (i, &line) in lines.iter().enumerate() {
        // ── git log --oneline [--graph] ────────────────────────────────────
        if let Some(caps) = oneline_re().captures(line) {
            let hash = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let subject = caps.get(2).map(|m| m.as_str()).unwrap_or("").trim();
            if hash.len() >= 7 && has_hex_letter(hash) {
                let hash_match = caps.get(1).unwrap();
                let span_start = byte_offset + hash_match.start();
                let span_end = byte_offset + hash_match.end();
                let display = format!("{hash} {subject}");
                let mut fields = HashMap::new();
                fields.insert("sha".to_string(), hash.to_string());
                fields.insert("subject".to_string(), subject.to_string());
                out.push(Match {
                    ty: MatchType::Git,
                    raw: hash.to_string(),
                    display,
                    context: line.to_string(),
                    label: None,
                    source_pane_id: None,
                    span: (span_start, span_end),
                    fields,
                });
                byte_offset += line.len() + 1;
                continue;
            }
        }

        // ── git log full-format commit line ───────────────────────────────
        if let Some(caps) = commit_re().captures(line) {
            let hash = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if has_hex_letter(hash) {
                let hash_match = caps.get(1).unwrap();
                let span_start = byte_offset + hash_match.start();
                let span_end = byte_offset + hash_match.end();
                // Look ahead for the commit subject (first indented non-empty line).
                let subject = lines[i + 1..].iter().take(7).find_map(|l| {
                    let t = l.trim_start();
                    if !t.is_empty() && l.starts_with("    ") {
                        Some(t)
                    } else {
                        None
                    }
                });
                let short = &hash[..7];
                let display = match subject {
                    Some(s) => format!("{short} {s}"),
                    None => hash.to_string(),
                };
                let mut fields = HashMap::new();
                fields.insert("sha".to_string(), hash.to_string());
                if let Some(s) = subject {
                    fields.insert("subject".to_string(), s.to_string());
                }
                out.push(Match {
                    ty: MatchType::Git,
                    raw: hash.to_string(),
                    display,
                    context: line.to_string(),
                    label: None,
                    source_pane_id: None,
                    span: (span_start, span_end),
                    fields,
                });
            }
        }

        byte_offset += line.len() + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_raw(text: &str) -> String {
        extract(text).into_iter().next().map(|m| m.raw).unwrap_or_default()
    }

    fn first_display(text: &str) -> String {
        extract(text).into_iter().next().map(|m| m.display).unwrap_or_default()
    }

    // ── git log --oneline ─────────────────────────────────────────────────

    #[test]
    fn oneline_plain() {
        let m = extract("f2d1431 docs: backfill CHANGELOG for 0.2.0 through 0.3.1");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431");
        assert_eq!(m[0].display, "f2d1431 docs: backfill CHANGELOG for 0.2.0 through 0.3.1");
        assert_eq!(m[0].fields["subject"], "docs: backfill CHANGELOG for 0.2.0 through 0.3.1");
    }

    #[test]
    fn oneline_color() {
        let input = "\x1b[33mf2d1431\x1b[m docs: backfill CHANGELOG for 0.2.0 through 0.3.1";
        let m = extract(input);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431");
        assert_eq!(m[0].display, "f2d1431 docs: backfill CHANGELOG for 0.2.0 through 0.3.1");
    }

    #[test]
    fn oneline_graph_star() {
        let m = extract("* f2d1431 docs: backfill CHANGELOG");
        assert_eq!(first_raw("* f2d1431 docs: backfill CHANGELOG"), "f2d1431");
        assert_eq!(m[0].display, "f2d1431 docs: backfill CHANGELOG");
    }

    #[test]
    fn oneline_graph_pipe_star() {
        assert_eq!(first_raw("| * f2d1431 docs: backfill CHANGELOG"), "f2d1431");
    }

    #[test]
    fn oneline_graph_colored() {
        let input = "\x1b[31m|\x1b[m \x1b[31m*\x1b[m \x1b[33mf2d1431\x1b[m docs: backfill CHANGELOG";
        assert_eq!(first_raw(input), "f2d1431");
        assert_eq!(first_display(input), "f2d1431 docs: backfill CHANGELOG");
    }

    #[test]
    fn oneline_multiple_commits() {
        let input = "\
f2d1431 docs: backfill CHANGELOG for 0.2.0 through 0.3.1
d0ef973 chore: release v0.3.1
dc97c4b Merge pull request #22 from codingfragments/feature/auto-resize-min-size";
        let m = extract(input);
        assert_eq!(m.len(), 3);
        let raws: Vec<_> = m.iter().map(|x| x.raw.as_str()).collect();
        assert!(raws.contains(&"f2d1431"));
        assert!(raws.contains(&"d0ef973"));
        assert!(raws.contains(&"dc97c4b"));
    }

    // ── git log full format ───────────────────────────────────────────────

    #[test]
    fn full_log_plain() {
        let input = "commit f2d1431aedae0d23a36c71a4882fa97a76271960";
        let m = extract(input);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431aedae0d23a36c71a4882fa97a76271960");
        // No subject on same line — display falls back to hash only.
        assert_eq!(m[0].display, "f2d1431aedae0d23a36c71a4882fa97a76271960");
    }

    #[test]
    fn full_log_with_subject() {
        let input = "\
commit f2d1431aedae0d23a36c71a4882fa97a76271960
Author: Stefan Marx <stefan@example.com>
Date:   Mon Jun 1 00:51:06 2026 +0200

    docs: backfill CHANGELOG";
        let m = extract(input);
        // One git match (the commit line); Author/Date are not git matches.
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431aedae0d23a36c71a4882fa97a76271960");
        assert_eq!(m[0].display, "f2d1431 docs: backfill CHANGELOG");
        assert_eq!(m[0].fields["subject"], "docs: backfill CHANGELOG");
    }

    #[test]
    fn full_log_colored() {
        let input = "\x1b[33mcommit f2d1431aedae0d23a36c71a4882fa97a76271960\x1b[m\nAuthor: Stefan Marx <stefan@example.com>\nDate:   Mon Jun 1 00:51:06 2026 +0200\n\n    docs: backfill CHANGELOG";
        let m = extract(input);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431aedae0d23a36c71a4882fa97a76271960");
        assert_eq!(m[0].display, "f2d1431 docs: backfill CHANGELOG");
    }

    #[test]
    fn full_log_with_decorate() {
        // git log --decorate appends (HEAD -> main, ...) after hash
        let input = "commit f2d1431aedae0d23a36c71a4882fa97a76271960 (HEAD -> main, tag: v0.3.1)";
        let m = extract(input);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "f2d1431aedae0d23a36c71a4882fa97a76271960");
    }

    // ── non-git lines do not match ────────────────────────────────────────

    #[test]
    fn no_match_pure_numeric_at_line_start() {
        let m = extract("1234567 some text here");
        assert!(m.is_empty(), "pure-numeric token should not match: {m:?}");
    }

    #[test]
    fn no_match_short_hash() {
        let m = extract("abc123 too short");
        assert!(m.is_empty());
    }

    #[test]
    fn no_match_non_hex_word_at_start() {
        // "feature" contains non-hex letters (t, u, r) — no match.
        let m = extract("feature branch pushed");
        assert!(m.is_empty());
    }

    #[test]
    fn no_match_hash_mid_line_without_commit_prefix() {
        // A hash in the middle of a prose line is NOT a git log line.
        let m = extract("see commit abc1234f for details");
        assert!(m.is_empty(), "mid-line hash without git log structure should not match");
    }
}
