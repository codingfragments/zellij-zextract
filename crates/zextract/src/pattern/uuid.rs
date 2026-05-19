//! UUID pattern: any version (8-4-4-4-12 hex). Captures `{uuid}` field.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};

fn uuid_regex() -> &'static Regex {
    static UUID_RE: OnceLock<Regex> = OnceLock::new();
    UUID_RE.get_or_init(|| {
        Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("uuid regex compiles")
    })
}

pub fn extract(text: &str) -> Vec<Match> {
    let re = uuid_regex();
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in text.lines() {
        for m in re.find_iter(line) {
            let raw = m.as_str().to_string();
            let span_start = byte_offset_of_line + m.start();
            let span_end = span_start + raw.len();
            let mut fields = HashMap::new();
            fields.insert("uuid".to_string(), raw.clone());
            out.push(Match {
                ty: MatchType::Uuid,
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

    #[test]
    fn extracts_uuid() {
        let m = extract("trace id 550e8400-e29b-41d4-a716-446655440000 in logs");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(m[0].fields["uuid"], "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn uppercase_uuid() {
        let m = extract("ID 550E8400-E29B-41D4-A716-446655440000 here");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn rejects_partial() {
        let m = extract("ID 550e8400-e29b-41d4-a716 (incomplete)");
        assert!(m.is_empty());
    }
}
