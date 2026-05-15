//! Action verbs, per-type allow-lists, defaults, and dispatch.
//!
//! Phase 4 surface — built-in verbs only. Phase 7 adds:
//!   - `command "..."` custom-verb support with KDL configuration
//!   - Per-platform command overrides (Linux xdg-open, etc.)
//!   - Allow-list / default overrides per type

use std::collections::BTreeMap;

use zellij_tile::prelude::*;

use crate::extract::{Match, MatchType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    CopyRaw,
    CopyDisplay,
    Insert,
    InsertDisplay,
    Open,
    Edit,
    Reveal,
    Preview,
}

impl Verb {
    pub fn label(self) -> &'static str {
        match self {
            Verb::CopyRaw => "copy",
            Verb::CopyDisplay => "copy-display",
            Verb::Insert => "insert",
            Verb::InsertDisplay => "insert-display",
            Verb::Open => "open",
            Verb::Edit => "edit",
            Verb::Reveal => "reveal",
            Verb::Preview => "preview",
        }
    }

    /// The key letter (in List mode) that fires this verb. Returns None
    /// for verbs that don't have a dedicated key in v1.
    pub fn key_label(self) -> &'static str {
        match self {
            Verb::CopyRaw => "y",
            Verb::CopyDisplay => "Y",
            Verb::Insert => "i",
            Verb::InsertDisplay => "I",
            Verb::Open => "o",
            Verb::Edit => "e",
            Verb::Reveal => "r",
            Verb::Preview => "p",
        }
    }
}

/// Map a List-mode keystroke to a Verb, if any. Plain `c` (with no
/// modifiers) maps to CopyRaw, etc. Uppercase variants (Shift) map to
/// the *-display variants where applicable.
pub fn verb_from_char(c: char) -> Option<Verb> {
    match c {
        'y' => Some(Verb::CopyRaw),
        'Y' => Some(Verb::CopyDisplay),
        'i' => Some(Verb::Insert),
        'I' => Some(Verb::InsertDisplay),
        'o' => Some(Verb::Open),
        'e' => Some(Verb::Edit),
        'r' => Some(Verb::Reveal),
        'p' => Some(Verb::Preview),
        _ => None,
    }
}

/// Static type-keyed allow-list. Per-match conditions (e.g. file
/// existence) are layered on top by `allowed_verbs`.
fn static_allowed_verbs(ty: MatchType) -> &'static [Verb] {
    use MatchType::*;
    use Verb::*;
    match ty {
        Url => &[Open, CopyRaw, Insert],
        // Open/reveal removed for file/diag — without a reliable
        // existence check (see pattern/mod.rs comment) those actions
        // either work silently or fail at run_command time; keeping
        // them in the menu just creates fragile UX. Edit covers the
        // "do something with the file" case via $EDITOR.
        File | Diagnostic => &[Edit, CopyRaw, Insert],
        Sha => &[CopyRaw, Insert],
        Ipv4 | Ipv6 => &[CopyRaw, Insert],
        Uuid => &[CopyRaw, Insert],
        // QuotedString is the one type where raw != display in v1:
        // raw includes the surrounding quotes, display is the unquoted
        // content. Y/I make sense here; for other types they'd duplicate
        // y/i. Phase 8 expands these to url/file when truncation lands.
        QuotedString => &[CopyRaw, CopyDisplay, Insert, InsertDisplay],
        // Command: insert (paste back to prompt for review) is the
        // default; copy always allowed.
        Command => &[Insert, CopyRaw],
        // Secret: NEVER `open` / `edit` / `reveal` — hardcoded deny.
        // Even if Phase 7 config asks for it, the deny in is_verb_allowed
        // refuses. Only copy + insert.
        Secret => &[CopyRaw, Insert],
    }
}

/// Per-match allow-list. Currently a thin wrapper over the static
/// type-keyed list. The signature stays Match-aware so Phase 7 (KDL
/// config) can add per-match conditions (e.g. user-defined denies)
/// without another API change.
pub fn allowed_verbs(m: &Match) -> Vec<Verb> {
    static_allowed_verbs(m.ty).to_vec()
}

/// Default Verb fired by Enter on a given match.
///   - URL → Open (browser)
///   - Diagnostic → Edit (always carries a usable {line}; jumping straight
///     to it is the only thing that distinguishes diag from file)
///   - Everything else → Insert (captured text lands at the source pane's
///     prompt where the user can review and hit Enter)
pub fn default_verb(m: &Match) -> Verb {
    use MatchType::*;
    use Verb::*;
    match m.ty {
        Url => Open,
        Diagnostic => Edit,
        File | Command | Sha | Ipv4 | Ipv6 | Uuid | QuotedString | Secret => Insert,
    }
}

/// True if the verb may fire for the given match. CopyRaw is always
/// allowed (per planning.md Q8). Secrets hardcoded-deny Open/Edit/Reveal.
pub fn is_verb_allowed(m: &Match, verb: Verb) -> bool {
    if matches!(verb, Verb::CopyRaw) {
        return true;
    }
    if matches!(m.ty, MatchType::Secret)
        && matches!(verb, Verb::Open | Verb::Edit | Verb::Reveal)
    {
        return false;
    }
    allowed_verbs(m).contains(&verb)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchResult {
    Closed,
    StayOpen,
    Rejected,
}

pub fn dispatch(verb: Verb, m: &Match, source_pane: Option<u32>) -> DispatchResult {
    if !is_verb_allowed(m, verb) {
        return DispatchResult::Rejected;
    }
    match verb {
        Verb::CopyRaw => {
            copy_to_clipboard(&m.raw);
            DispatchResult::Closed
        }
        Verb::CopyDisplay => {
            copy_to_clipboard(&m.display);
            DispatchResult::Closed
        }
        Verb::Insert => insert_text(&m.raw, source_pane),
        Verb::InsertDisplay => insert_text(&m.display, source_pane),
        Verb::Open => run_open(m),
        Verb::Edit => run_edit(m, source_pane),
        Verb::Reveal => run_reveal(m),
        Verb::Preview => DispatchResult::StayOpen, // Phase 8 wires real preview
    }
}

fn insert_text(text: &str, source_pane: Option<u32>) -> DispatchResult {
    let Some(pane_id) = source_pane else {
        return DispatchResult::Rejected;
    };
    write_chars_to_pane_id(text, PaneId::Terminal(pane_id));
    DispatchResult::Closed
}

fn run_open(m: &Match) -> DispatchResult {
    // Phase 4 hardcodes the macOS `open` command. Phase 7 makes this
    // configurable per-platform via KDL; defaults branch by OS.
    let url_target;
    let target: &str = match m.ty {
        MatchType::Url => &m.raw,
        MatchType::File | MatchType::Diagnostic => m
            .fields
            .get("file")
            .map(|s| s.as_str())
            .unwrap_or(&m.raw),
        _ => {
            url_target = m.raw.clone();
            &url_target
        }
    };
    run_command(&["open", target], BTreeMap::new());
    DispatchResult::Closed
}

/// Edit verb implementation.
///
/// Builds the shell-command string and **inserts it into the source
/// pane** via `write_chars_to_pane_id`. The user reviews at their
/// prompt and hits Enter (or edits / discards) themselves. Running
/// the editor as a background subprocess via `run_command` would
/// silently detach (was the earlier "nothing happens" symptom).
///
/// Template (Phase 4 hardcoded; Phase 7 KDL config will expose
/// `editor_command_prefix` and a per-type override):
///   - With line: `<editor> +<line> <quoted-path>`
///   - Without line: `<editor> <quoted-path>`
///
/// The `+<line>` form is what nvim / vim / less / many editors accept.
/// VSCode-style users will override the template in Phase 7 to
/// something like `code -g <path>:<line>`.
///
/// Editor resolution order:
///   1. `$EDITOR` env var, if set
///   2. `nvim` as a reasonable fallback
///
/// Path quoting: applied only when the path contains characters that
/// would be re-interpreted by the shell (spaces, `$`, backticks, etc.).
/// Single-quoted with embedded `'` escaped as `'\''` — standard POSIX.
fn run_edit(m: &Match, source_pane: Option<u32>) -> DispatchResult {
    let Some(pane_id) = source_pane else {
        return DispatchResult::Rejected;
    };
    let file = m.fields.get("file").map(|s| s.as_str()).unwrap_or(&m.raw);
    let line = m.fields.get("line").map(|s| s.as_str()).unwrap_or("");
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string());
    let cmd = if line.is_empty() {
        format!("{} {}", editor, shell_quote(file))
    } else {
        // Line is always digit-only (regex constraint) — no shell quoting needed.
        format!("{} +{} {}", editor, line, shell_quote(file))
    };
    write_chars_to_pane_id(&cmd, PaneId::Terminal(pane_id));
    DispatchResult::Closed
}

/// POSIX-safe shell quoting. If `s` contains only chars that the shell
/// won't reinterpret, returns it as-is; otherwise wraps it in single
/// quotes (with embedded `'` escaped as `'\''`).
pub(crate) fn shell_quote(s: &str) -> String {
    let is_safe = !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | '~' | ':' | '+' | ',' | '@' | '=')
        });
    if is_safe {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn run_reveal(m: &Match) -> DispatchResult {
    let file = m.fields.get("file").map(|s| s.as_str()).unwrap_or(&m.raw);
    run_command(&["open", "-R", file], BTreeMap::new());
    DispatchResult::Closed
}

/// Substitute `{name}` placeholders in a template with values from a
/// match. Universal vars resolve to fields on the Match struct itself;
/// per-type vars come from `m.fields`. Unknown names substitute the
/// empty string (per planning.md Q20). Built now even though Phase 4
/// has no custom-verb consumers — Phase 7's KDL `command "..."` will
/// call this.
#[allow(dead_code)]
pub fn substitute(template: &str, m: &Match) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        // `{{` escapes a literal `{`.
        if chars.peek() == Some(&'{') {
            chars.next();
            out.push('{');
            continue;
        }
        let mut name = String::new();
        let mut closed = false;
        for nc in chars.by_ref() {
            if nc == '}' {
                closed = true;
                break;
            }
            name.push(nc);
        }
        if !closed {
            // Unclosed `{...` — emit literally as a safety fallback.
            out.push('{');
            out.push_str(&name);
            continue;
        }
        let value = match name.as_str() {
            "match" | "raw" => m.raw.clone(),
            "display" => m.display.clone(),
            "type" => m.ty.tag().to_string(),
            "context" => m.context.clone(),
            _ => m.fields.get(&name).cloned().unwrap_or_default(),
        };
        out.push_str(&value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn url_match() -> Match {
        let mut fields = HashMap::new();
        fields.insert("url".to_string(), "https://example.com".to_string());
        fields.insert("scheme".to_string(), "https".to_string());
        fields.insert("host".to_string(), "example.com".to_string());
        Match {
            ty: MatchType::Url,
            raw: "https://example.com".to_string(),
            display: "https://example.com".to_string(),
            context: "see https://example.com here".to_string(),
            span: (4, 23),
            fields,
        }
    }

    fn file_match() -> Match {
        let mut fields = HashMap::new();
        fields.insert("file".to_string(), "src/main.rs".to_string());
        fields.insert("line".to_string(), "42".to_string());
        fields.insert("col".to_string(), "8".to_string());
        Match {
            ty: MatchType::File,
            raw: "src/main.rs:42:8".to_string(),
            display: "src/main.rs:42:8".to_string(),
            context: "error at src/main.rs:42:8".to_string(),
            span: (9, 25),
            fields,
        }
    }

    fn secret_match() -> Match {
        let mut fields = HashMap::new();
        fields.insert("secret".to_string(), "ghp_abc".to_string());
        fields.insert("secret_format".to_string(), "github".to_string());
        Match {
            ty: MatchType::Secret,
            raw: "ghp_abc".to_string(),
            display: "ghp_abc".to_string(),
            context: String::new(),
            span: (0, 7),
            fields,
        }
    }

    fn empty_match(ty: MatchType) -> Match {
        Match {
            ty,
            raw: String::new(),
            display: String::new(),
            context: String::new(),
            span: (0, 0),
            fields: HashMap::new(),
        }
    }

    #[test]
    fn default_verbs_per_type() {
        assert_eq!(default_verb(&empty_match(MatchType::Url)), Verb::Open);
        // Diagnostic jumps straight to its captured line — that's the
        // thing that distinguishes it from a plain file path.
        assert_eq!(default_verb(&empty_match(MatchType::Diagnostic)), Verb::Edit);
        // Everything else: insert at the prompt.
        for ty in [
            MatchType::File, MatchType::Command,
            MatchType::Sha, MatchType::Ipv4, MatchType::Ipv6, MatchType::Uuid,
            MatchType::QuotedString, MatchType::Secret,
        ] {
            assert_eq!(default_verb(&empty_match(ty)), Verb::Insert, "{ty:?}");
        }
    }

    #[test]
    fn copy_always_allowed() {
        for ty in [
            MatchType::Url, MatchType::File, MatchType::Diagnostic,
            MatchType::Sha, MatchType::Ipv4, MatchType::Ipv6,
            MatchType::Uuid, MatchType::QuotedString, MatchType::Command,
            MatchType::Secret,
        ] {
            assert!(is_verb_allowed(&empty_match(ty), Verb::CopyRaw));
        }
    }

    #[test]
    fn secret_denies_open_edit_reveal() {
        let m = empty_match(MatchType::Secret);
        assert!(!is_verb_allowed(&m, Verb::Open));
        assert!(!is_verb_allowed(&m, Verb::Edit));
        assert!(!is_verb_allowed(&m, Verb::Reveal));
    }

    #[test]
    fn file_allow_set_is_edit_copy_insert() {
        let m = empty_match(MatchType::File);
        for v in [Verb::Edit, Verb::CopyRaw, Verb::Insert] {
            assert!(is_verb_allowed(&m, v), "file must allow {v:?}");
        }
        // Open and Reveal removed in this simplification.
        assert!(!is_verb_allowed(&m, Verb::Open));
        assert!(!is_verb_allowed(&m, Verb::Reveal));
    }

    #[test]
    fn diagnostic_allow_set_is_edit_copy_insert() {
        let m = empty_match(MatchType::Diagnostic);
        for v in [Verb::Edit, Verb::CopyRaw, Verb::Insert] {
            assert!(is_verb_allowed(&m, v), "diag must allow {v:?}");
        }
        assert!(!is_verb_allowed(&m, Verb::Open));
        assert!(!is_verb_allowed(&m, Verb::Reveal));
    }

    #[test]
    fn url_does_not_allow_reveal() {
        assert!(!is_verb_allowed(&empty_match(MatchType::Url), Verb::Reveal));
    }

    #[test]
    fn display_variants_only_on_quoted_string_for_now() {
        assert!(is_verb_allowed(&empty_match(MatchType::QuotedString), Verb::CopyDisplay));
        assert!(is_verb_allowed(&empty_match(MatchType::QuotedString), Verb::InsertDisplay));
        for ty in [
            MatchType::Url, MatchType::Sha, MatchType::Ipv4, MatchType::Ipv6,
            MatchType::Uuid, MatchType::Command, MatchType::Secret,
        ] {
            assert!(!is_verb_allowed(&empty_match(ty), Verb::CopyDisplay), "{ty:?}");
            assert!(!is_verb_allowed(&empty_match(ty), Verb::InsertDisplay), "{ty:?}");
        }
    }

    #[test]
    fn verb_from_char_basic() {
        assert_eq!(verb_from_char('y'), Some(Verb::CopyRaw));
        assert_eq!(verb_from_char('Y'), Some(Verb::CopyDisplay));
        assert_eq!(verb_from_char('o'), Some(Verb::Open));
        assert_eq!(verb_from_char('e'), Some(Verb::Edit));
        assert_eq!(verb_from_char('i'), Some(Verb::Insert));
        assert_eq!(verb_from_char('I'), Some(Verb::InsertDisplay));
        assert_eq!(verb_from_char('z'), None);
    }

    #[test]
    fn substitute_universal_vars() {
        let m = url_match();
        assert_eq!(substitute("{type}: {match}", &m), "url: https://example.com");
        assert_eq!(substitute("{display}", &m), "https://example.com");
    }

    #[test]
    fn substitute_type_specific_vars() {
        let m = file_match();
        assert_eq!(
            substitute("$EDITOR {file} +{line}", &m),
            "$EDITOR src/main.rs +42"
        );
    }

    #[test]
    fn substitute_unknown_field_is_empty() {
        let m = url_match();
        assert_eq!(substitute("[{nonexistent}]", &m), "[]");
    }

    #[test]
    fn substitute_escapes_double_brace() {
        // Only `{{` → `{` is escaped; `}` outside a placeholder is literal.
        // Phase 7 refines when KDL templates are user-facing.
        let m = url_match();
        assert_eq!(substitute("{{literal} {url}", &m), "{literal} https://example.com");
    }

    #[test]
    fn substitute_unclosed_placeholder_is_literal() {
        let m = url_match();
        assert_eq!(substitute("see {url and more", &m), "see {url and more");
    }

    #[test]
    fn shell_quote_passes_safe_strings_through() {
        assert_eq!(shell_quote("src/main.rs"), "src/main.rs");
        assert_eq!(shell_quote("/etc/hosts"), "/etc/hosts");
        assert_eq!(shell_quote("~/dotfiles/config.kdl"), "~/dotfiles/config.kdl");
        assert_eq!(shell_quote("a.b.c-1_2"), "a.b.c-1_2");
    }

    #[test]
    fn shell_quote_wraps_strings_with_spaces() {
        assert_eq!(shell_quote("file with space.txt"), "'file with space.txt'");
    }

    #[test]
    fn shell_quote_wraps_shell_metachars() {
        assert_eq!(shell_quote("$VAR"), "'$VAR'");
        assert_eq!(shell_quote("`cmd`"), "'`cmd`'");
        assert_eq!(shell_quote("a;b"), "'a;b'");
        assert_eq!(shell_quote("a|b"), "'a|b'");
        assert_eq!(shell_quote("a&b"), "'a&b'");
        assert_eq!(shell_quote("a*b"), "'a*b'");
        assert_eq!(shell_quote("a?b"), "'a?b'");
        assert_eq!(shell_quote("a(b"), "'a(b'");
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quote() {
        // POSIX trick: end the quoted run, emit an escaped quote, restart.
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn shell_quote_handles_empty() {
        // Empty strings are not "safe" — they need quotes so the shell
        // sees an empty arg rather than nothing.
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn secret_match_struct_does_not_leak_open() {
        // Spot-check: the example secret match is built; if anyone wires
        // is_verb_allowed wrong, this test will catch it via the deny
        // check above. Sanity-check the example match here too.
        let m = secret_match();
        assert_eq!(m.fields["secret_format"], "github");
    }
}
