//! IPv4 with optional `:port` suffix. Each octet validated 0-255 in
//! post-regex code so the regex itself stays simple.
//!
//! Captures `{ip}` and `{port}` fields.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::extract::{Match, MatchType};

fn ipv4_regex() -> &'static Regex {
    static IPV4_RE: OnceLock<Regex> = OnceLock::new();
    IPV4_RE.get_or_init(|| {
        Regex::new(r"\b(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})(?::(\d{1,5}))?\b")
            .expect("ipv4 regex compiles")
    })
}

pub fn extract(text: &str) -> Vec<Match> {
    let re = ipv4_regex();
    let mut out = Vec::new();
    let mut byte_offset_of_line = 0usize;
    for line in text.lines() {
        for caps in re.captures_iter(line) {
            let m = caps.get(0).expect("full match present");
            let ip_str = caps.get(1).map(|c| c.as_str()).unwrap_or("");
            // Validate each octet 0-255.
            if !valid_ipv4(ip_str) {
                continue;
            }
            let port_str = caps.get(2).map(|c| c.as_str());
            if let Some(p) = port_str {
                if p.parse::<u32>().map(|n| n > 65535).unwrap_or(true) {
                    continue;
                }
            }
            let raw = m.as_str().to_string();
            let span_start = byte_offset_of_line + m.start();
            let span_end = span_start + raw.len();
            let mut fields = HashMap::new();
            fields.insert("ip".to_string(), ip_str.to_string());
            fields.insert(
                "port".to_string(),
                port_str.unwrap_or("").to_string(),
            );
            out.push(Match {
                ty: MatchType::Ipv4,
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

fn valid_ipv4(s: &str) -> bool {
    let octets: Vec<&str> = s.split('.').collect();
    if octets.len() != 4 {
        return false;
    }
    octets.iter().all(|o| {
        !o.is_empty()
            && o.len() <= 3
            && o.chars().all(|c| c.is_ascii_digit())
            && o.parse::<u32>().map(|n| n <= 255).unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ipv4() {
        let m = extract("connect to 192.168.1.1 now");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "192.168.1.1");
        assert_eq!(m[0].fields["ip"], "192.168.1.1");
        assert_eq!(m[0].fields["port"], "");
    }

    #[test]
    fn with_port() {
        let m = extract("redis at 10.0.0.5:6379 here");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "10.0.0.5:6379");
        assert_eq!(m[0].fields["ip"], "10.0.0.5");
        assert_eq!(m[0].fields["port"], "6379");
    }

    #[test]
    fn rejects_out_of_range_octet() {
        let m = extract("not 999.1.1.1 valid");
        assert!(m.is_empty());
    }

    #[test]
    fn rejects_out_of_range_port() {
        let m = extract("not 10.0.0.5:99999 valid");
        assert!(m.is_empty());
    }
}
