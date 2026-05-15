//! Pattern modules — one file per pattern type. Each module exposes a
//! `pub fn extract(text: &str) -> Vec<Match>` that finds all matches of
//! its type, populates type-specific fields, and stamps a byte-offset
//! span. `extract::extract` calls all of them and dedupes by
//! `(type, raw)` keeping the latest occurrence (per spec Q25).

pub mod file;
pub mod ipv4;
pub mod sha;
pub mod url;
pub mod uuid;

/// Trim trailing punctuation that's commonly adjacent to a match in
/// prose but not part of it. Used by URL, file, diagnostic etc.
pub fn trim_trailing_punct(s: &str) -> &str {
    s.trim_end_matches(|c: char| {
        matches!(
            c,
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>' | '"' | '\''
        )
    })
}
