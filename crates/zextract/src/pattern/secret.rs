//! Secret / API key detection.
//!
//! Two strategies in order:
//!   1. Curated regexes for known formats (JWT, AWS, GitHub, GitLab,
//!      Stripe, OpenAI, Anthropic, Slack, Bearer). High precision.
//!   2. Entropy fallback for long high-entropy tokens not already
//!      matched by a specific format. Lower confidence but catches
//!      unknown formats. Filters per planning.md Q15:
//!         - length 20–200
//!         - at least 3 character classes (lower/upper/digit/special)
//!         - Shannon entropy ≥ 3.5 bits/char
//!
//! Captures `{secret}` (the raw token) and `{secret_format}`
//! (e.g. "jwt", "github", "entropy").

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::config::schema::SecretPatternConfig;
use crate::extract::{Match, MatchType};

struct FormatPattern {
    name: &'static str,
    regex: Regex,
}

fn formats() -> &'static [FormatPattern] {
    static FORMATS: OnceLock<Vec<FormatPattern>> = OnceLock::new();
    FORMATS.get_or_init(|| {
        let raws = [
            (
                "jwt",
                r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            ),
            ("aws", r"\bA(?:KIA|SIA)[0-9A-Z]{16}\b"),
            ("github", r"\bgh[pousr]_[A-Za-z0-9]{36,}\b"),
            ("github_pat", r"\bgithub_pat_[A-Za-z0-9_]{82}\b"),
            (
                "gitlab",
                r"\bg(?:lpat|loas|lptt|lrt|lsoat|lagent)-[A-Za-z0-9_-]{20,}\b",
            ),
            (
                "stripe",
                r"\b(?:sk_live|sk_test|pk_live|pk_test|rk_live|whsec)_[A-Za-z0-9]{24,}\b",
            ),
            ("openai", r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}\b"),
            (
                "anthropic",
                r"\bsk-ant-(?:api|admin)\d{2}-[A-Za-z0-9_-]{20,}\b",
            ),
            ("slack", r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b"),
            ("bearer", r"Bearer\s+[A-Za-z0-9_\-\.~+/]+={0,2}"),
        ];
        raws.iter()
            .map(|(name, p)| FormatPattern {
                name,
                regex: Regex::new(p).expect("secret format regex compiles"),
            })
            .collect()
    })
}

pub fn extract(text: &str, config: &SecretPatternConfig) -> Vec<Match> {
    let mut out = Vec::new();
    let mut matched_spans: Vec<(usize, usize)> = Vec::new();
    let mut byte_offset_of_line = 0usize;

    for line in text.lines() {
        // Curated formats first.
        for fp in formats() {
            for m in fp.regex.find_iter(line) {
                let span_start = byte_offset_of_line + m.start();
                let span_end = span_start + m.as_str().len();
                if overlaps(&matched_spans, span_start, span_end) {
                    continue;
                }
                matched_spans.push((span_start, span_end));
                push(&mut out, m.as_str(), fp.name, line, span_start, span_end);
            }
        }
        // Entropy fallback on remaining tokens (skipped when disabled via config).
        if config.entropy_filter {
            for (raw, off) in tokens_with_byte_offsets(line) {
                let span_start = byte_offset_of_line + off;
                let span_end = span_start + raw.len();
                if overlaps(&matched_spans, span_start, span_end) {
                    continue;
                }
                if !passes_entropy_filter(raw) {
                    continue;
                }
                matched_spans.push((span_start, span_end));
                push(&mut out, raw, "entropy", line, span_start, span_end);
            }
        }
        byte_offset_of_line += line.len() + 1;
    }
    out
}

fn overlaps(spans: &[(usize, usize)], start: usize, end: usize) -> bool {
    spans.iter().any(|&(s, e)| !(end <= s || start >= e))
}

fn push(
    out: &mut Vec<Match>,
    raw: &str,
    format: &str,
    context: &str,
    span_start: usize,
    span_end: usize,
) {
    let mut fields = HashMap::new();
    fields.insert("secret".to_string(), raw.to_string());
    fields.insert("secret_format".to_string(), format.to_string());
    out.push(Match {
        ty: MatchType::Secret,
        raw: raw.to_string(),
        display: raw.to_string(),
        context: context.to_string(),
        label: None,
        span: (span_start, span_end),
        fields,
    });
}

fn tokens_with_byte_offsets(line: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if let Ok(s) = std::str::from_utf8(&bytes[start..i]) {
            out.push((s, start));
        }
    }
    out
}

fn passes_entropy_filter(s: &str) -> bool {
    let len = s.len();
    if !(20..=200).contains(&len) {
        return false;
    }
    let mut has_lower = false;
    let mut has_upper = false;
    let mut has_digit = false;
    let mut has_special = false;
    for c in s.chars() {
        if c.is_ascii_lowercase() {
            has_lower = true;
        } else if c.is_ascii_uppercase() {
            has_upper = true;
        } else if c.is_ascii_digit() {
            has_digit = true;
        } else if matches!(c, '_' | '-' | '+' | '/' | '=' | '.') {
            has_special = true;
        } else {
            // Other chars present → not a secret-shaped token.
            return false;
        }
    }
    let class_count =
        (has_lower as u8) + (has_upper as u8) + (has_digit as u8) + (has_special as u8);
    if class_count < 3 {
        return false;
    }
    shannon_entropy_bits(s) >= 3.5
}

fn shannon_entropy_bits(s: &str) -> f64 {
    let mut counts = [0u32; 256];
    let mut total = 0u32;
    for b in s.bytes() {
        counts[b as usize] += 1;
        total += 1;
    }
    if total == 0 {
        return 0.0;
    }
    let mut entropy = 0.0f64;
    let total_f = total as f64;
    for &c in counts.iter() {
        if c > 0 {
            let p = c as f64 / total_f;
            entropy -= p * p.log2();
        }
    }
    entropy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_on() -> SecretPatternConfig {
        SecretPatternConfig {
            entropy_filter: true,
        }
    }

    fn cfg_off() -> SecretPatternConfig {
        SecretPatternConfig {
            entropy_filter: false,
        }
    }

    #[test]
    fn detects_jwt() {
        let token =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let m = extract(&format!("Authorization: {}", token), &cfg_on());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["secret_format"], "jwt");
    }

    #[test]
    fn detects_aws_access_key() {
        let m = extract("AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE", &cfg_on());
        assert!(m.iter().any(|x| x.fields["secret_format"] == "aws"));
    }

    #[test]
    fn detects_github_token() {
        let m = extract(
            "export TOKEN=ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789AB",
            &cfg_on(),
        );
        assert!(m.iter().any(|x| x.fields["secret_format"] == "github"));
    }

    #[test]
    fn detects_gitlab_pat() {
        let m = extract("GITLAB_TOKEN=glpat-aBcDeFgHiJkLmNoPqRsT", &cfg_on());
        assert!(m.iter().any(|x| x.fields["secret_format"] == "gitlab"));
    }

    #[test]
    fn detects_stripe_key() {
        let m = extract("STRIPE=sk_live_aBcDeFgHiJkLmNoPqRsTuVwX", &cfg_on());
        assert!(m.iter().any(|x| x.fields["secret_format"] == "stripe"));
    }

    #[test]
    fn detects_bearer() {
        let m = extract("Authorization: Bearer aBcDeFgHi", &cfg_on());
        assert!(m.iter().any(|x| x.fields["secret_format"] == "bearer"));
    }

    #[test]
    fn entropy_fallback_catches_unknown_format() {
        // 30 chars, 3 classes (upper+lower+digit), high entropy
        let m = extract("token: aBc12345XyZ987KkPpQqRrSsTtUu", &cfg_on());
        assert!(m
            .iter()
            .any(|x| x.fields.get("secret_format").map(|s| s.as_str()) == Some("entropy")));
    }

    #[test]
    fn entropy_rejects_pure_repetition() {
        let m = extract("token: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &cfg_on());
        assert!(m.is_empty());
    }

    #[test]
    fn entropy_rejects_short_tokens() {
        let m = extract("token: aB1cD2eF3", &cfg_on());
        assert!(m.is_empty());
    }

    #[test]
    fn specific_match_suppresses_entropy_fallback() {
        // A JWT also passes the entropy filter. Curated match takes
        // precedence — exactly one match emitted, with format=jwt.
        let token =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let m = extract(token, &cfg_on());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["secret_format"], "jwt");
    }

    #[test]
    fn entropy_filter_disabled_suppresses_fallback_matches() {
        // High-entropy unknown token that would fire with entropy_filter=true.
        let m = extract("token: aBc12345XyZ987KkPpQqRrSsTtUu", &cfg_off());
        assert!(!m
            .iter()
            .any(|x| x.fields.get("secret_format").map(|s| s.as_str()) == Some("entropy")));
    }

    #[test]
    fn entropy_filter_disabled_still_detects_curated_formats() {
        // Curated formats must fire regardless of the entropy_filter setting.
        let m = extract("STRIPE=sk_live_aBcDeFgHiJkLmNoPqRsTuVwX", &cfg_off());
        assert!(m.iter().any(|x| x.fields["secret_format"] == "stripe"));
    }
}
