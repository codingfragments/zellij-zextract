//! IPv6 (full form). Off-by-default per spec; included for Phase 7 to
//! toggle via config. Phase 3 emits matches if the pattern finds any.
//!
//! Recognizes:
//!   - Full form:    a:b:c:d:e:f:g:h (8 groups of 1-4 hex)
//!   - Compressed:   a::b   (single :: shorthand for zero runs)
//!   - With port:    [a:b::c]:port  bracketed form

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};

fn ipv6_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Bracketed form with port: [...]:port
        // Plain form: at least one `::` or 7+ colons (8 groups).
        // We use a permissive regex and validate in code.
        // Two alternations:
        //   - bracketed:   \[...\](:port)?
        //   - plain:       XXXX(:XXXX){2,7}  where each XXXX is 0-4 hex digits,
        //                  so the empty group inside `::` is allowed
        Regex::new(
            r"(?i)(\[[0-9a-f:]+\](?::\d{1,5})?)|([0-9a-f]{1,4}(?::[0-9a-f]{0,4}){2,7})",
        )
        .expect("ipv6 regex compiles")
    })
}

pub fn extract(text: &str) -> Vec<Match> {
    let re = ipv6_regex();
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in text.lines() {
        for m in re.find_iter(line) {
            let raw = m.as_str().to_string();
            let (ip, port) = parse_ipv6(&raw);
            if !valid_ipv6(&ip) {
                continue;
            }
            let span_start = byte_offset_of_line + m.start();
            let span_end = span_start + raw.len();
            let mut fields = HashMap::new();
            fields.insert("ip".to_string(), ip);
            fields.insert("port".to_string(), port);
            out.push(Match {
                ty: MatchType::Ipv6,
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

fn parse_ipv6(raw: &str) -> (String, String) {
    if let Some(stripped) = raw.strip_prefix('[') {
        if let Some(close) = stripped.find(']') {
            let ip = stripped[..close].to_string();
            let after = &stripped[close + 1..];
            let port = after.strip_prefix(':').unwrap_or("").to_string();
            return (ip, port);
        }
    }
    (raw.to_string(), String::new())
}

fn valid_ipv6(ip: &str) -> bool {
    // Reject too-short candidates the regex picks up (need ≥2 colons).
    let colon_count = ip.matches(':').count();
    if colon_count < 2 {
        return false;
    }
    // No more than one `::` allowed.
    if ip.matches("::").count() > 1 {
        return false;
    }
    // Each group must be ≤ 4 hex chars.
    for group in ip.split(':') {
        if group.len() > 4 {
            return false;
        }
        if !group.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    // With `::`, we tolerate fewer than 8 groups. Without, must be exactly 8.
    if !ip.contains("::") {
        let groups = ip.split(':').count();
        if groups != 8 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_form() {
        let m = extract("ping6 2001:0db8:0000:0000:0000:ff00:0042:8329 reachable");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["ip"], "2001:0db8:0000:0000:0000:ff00:0042:8329");
    }

    #[test]
    fn compressed_form() {
        let m = extract("at 2001:db8::1 connect");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["ip"], "2001:db8::1");
    }

    #[test]
    fn bracketed_with_port() {
        let m = extract("connect to [2001:db8::1]:8080 today");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].fields["ip"], "2001:db8::1");
        assert_eq!(m[0].fields["port"], "8080");
    }

    #[test]
    fn rejects_too_short() {
        let m = extract("not an ip: abcd:1");
        assert!(m.is_empty());
    }
}
