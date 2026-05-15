//! File path pattern: absolute (`/...`), home-relative (`~/...`),
//! explicit-relative (`./...`, `../...`), relative-with-slash
//! (`src/main.rs`), and filenames with extensions (`Cargo.toml`).
//! Optional `:line[:col]` suffix.
//!
//! Captures `{file}` (path sans line/col), `{line}`, `{col}`,
//! `{dir}` (parent), `{basename}`, `{ext}`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};
use crate::pattern::trim_trailing_punct;

fn file_regex() -> &'static Regex {
    static FILE_RE: OnceLock<Regex> = OnceLock::new();
    FILE_RE.get_or_init(|| {
        // Branches:
        //   ~?/...               absolute or home-relative ("/etc/passwd", "~/cfg")
        //   \.\.?/...            explicit-relative ("./bin", "../foo")
        //   [\w.\-]+/[\w.\-/]+   relative with at least one slash ("src/main.rs")
        //   [\w.\-]+\.[A-Za-z]\w{0,9}  bare filename with extension ("Cargo.toml")
        // Optional :line[:col] suffix.
        Regex::new(concat!(
            r"(?:~?/[\w.\-/]+",
            r"|\.\.?/[\w.\-/]+",
            r"|[\w.\-]+/[\w.\-/]+",
            r"|[\w.\-]+\.[A-Za-z]\w{0,9})",
            r"(?::\d+(?::\d+)?)?",
        ))
        .expect("file regex compiles")
    })
}

pub fn extract(text: &str) -> Vec<Match> {
    let re = file_regex();
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in text.lines() {
        for m in re.find_iter(line) {
            // Gate on the character immediately preceding the match — accept
            // only word-start contexts. Rejects file-shaped fragments inside
            // URLs (`/foo` after `://` or after another `/`) and inside
            // run-on words (`runfile/foo`).
            let prev = if m.start() == 0 {
                None
            } else {
                line.as_bytes().get(m.start() - 1).copied()
            };
            if !ok_preceding_byte(prev) {
                continue;
            }
            let raw_unt = m.as_str();
            let raw = trim_trailing_punct(raw_unt).to_string();
            if raw.len() < 3 {
                // Skip very short matches ("ab.c") — too noisy.
                continue;
            }
            // Skip if it's a pure-numeric "file" like "2.5" — that's a number.
            if looks_numeric(&raw) {
                continue;
            }

            let (path_part, line_part, col_part) = split_line_col(&raw);

            let mut fields = HashMap::new();
            fields.insert("file".to_string(), path_part.to_string());
            fields.insert("line".to_string(), line_part.unwrap_or_default());
            fields.insert("col".to_string(), col_part.unwrap_or_default());

            let p = Path::new(path_part);
            if let Some(parent) = p.parent().and_then(|p| p.to_str()) {
                fields.insert("dir".to_string(), parent.to_string());
            }
            if let Some(basename) = p.file_name().and_then(|s| s.to_str()) {
                fields.insert("basename".to_string(), basename.to_string());
            }
            if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                fields.insert("ext".to_string(), ext.to_string());
            }

            let span_start = byte_offset_of_line + m.start();
            let span_end = span_start + raw.len();

            out.push(Match {
                ty: MatchType::File,
                raw: raw.clone(),
                display: raw,
                context: line.to_string(),
                span: (span_start, span_end),
                fields,
            });
        }
        byte_offset_of_line += line.len() + 1;
    }
    out
}

fn split_line_col(raw: &str) -> (&str, Option<String>, Option<String>) {
    // Split off trailing :N or :N:N (file:42 or file:42:8). Anchored on
    // the LAST one or two colons so paths like /etc/host:5000 don't get
    // mis-split if they contain colons earlier.
    let bytes = raw.as_bytes();
    let last_colon = bytes.iter().rposition(|&b| b == b':');
    let Some(lc) = last_colon else {
        return (raw, None, None);
    };
    let after = &raw[lc + 1..];
    if !after.chars().all(|c| c.is_ascii_digit()) || after.is_empty() {
        return (raw, None, None);
    }
    let before = &raw[..lc];
    // Check for a second colon (column).
    if let Some(prev_colon) = before.as_bytes().iter().rposition(|&b| b == b':') {
        let mid = &before[prev_colon + 1..];
        if !mid.is_empty() && mid.chars().all(|c| c.is_ascii_digit()) {
            return (
                &before[..prev_colon],
                Some(mid.to_string()),
                Some(after.to_string()),
            );
        }
    }
    (before, Some(after.to_string()), None)
}

fn ok_preceding_byte(b: Option<u8>) -> bool {
    match b {
        None => true, // start of line
        Some(c) if c.is_ascii_whitespace() => true,
        Some(b'(' | b'[' | b'{' | b'<' | b'"' | b'\'' | b'`' | b'=' | b',' | b';') => true,
        _ => false,
    }
}

fn looks_numeric(s: &str) -> bool {
    // "2.5", "42.0" should not be treated as files.
    let no_suffix = s.split(':').next().unwrap_or(s);
    no_suffix
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path() {
        let m = extract("see /etc/passwd today");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "/etc/passwd");
        assert_eq!(m[0].fields["basename"], "passwd");
        assert_eq!(m[0].fields["dir"], "/etc");
    }

    #[test]
    fn relative_with_slash() {
        let m = extract("error in src/main.rs near top");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "src/main.rs");
        assert_eq!(m[0].fields["basename"], "main.rs");
        assert_eq!(m[0].fields["ext"], "rs");
    }

    #[test]
    fn line_col_suffix() {
        let m = extract("error: borrow at src/main.rs:42:8 here");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "src/main.rs:42:8");
        assert_eq!(m[0].fields["file"], "src/main.rs");
        assert_eq!(m[0].fields["line"], "42");
        assert_eq!(m[0].fields["col"], "8");
    }

    #[test]
    fn line_only_suffix() {
        let m = extract("see src/main.rs:42 for details");
        assert_eq!(m[0].fields["line"], "42");
        assert_eq!(m[0].fields["col"], "");
    }

    #[test]
    fn home_relative() {
        let m = extract("config at ~/dotfiles/zextract.kdl now");
        assert_eq!(m.len(), 1);
        assert!(m[0].raw.starts_with("~/dotfiles/"));
    }

    #[test]
    fn bare_filename_with_extension() {
        let m = extract("update Cargo.toml then run cargo build");
        let raws: Vec<_> = m.iter().map(|x| x.raw.as_str()).collect();
        assert!(raws.contains(&"Cargo.toml"));
    }

    #[test]
    fn skips_urls() {
        let m = extract("https://example.com/foo.html visited");
        // Should not emit a `file` match for the URL.
        assert!(m.is_empty());
    }

    #[test]
    fn skips_pure_numeric() {
        let m = extract("the value 2.5 is suspect");
        let raws: Vec<_> = m.iter().map(|x| x.raw.as_str()).collect();
        assert!(!raws.contains(&"2.5"));
    }

    #[test]
    fn trims_trailing_punct() {
        let m = extract("look at Cargo.toml.");
        assert_eq!(m[0].raw, "Cargo.toml");
    }
}
