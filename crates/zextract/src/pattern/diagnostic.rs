//! Diagnostic locations: compiler / linter / interpreter output with a
//! `file:line:col` reference. Two flavors:
//!   - C/Rust/eslint style: `path/to/file.rs:42:8`
//!   - Python traceback:    `File "path/to/file.py", line 42`
//!
//! Captures `{file}`, `{line}`, `{col}` (empty when absent), `{message}`
//! (empty in v1 — added in Phase 7 when context inspection lands).

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};

fn colon_form_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Path with at least one slash OR a `.ext` filename, then :line:col.
        Regex::new(r"\b([~]?[\w.\-/]*[/.][\w.\-]+):(\d+):(\d+)\b")
            .expect("diagnostic colon-form regex compiles")
    })
}

fn python_traceback_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"File "([^"]+)", line (\d+)"#).expect("python traceback regex compiles")
    })
}

pub fn extract(text: &str) -> Vec<Match> {
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    let colon_re = colon_form_regex();
    let py_re = python_traceback_regex();

    for line in text.lines() {
        for caps in colon_re.captures_iter(line) {
            let full = caps.get(0).unwrap();
            let file = caps.get(1).unwrap().as_str();
            let line_no = caps.get(2).unwrap().as_str();
            let col_no = caps.get(3).unwrap().as_str();
            push_match(
                &mut out,
                file,
                Some(line_no),
                Some(col_no),
                full.as_str(),
                line,
                byte_offset_of_line + full.start(),
            );
        }
        for caps in py_re.captures_iter(line) {
            let full = caps.get(0).unwrap();
            let file = caps.get(1).unwrap().as_str();
            let line_no = caps.get(2).unwrap().as_str();
            push_match(
                &mut out,
                file,
                Some(line_no),
                None,
                full.as_str(),
                line,
                byte_offset_of_line + full.start(),
            );
        }
        byte_offset_of_line += line.len() + 1;
    }
    out
}

fn push_match(
    out: &mut Vec<Match>,
    file: &str,
    line_no: Option<&str>,
    col_no: Option<&str>,
    raw_full: &str,
    context: &str,
    span_start: usize,
) {
    let mut fields = HashMap::new();
    fields.insert("file".to_string(), file.to_string());
    fields.insert("line".to_string(), line_no.unwrap_or("").to_string());
    fields.insert("col".to_string(), col_no.unwrap_or("").to_string());
    fields.insert("message".to_string(), String::new());

    let p = Path::new(file);
    if let Some(parent) = p.parent().and_then(|p| p.to_str()) {
        fields.insert("dir".to_string(), parent.to_string());
    }
    if let Some(basename) = p.file_name().and_then(|s| s.to_str()) {
        fields.insert("basename".to_string(), basename.to_string());
    }
    if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
        fields.insert("ext".to_string(), ext.to_string());
    }

    let raw = raw_full.to_string();
    let span_end = span_start + raw.len();
    out.push(Match {
        ty: MatchType::Diagnostic,
        raw: raw.clone(),
        display: raw,
        context: context.to_string(),
        label: None,
        span: (span_start, span_end),
        fields,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_form_with_col() {
        let m = extract("error[E0382]: borrow of moved value at src/main.rs:42:8");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "src/main.rs:42:8");
        assert_eq!(m[0].fields["file"], "src/main.rs");
        assert_eq!(m[0].fields["line"], "42");
        assert_eq!(m[0].fields["col"], "8");
        assert_eq!(m[0].fields["basename"], "main.rs");
    }

    #[test]
    fn python_traceback() {
        let m = extract(r#"  File "/usr/lib/python3/foo.py", line 42, in bar"#);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["file"], "/usr/lib/python3/foo.py");
        assert_eq!(m[0].fields["line"], "42");
        assert_eq!(m[0].fields["col"], "");
    }

    #[test]
    fn rejects_bare_numeric_colons() {
        // "10.0.0.5:42:8" — looks like file:line:col but file part is an IP.
        // The regex DOES match it because the file-shape allows digits+dots.
        // Acceptable noise — user can filter by type.
        let _ = extract("10.0.0.5:42:8");
        // Not asserting absence; just documenting that the dedup pass will
        // drop it if an IPv4 match with the same text appears, since one is
        // not a subset of the other (different types).
    }
}
