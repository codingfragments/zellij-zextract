//! Inline `#type` filter parsing for the picker query.
//!
//! The picker's query text mixes two concerns:
//!   - **Filter tokens** like `#url`, `#!secret`, `#ur` (prefix match),
//!     or `##main` (escape — literal `#main` text). They restrict
//!     which matches are visible.
//!   - **Fuzzy tokens** — everything else. Passed to nucleo for
//!     scoring against each remaining match.
//!
//! This module's `parse_query` is **pure**: it takes the raw query
//! text and a slice of *known tags* (strings) and returns a
//! `ParsedQuery` with the three buckets resolved. The known-tag set
//! is injected by the caller — derived from `extract::TYPE_PRIORITY`
//! in v1, and later (Phase 7+) optionally extended with user-defined
//! custom-pattern type names from KDL config. The parser itself has
//! no `MatchType` awareness; reordering or extending the type set
//! does not touch this code.
//!
//! Prefix matching: `#X` resolves to the type whose tag uniquely
//! starts with `X`. Ambiguous prefixes (multiple tags match) and
//! unknown prefixes (no tag matches) fall back to fuzzy treatment —
//! the original `#X` token becomes literal fuzzy text rather than
//! silently filtering to the wrong thing.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedQuery {
    /// Tag names to **include** (only matches with one of these types
    /// pass). Empty = no include constraint, all types pass.
    pub includes: Vec<String>,
    /// Tag names to **exclude** (matches with one of these types are
    /// dropped). Applied after includes.
    pub excludes: Vec<String>,
    /// The fuzzy-search text — non-filter tokens joined by spaces,
    /// passed to nucleo as the needle. Empty = passthrough (all
    /// post-filter matches kept).
    pub fuzzy: String,
}

/// Parse `text` into filter buckets + fuzzy text, resolving `#…`
/// tokens against `known_tags` by exact-or-unique-prefix match.
///
/// Token forms recognized:
///   - `#X`       include filter, prefix-resolved
///   - `#!X`      exclude filter, prefix-resolved
///   - `##X`      escape: emit literal `#X` as fuzzy text
///   - anything else → fuzzy text
///
/// Ambiguous prefix (multiple tags match) or unknown prefix (no tag
/// matches) → falls back to fuzzy (the literal `#…` text).
pub fn parse_query(text: &str, known_tags: &[&str]) -> ParsedQuery {
    let mut out = ParsedQuery::default();
    let mut fuzzy_parts: Vec<&str> = Vec::new();

    for token in text.split_whitespace() {
        // `##X` → literal `#X` as fuzzy
        if let Some(rest) = token.strip_prefix("##") {
            // Emit as `#X` — the user's escape intent.
            // To avoid an alloc we'd need to rebuild without `\n` etc.;
            // for now, allocate (rare path).
            out.fuzzy
                .push_str(if out.fuzzy.is_empty() { "" } else { " " });
            out.fuzzy.push('#');
            out.fuzzy.push_str(rest);
            continue;
        }
        // `#!X` → exclude
        if let Some(name) = token.strip_prefix("#!") {
            match resolve_tag(name, known_tags) {
                Some(tag) => out.excludes.push(tag.to_string()),
                None => fuzzy_parts.push(token), // ambiguous/unknown — literal
            }
            continue;
        }
        // `#X` → include
        if let Some(name) = token.strip_prefix('#') {
            if name.is_empty() {
                fuzzy_parts.push(token); // bare `#` — literal
                continue;
            }
            match resolve_tag(name, known_tags) {
                Some(tag) => out.includes.push(tag.to_string()),
                None => fuzzy_parts.push(token), // ambiguous/unknown — literal
            }
            continue;
        }
        // plain fuzzy token
        fuzzy_parts.push(token);
    }

    if !fuzzy_parts.is_empty() {
        if !out.fuzzy.is_empty() {
            out.fuzzy.push(' ');
        }
        out.fuzzy.push_str(&fuzzy_parts.join(" "));
    }
    out
}

/// Resolve a typed prefix against the known-tag set. Returns the tag
/// when exactly one tag starts with `prefix` (case-insensitive), or
/// when `prefix` is itself an exact tag match (handles the case where
/// one tag is a prefix of another — e.g. if there were both `s` and
/// `sha`, typing `s` should mean `s`, not error as "ambiguous").
fn resolve_tag<'a>(prefix: &str, known_tags: &'a [&'a str]) -> Option<&'a str> {
    // Exact match wins.
    if let Some(t) = known_tags.iter().find(|t| eq_ic(t, prefix)) {
        return Some(*t);
    }
    // Otherwise: unique prefix match.
    let mut candidates = known_tags.iter().filter(|t| starts_with_ic(t, prefix));
    let first = candidates.next()?;
    if candidates.next().is_some() {
        None // ambiguous — multiple tags share this prefix
    } else {
        Some(*first)
    }
}

fn eq_ic(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

fn starts_with_ic(haystack: &str, prefix: &str) -> bool {
    haystack.len() >= prefix.len()
        && haystack
            .bytes()
            .zip(prefix.bytes())
            .all(|(h, p)| h.eq_ignore_ascii_case(&p))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic tag set — mirrors v1 picker tags but doesn't depend on
    // MatchType, so the parser test stays independent of extract.rs.
    const V1_TAGS: &[&str] = &[
        "url", "file", "diag", "sha", "ipv4", "ipv6", "uuid", "quote", "cmd", "secret",
    ];

    fn parse(text: &str) -> ParsedQuery {
        parse_query(text, V1_TAGS)
    }

    // ---- exact tag matches ----

    #[test]
    fn include_exact_tag() {
        let p = parse("#url");
        assert_eq!(p.includes, vec!["url"]);
        assert!(p.excludes.is_empty());
        assert!(p.fuzzy.is_empty());
    }

    #[test]
    fn exclude_exact_tag() {
        let p = parse("#!secret config");
        assert_eq!(p.excludes, vec!["secret"]);
        assert!(p.includes.is_empty());
        assert_eq!(p.fuzzy, "config");
    }

    // ---- unique-prefix matches ----

    #[test]
    fn prefix_unique_resolves() {
        // `ur` uniquely identifies `url` (other u-tag is `uuid`)
        let p = parse("#ur install");
        assert_eq!(p.includes, vec!["url"]);
        assert_eq!(p.fuzzy, "install");
    }

    #[test]
    fn prefix_single_letter_unique() {
        // `f` uniquely identifies `file` — no other tag starts with f
        let p = parse("#f");
        assert_eq!(p.includes, vec!["file"]);
    }

    #[test]
    fn prefix_uu_unique_to_uuid() {
        let p = parse("#uu");
        assert_eq!(p.includes, vec!["uuid"]);
    }

    #[test]
    fn prefix_exclude_form_works() {
        // `#!se` should uniquely resolve to `secret` (the other s-tag is `sha`)
        let p = parse("#!se find me");
        assert_eq!(p.excludes, vec!["secret"]);
        assert_eq!(p.fuzzy, "find me");
    }

    // ---- ambiguous prefixes fall back to literal ----

    #[test]
    fn ambiguous_prefix_becomes_literal_fuzzy() {
        // `u` is shared by url and uuid → no filter, token treated as
        // literal fuzzy text
        let p = parse("#u install");
        assert!(p.includes.is_empty());
        assert!(p.excludes.is_empty());
        assert_eq!(p.fuzzy, "#u install");
    }

    #[test]
    fn ambiguous_s_prefix() {
        let p = parse("#s");
        assert!(p.includes.is_empty());
        assert_eq!(p.fuzzy, "#s");
    }

    #[test]
    fn ambiguous_i_prefix_for_ipv4_ipv6() {
        let p = parse("#i");
        assert!(p.includes.is_empty());
        assert_eq!(p.fuzzy, "#i");
    }

    #[test]
    fn ipv4_full_disambiguates() {
        let p = parse("#ipv4");
        assert_eq!(p.includes, vec!["ipv4"]);
    }

    // ---- unknown prefixes pass through as literal ----

    #[test]
    fn unknown_type_is_literal_fuzzy() {
        // Common case: CSS class name `#main-content`
        let p = parse("#main-content");
        assert!(p.includes.is_empty());
        assert_eq!(p.fuzzy, "#main-content");
    }

    // ---- escape syntax ----

    #[test]
    fn escape_double_hash() {
        let p = parse("##main install");
        assert!(p.includes.is_empty());
        assert_eq!(p.fuzzy, "#main install");
    }

    // ---- bare `#` and edge cases ----

    #[test]
    fn bare_hash_is_literal() {
        let p = parse("# alone");
        assert_eq!(p.fuzzy, "# alone");
    }

    #[test]
    fn empty_query() {
        let p = parse("");
        assert!(p.includes.is_empty());
        assert!(p.excludes.is_empty());
        assert!(p.fuzzy.is_empty());
    }

    // ---- multiple filters compose ----

    #[test]
    fn multiple_includes() {
        let p = parse("#url #file brew");
        assert_eq!(p.includes, vec!["url", "file"]);
        assert_eq!(p.fuzzy, "brew");
    }

    #[test]
    fn include_plus_exclude() {
        let p = parse("#file #!secret config");
        assert_eq!(p.includes, vec!["file"]);
        assert_eq!(p.excludes, vec!["secret"]);
        assert_eq!(p.fuzzy, "config");
    }

    #[test]
    fn token_order_independent() {
        let a = parse("#url install");
        let b = parse("install #url");
        assert_eq!(a.includes, b.includes);
        assert_eq!(a.fuzzy, b.fuzzy);
    }

    // ---- case-insensitive ----

    #[test]
    fn case_insensitive_prefix() {
        let p = parse("#URL install");
        assert_eq!(p.includes, vec!["url"]);
        let p2 = parse("#Ur install");
        assert_eq!(p2.includes, vec!["url"]);
    }

    // ---- extension-friendly: parser is parameterized on tag set ----

    #[test]
    fn caller_supplied_tag_set() {
        // Simulate v2+ with a user-defined `jira` type added to the set.
        let tags: &[&str] = &["url", "file", "jira"];
        let p = parse_query("#j PROJ-123", tags);
        assert_eq!(p.includes, vec!["jira"]);
        assert_eq!(p.fuzzy, "PROJ-123");
    }

    #[test]
    fn caller_supplied_tag_set_handles_prefix_collisions_with_custom_types() {
        // If the caller adds a custom `urgent` type, `#ur` becomes
        // ambiguous between `url` and `urgent` — parser must back off
        // to literal fuzzy without further config.
        let tags: &[&str] = &["url", "file", "urgent"];
        let p = parse_query("#ur install", tags);
        assert!(p.includes.is_empty(), "got {:?}", p.includes);
        assert_eq!(p.fuzzy, "#ur install");
    }
}
