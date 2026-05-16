//! Fuzzy matching wrapper over `nucleo-matcher`. Phase 2 surface.
//!
//! Smart-case: query containing any uppercase char → case-sensitive match.
//! Otherwise case-insensitive (nucleo's `Config::ignore_case = true`).
//! Empty query returns all items in their input order with empty indices.
//! Non-empty query returns items sorted by descending score.

use nucleo_matcher::{Config, Matcher, Utf32Str};

pub struct FuzzyEngine {
    matcher: Matcher,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoredMatch {
    /// Index into the input `items` slice.
    pub index: usize,
    /// Fuzzy score (higher = better) plus any per-item bonus. 0 for
    /// empty-query passthrough. i32 so bonuses can be negative.
    pub score: i32,
    /// Character positions in the item that matched. Empty when query is empty.
    pub indices: Vec<u32>,
}

impl Default for FuzzyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FuzzyEngine {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    #[allow(dead_code)] // bonus-less convenience for tests; prod path is filter_with_bonus
    pub fn filter(&mut self, query: &str, items: &[&str]) -> Vec<ScoredMatch> {
        self.filter_with_bonus(query, items, |_| 0)
    }

    /// Variant of `filter` that adds a per-item bonus to the nucleo score.
    /// Used by zextract to bias type-priority (diagnostic ranks higher
    /// than sha when fuzzy scores are close).
    pub fn filter_with_bonus<F: Fn(usize) -> i32>(
        &mut self,
        query: &str,
        items: &[&str],
        bonus_fn: F,
    ) -> Vec<ScoredMatch> {
        if query.is_empty() {
            return (0..items.len())
                .map(|i| ScoredMatch {
                    index: i,
                    score: 0,
                    indices: Vec::new(),
                })
                .collect();
        }

        // Smart-case: any uppercase in query → respect case.
        self.matcher.config.ignore_case = !query.chars().any(|c| c.is_ascii_uppercase());

        // Build needle once. Use Utf32Str variants directly so we don't
        // entangle borrows with `self.matcher` inside the loop.
        let needle_chars: Vec<char>;
        let needle = if query.is_ascii() {
            Utf32Str::Ascii(query.as_bytes())
        } else {
            needle_chars = query.chars().collect();
            Utf32Str::Unicode(&needle_chars)
        };

        let mut results: Vec<ScoredMatch> = Vec::with_capacity(items.len());
        let mut haystack_chars: Vec<char> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        for (i, item) in items.iter().enumerate() {
            let haystack = if item.is_ascii() {
                Utf32Str::Ascii(item.as_bytes())
            } else {
                haystack_chars.clear();
                haystack_chars.extend(item.chars());
                Utf32Str::Unicode(&haystack_chars)
            };
            indices.clear();
            if let Some(score) = self.matcher.fuzzy_indices(haystack, needle, &mut indices) {
                results.push(ScoredMatch {
                    index: i,
                    score: score as i32 + bonus_fn(i),
                    indices: indices.clone(),
                });
            }
        }

        results.sort_unstable_by(|a, b| b.score.cmp(&a.score));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_all_in_order() {
        let mut fz = FuzzyEngine::new();
        let items = ["alpha", "beta", "gamma"];
        let result = fz.filter("", &items);
        assert_eq!(result.len(), 3);
        assert_eq!(
            result.iter().map(|r| r.index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(result.iter().all(|r| r.indices.is_empty()));
    }

    #[test]
    fn narrows_to_matching_items() {
        let mut fz = FuzzyEngine::new();
        let items = [
            "https://example.com",
            "ftp://archive.org",
            "https://docs.rs",
        ];
        let result = fz.filter("docs", &items);
        let indices: Vec<usize> = result.iter().map(|r| r.index).collect();
        assert!(indices.contains(&2));
        // Items without "docs" subsequence should not be present
        let raws: Vec<&str> = indices.iter().map(|&i| items[i]).collect();
        for raw in &raws {
            assert!(raw.contains("docs") || subseq("docs", raw));
        }
    }

    fn subseq(needle: &str, haystack: &str) -> bool {
        let mut hi = haystack.chars();
        for nc in needle.chars() {
            let nc = nc.to_ascii_lowercase();
            loop {
                match hi.next() {
                    Some(c) if c.to_ascii_lowercase() == nc => break,
                    Some(_) => continue,
                    None => return false,
                }
            }
        }
        true
    }

    #[test]
    fn smart_case_lowercase_query_is_insensitive() {
        let mut fz = FuzzyEngine::new();
        let items = ["EXAMPLE.com", "example.com"];
        let result = fz.filter("example", &items);
        // Both should match
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn smart_case_uppercase_query_is_sensitive() {
        let mut fz = FuzzyEngine::new();
        let items = ["EXAMPLE.com", "example.com"];
        let result = fz.filter("EXAMPLE", &items);
        // Only the uppercase one should match
        let matched: Vec<usize> = result.iter().map(|r| r.index).collect();
        assert!(matched.contains(&0));
        assert!(!matched.contains(&1));
    }

    #[test]
    fn returns_indices_for_matched_chars() {
        let mut fz = FuzzyEngine::new();
        let items = ["docs.rs"];
        let result = fz.filter("dr", &items);
        assert_eq!(result.len(), 1);
        // "dr" should match positions of 'd' (0) and 'r' (5) in "docs.rs"
        let indices = &result[0].indices;
        assert!(!indices.is_empty());
        // The first matched char must be 'd' at position 0.
        assert_eq!(indices[0], 0);
    }

    #[test]
    fn sorts_by_descending_score() {
        let mut fz = FuzzyEngine::new();
        let items = ["foo-bar", "foobar", "f-o-o-b-a-r"];
        let result = fz.filter("foobar", &items);
        // Contiguous "foobar" should rank above the dashed variants.
        assert_eq!(items[result[0].index], "foobar");
    }
}
