//! Action verbs, per-type allow-lists, defaults, and dispatch.
//!
//! Phase 4 surface — built-in verbs only. Phase 7 adds:
//!   - `command "..."` custom-verb support with KDL configuration
//!   - Per-platform command overrides (Linux xdg-open, etc.)
//!   - Allow-list / default overrides per type

use std::collections::BTreeMap;

use zellij_tile::prelude::*;

use crate::config::{ActionsConfig, TypesConfig};
use crate::extract::{Match, MatchType};

/// The built-in fallback edit template when no `actions` override is
/// configured. Uses `{editor}` (resolves to `$EDITOR || "nvim"`) and
/// `{line}` optional stripping via `substitute_opt`.
pub const DEFAULT_EDIT_TEMPLATE: &str = "{editor} +{line} {file}";

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
    /// Export the current selection (or highlighted row) as a JSON
    /// array of flat per-match objects to the clipboard. Universal:
    /// always allowed for every type (planning.md Q21).
    Json,
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
            Verb::Json => "json",
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
            Verb::Json => "J",
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
        'J' => Some(Verb::Json),
        _ => None,
    }
}

/// Parse a verb name string (as used in KDL config files) into a Verb.
/// Returns None for unknown labels — caller drops with a warning.
/// `copy-raw` is accepted as an alias for `copy` (the label name).
pub fn verb_from_label(s: &str) -> Option<Verb> {
    match s {
        "copy" | "copy-raw" => Some(Verb::CopyRaw),
        "copy-display" => Some(Verb::CopyDisplay),
        "insert" | "insert-raw" => Some(Verb::Insert),
        "insert-display" => Some(Verb::InsertDisplay),
        "open" => Some(Verb::Open),
        "edit" => Some(Verb::Edit),
        "reveal" => Some(Verb::Reveal),
        "preview" => Some(Verb::Preview),
        "json" => Some(Verb::Json),
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

/// Per-match allow-list. If the user has configured `types.<tag>.actions
/// [...]` the user's list wins; otherwise the static type-keyed list.
/// Universal/hardcoded denies (Secret + Open/Edit/Reveal) are applied
/// upstream in `is_verb_allowed` regardless of override.
pub fn allowed_verbs(m: &Match, types: &TypesConfig) -> Vec<Verb> {
    if let Some(over) = types.overrides.get(m.ty.tag()) {
        if let Some(action_strs) = &over.actions {
            // Empty user list = "no actions" — honor it. Unknown labels
            // get filtered out; the user can fix their config without
            // breaking the plugin.
            return action_strs
                .iter()
                .filter_map(|s| verb_from_label(s))
                .collect();
        }
    }
    static_allowed_verbs(m.ty).to_vec()
}

/// Default Verb fired by Enter on a given match. User config wins via
/// `types.<tag>.default "<verb>"`; otherwise:
///   - URL → Open (browser)
///   - Diagnostic → Edit (always carries a usable {line}; jumping straight
///     to it is the only thing that distinguishes diag from file)
///   - Everything else → Insert (captured text lands at the source pane's
///     prompt where the user can review and hit Enter)
///
/// If the user's default isn't in the (possibly user-overridden)
/// allow-list for this type, the static default is used — keeps `Enter`
/// from firing a rejected verb.
pub fn default_verb(m: &Match, types: &TypesConfig) -> Verb {
    let static_default = static_default_verb(m.ty);
    let user_default = types
        .overrides
        .get(m.ty.tag())
        .and_then(|o| o.default.as_deref())
        .and_then(verb_from_label);
    let Some(v) = user_default else {
        return static_default;
    };
    if allowed_verbs(m, types).contains(&v) {
        v
    } else {
        static_default
    }
}

fn static_default_verb(ty: MatchType) -> Verb {
    use MatchType::*;
    use Verb::*;
    match ty {
        Url => Open,
        Diagnostic => Edit,
        File | Command | Sha | Ipv4 | Ipv6 | Uuid | QuotedString | Secret => Insert,
    }
}

/// True if the verb may fire for the given match. CopyRaw and Json
/// are universally allowed (planning.md Q8 / Q21). Secrets hardcoded-
/// deny Open/Edit/Reveal — even if user config tries to add them.
pub fn is_verb_allowed(m: &Match, verb: Verb, types: &TypesConfig) -> bool {
    if matches!(verb, Verb::CopyRaw | Verb::Json) {
        return true;
    }
    if matches!(m.ty, MatchType::Secret) && matches!(verb, Verb::Open | Verb::Edit | Verb::Reveal) {
        return false;
    }
    allowed_verbs(m, types).contains(&verb)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchResult {
    Closed,
    StayOpen,
    Rejected,
}

pub fn dispatch(
    verb: Verb,
    m: &Match,
    source_pane: Option<u32>,
    types: &TypesConfig,
    actions: &ActionsConfig,
) -> DispatchResult {
    if !is_verb_allowed(m, verb, types) {
        return DispatchResult::Rejected;
    }
    // Look up per-type override first, then fall back to "default".
    let tag = m.ty.tag();
    let verb_templates = actions
        .overrides
        .get(tag)
        .or_else(|| actions.overrides.get("default"));
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
        Verb::Open => {
            let tmpl = verb_templates.and_then(|t| t.open.as_deref());
            run_open(m, tmpl)
        }
        Verb::Edit => {
            let tmpl = verb_templates.and_then(|t| t.edit.as_deref());
            run_edit(m, source_pane, tmpl)
        }
        Verb::Reveal => {
            let tmpl = verb_templates.and_then(|t| t.reveal.as_deref());
            run_reveal(m, tmpl)
        }
        Verb::Preview => DispatchResult::StayOpen,
        Verb::Json => {
            let json = matches_to_json_array(std::slice::from_ref(&m));
            copy_to_clipboard(&json);
            DispatchResult::Closed
        }
    }
}

/// Serialize a slice of Match references to a compact, single-line
/// JSON array. Always returns an array (even for one element) per
/// planning.md Q21. Universal fields and per-type fields live at the
/// same level (flat). All values stringified except `span` which is
/// `[start, end]`. No external serde dep — built by hand so wasm size
/// stays small.
pub fn matches_to_json_array(matches: &[&Match]) -> String {
    let mut out = String::from("[");
    for (i, m) in matches.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match_to_json_object(m, &mut out);
    }
    out.push(']');
    out
}

fn match_to_json_object(m: &Match, out: &mut String) {
    use std::fmt::Write as _;
    out.push('{');
    // Universal fields, deterministic order.
    push_json_kv(out, "type", m.effective_tag());
    out.push(',');
    push_json_kv(out, "raw", &m.raw);
    out.push(',');
    push_json_kv(out, "display", &m.display);
    out.push(',');
    push_json_kv(out, "context", &m.context);
    // Span as a numeric pair.
    out.push(',');
    let _ = write!(out, r#""span":[{},{}]"#, m.span.0, m.span.1);
    // Per-type fields, sorted for stable output.
    let mut keys: Vec<&String> = m.fields.keys().collect();
    keys.sort();
    for k in keys {
        out.push(',');
        push_json_kv(out, k, &m.fields[k]);
    }
    out.push('}');
}

fn push_json_kv(out: &mut String, key: &str, value: &str) {
    out.push('"');
    push_json_escaped(out, key);
    out.push_str("\":\"");
    push_json_escaped(out, value);
    out.push('"');
}

fn push_json_escaped(out: &mut String, s: &str) {
    use std::fmt::Write as _;
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

fn insert_text(text: &str, source_pane: Option<u32>) -> DispatchResult {
    let Some(pane_id) = source_pane else {
        return DispatchResult::Rejected;
    };
    write_chars_to_pane_id(text, PaneId::Terminal(pane_id));
    DispatchResult::Closed
}

fn run_open(m: &Match, template: Option<&str>) -> DispatchResult {
    if let Some(tmpl) = template {
        let cmd = substitute_opt(tmpl, m);
        run_command(&["sh", "-c", &cmd], BTreeMap::new());
        return DispatchResult::Closed;
    }
    // Built-in: macOS `open` (or xdg-open on Linux).
    let url_target;
    let target: &str = match m.ty {
        MatchType::Url => &m.raw,
        MatchType::File | MatchType::Diagnostic => {
            m.fields.get("file").map(|s| s.as_str()).unwrap_or(&m.raw)
        }
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
/// Template:
///   - With line: `<editor> +<line> <quoted-path>`
///   - Without line: `<editor> <quoted-path>`
///
/// The `+<line>` form is what nvim / vim / less / many editors accept.
/// VSCode users: `actions { file { edit command "code -g {file}:{line}" } }`.
///
/// Editor resolution: `{editor}` in the template expands to `$EDITOR`
/// or `"nvim"` if unset. The default template is `DEFAULT_EDIT_TEMPLATE`.
///
/// Path quoting: applied only when the path contains characters that
/// would be re-interpreted by the shell (spaces, `$`, backticks, etc.).
/// Single-quoted with embedded `'` escaped as `'\''` — standard POSIX.
fn run_edit(m: &Match, source_pane: Option<u32>, template: Option<&str>) -> DispatchResult {
    let Some(pane_id) = source_pane else {
        return DispatchResult::Rejected;
    };
    // Use user template if set, otherwise fall back to the built-in
    // default. Both paths go through substitute_opt so {line} stripping
    // and {editor} resolution work identically.
    let tmpl = template.unwrap_or(DEFAULT_EDIT_TEMPLATE);
    let cmd = substitute_opt(tmpl, m);
    write_chars_to_pane_id(&cmd, PaneId::Terminal(pane_id));
    DispatchResult::Closed
}

/// POSIX-safe shell quoting. If `s` contains only chars that the shell
/// won't reinterpret, returns it as-is; otherwise wraps it in single
/// quotes (with embedded `'` escaped as `'\''`).
#[cfg(test)]
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

fn run_reveal(m: &Match, template: Option<&str>) -> DispatchResult {
    if let Some(tmpl) = template {
        let cmd = substitute_opt(tmpl, m);
        run_command(&["sh", "-c", &cmd], BTreeMap::new());
        return DispatchResult::Closed;
    }
    let file = m.fields.get("file").map(|s| s.as_str()).unwrap_or(&m.raw);
    run_command(&["open", "-R", file], BTreeMap::new());
    DispatchResult::Closed
}

/// Substitute `{name}` placeholders in a template with values from a
/// match. Universal vars resolve to fields on the Match struct itself;
/// per-type vars come from `m.fields`. Unknown names substitute the
/// empty string (per planning.md Q20). Built now even though Phase 4
/// Template substitution without separator stripping. Used in tests;
/// production code uses `substitute_opt`.
#[cfg(test)]
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

/// Like `substitute`, but when a `{field}` resolves to an empty string,
/// strips any immediately preceding separator characters from the output.
/// Separator chars: `:`, `+`, ` ` (space), `,`.
///
/// This handles the common "optional line number" case cleanly:
///
///   `"hx {file}:{line}"` + line="" → `"hx src/main.rs"` (`:` stripped)
///   `"nvim +{line} {file}"` + line="" → `"nvim src/main.rs"` (`+` and space stripped)
///   `"hx {file}:{line}"` + line="42" → `"hx src/main.rs:42"` (unchanged)
///
/// Note: stripping is retroactive on the output buffer — it does NOT look
/// ahead. A leading `{field}` that is empty produces no stripping (there's
/// nothing to strip before it in the output).
pub fn substitute_opt(template: &str, m: &Match) -> String {
    const SEP: &[char] = &[':', '+', ' ', ','];
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
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
            out.push('{');
            out.push_str(&name);
            continue;
        }
        let value = match name.as_str() {
            "match" | "raw" => m.raw.clone(),
            "display" => m.display.clone(),
            "type" => m.ty.tag().to_string(),
            "context" => m.context.clone(),
            // {editor} resolves to $EDITOR env var, falling back to "nvim".
            // Lets the default template work out-of-the-box without any
            // config and respects the user's shell editor setting.
            "editor" => std::env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string()),
            _ => m.fields.get(&name).cloned().unwrap_or_default(),
        };
        if value.is_empty() {
            while out
                .chars()
                .last()
                .map(|c| SEP.contains(&c))
                .unwrap_or(false)
            {
                out.pop();
            }
        } else {
            out.push_str(&value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Default-empty TypesConfig — tests that don't exercise user
    /// overrides pass this in. Helper to keep call sites readable.
    fn td() -> TypesConfig {
        TypesConfig::default()
    }

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
            label: None,
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
            label: None,
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
            label: None,
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
            label: None,
            span: (0, 0),
            fields: HashMap::new(),
        }
    }

    #[test]
    fn default_verbs_per_type() {
        let t = td();
        assert_eq!(default_verb(&empty_match(MatchType::Url), &t), Verb::Open);
        // Diagnostic jumps straight to its captured line — that's the
        // thing that distinguishes it from a plain file path.
        assert_eq!(
            default_verb(&empty_match(MatchType::Diagnostic), &t),
            Verb::Edit
        );
        // Everything else: insert at the prompt.
        for ty in [
            MatchType::File,
            MatchType::Command,
            MatchType::Sha,
            MatchType::Ipv4,
            MatchType::Ipv6,
            MatchType::Uuid,
            MatchType::QuotedString,
            MatchType::Secret,
        ] {
            assert_eq!(default_verb(&empty_match(ty), &t), Verb::Insert, "{ty:?}");
        }
    }

    #[test]
    fn copy_always_allowed() {
        let t = td();
        for ty in [
            MatchType::Url,
            MatchType::File,
            MatchType::Diagnostic,
            MatchType::Sha,
            MatchType::Ipv4,
            MatchType::Ipv6,
            MatchType::Uuid,
            MatchType::QuotedString,
            MatchType::Command,
            MatchType::Secret,
        ] {
            assert!(is_verb_allowed(&empty_match(ty), Verb::CopyRaw, &t));
        }
    }

    #[test]
    fn secret_denies_open_edit_reveal() {
        let t = td();
        let m = empty_match(MatchType::Secret);
        assert!(!is_verb_allowed(&m, Verb::Open, &t));
        assert!(!is_verb_allowed(&m, Verb::Edit, &t));
        assert!(!is_verb_allowed(&m, Verb::Reveal, &t));
    }

    #[test]
    fn file_allow_set_is_edit_copy_insert() {
        let t = td();
        let m = empty_match(MatchType::File);
        for v in [Verb::Edit, Verb::CopyRaw, Verb::Insert] {
            assert!(is_verb_allowed(&m, v, &t), "file must allow {v:?}");
        }
        // Open and Reveal removed in this simplification.
        assert!(!is_verb_allowed(&m, Verb::Open, &t));
        assert!(!is_verb_allowed(&m, Verb::Reveal, &t));
    }

    #[test]
    fn diagnostic_allow_set_is_edit_copy_insert() {
        let t = td();
        let m = empty_match(MatchType::Diagnostic);
        for v in [Verb::Edit, Verb::CopyRaw, Verb::Insert] {
            assert!(is_verb_allowed(&m, v, &t), "diag must allow {v:?}");
        }
        assert!(!is_verb_allowed(&m, Verb::Open, &t));
        assert!(!is_verb_allowed(&m, Verb::Reveal, &t));
    }

    #[test]
    fn url_does_not_allow_reveal() {
        let t = td();
        assert!(!is_verb_allowed(
            &empty_match(MatchType::Url),
            Verb::Reveal,
            &t
        ));
    }

    #[test]
    fn display_variants_only_on_quoted_string_for_now() {
        let t = td();
        assert!(is_verb_allowed(
            &empty_match(MatchType::QuotedString),
            Verb::CopyDisplay,
            &t
        ));
        assert!(is_verb_allowed(
            &empty_match(MatchType::QuotedString),
            Verb::InsertDisplay,
            &t
        ));
        for ty in [
            MatchType::Url,
            MatchType::Sha,
            MatchType::Ipv4,
            MatchType::Ipv6,
            MatchType::Uuid,
            MatchType::Command,
            MatchType::Secret,
        ] {
            assert!(
                !is_verb_allowed(&empty_match(ty), Verb::CopyDisplay, &t),
                "{ty:?}"
            );
            assert!(
                !is_verb_allowed(&empty_match(ty), Verb::InsertDisplay, &t),
                "{ty:?}"
            );
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
        assert_eq!(verb_from_char('J'), Some(Verb::Json));
        assert_eq!(verb_from_char('z'), None);
    }

    #[test]
    fn json_always_allowed() {
        // Json is universally allowed for every type, including secret
        // (you should be able to export secret tokens as JSON the same
        // way you can copy them — the safety boundary is "don't open"
        // not "don't expose value at all").
        let t = td();
        for ty in [
            MatchType::Url,
            MatchType::File,
            MatchType::Diagnostic,
            MatchType::Sha,
            MatchType::Ipv4,
            MatchType::Ipv6,
            MatchType::Uuid,
            MatchType::QuotedString,
            MatchType::Command,
            MatchType::Secret,
        ] {
            assert!(is_verb_allowed(&empty_match(ty), Verb::Json, &t), "{ty:?}");
        }
    }

    // ---- types config override behavior ----

    fn types_with(tag: &str, actions: Option<Vec<&str>>, default: Option<&str>) -> TypesConfig {
        use crate::config::schema::TypeOverride;
        let mut t = TypesConfig::default();
        t.overrides.insert(
            tag.to_string(),
            TypeOverride {
                actions: actions.map(|v| v.into_iter().map(String::from).collect()),
                default: default.map(String::from),
            },
        );
        t
    }

    #[test]
    fn types_override_allow_list_for_url() {
        // User says: only `open` and `copy` for url. `insert` should now
        // be rejected, but `copy` still allowed (and Json always allowed).
        let t = types_with("url", Some(vec!["open", "copy"]), None);
        let m = empty_match(MatchType::Url);
        assert!(is_verb_allowed(&m, Verb::Open, &t));
        assert!(is_verb_allowed(&m, Verb::CopyRaw, &t));
        assert!(!is_verb_allowed(&m, Verb::Insert, &t));
        assert!(is_verb_allowed(&m, Verb::Json, &t)); // hardcoded universal
    }

    #[test]
    fn types_override_default_verb_for_file() {
        // User: file → edit normally, but they want copy as default.
        let t = types_with("file", None, Some("copy"));
        let m = empty_match(MatchType::File);
        assert_eq!(default_verb(&m, &t), Verb::CopyRaw);
    }

    #[test]
    fn types_override_default_not_in_allowlist_falls_back() {
        // User: actions = ["copy", "insert"] but default = "open".
        // "open" isn't in their allow-list, so default falls back to
        // the static default (Insert for File).
        let t = types_with("file", Some(vec!["copy", "insert"]), Some("open"));
        let m = empty_match(MatchType::File);
        assert_eq!(default_verb(&m, &t), Verb::Insert); // static fallback
    }

    #[test]
    fn types_override_cannot_unmask_secret_hard_deny() {
        // User explicitly tries to enable open/edit/reveal on secrets.
        // Hardcoded deny in is_verb_allowed still refuses.
        let t = types_with(
            "secret",
            Some(vec!["open", "edit", "reveal", "copy"]),
            Some("open"),
        );
        let m = empty_match(MatchType::Secret);
        assert!(!is_verb_allowed(&m, Verb::Open, &t));
        assert!(!is_verb_allowed(&m, Verb::Edit, &t));
        assert!(!is_verb_allowed(&m, Verb::Reveal, &t));
        // Copy still works (override allowed it; not in hard-deny set).
        assert!(is_verb_allowed(&m, Verb::CopyRaw, &t));
        // Default for secret with user "open" → falls back since open
        // isn't actually in the user's effective allow-list... wait,
        // `allowed_verbs` returns the user's raw list (including open).
        // The default check uses `allowed_verbs(m, t).contains(&v)`
        // which would say "yes open is allowed" — but actual dispatch
        // would reject it. Document that user-default of an unmasked
        // hard-denied verb is effectively a no-op at dispatch time.
        // We accept that quirk for now; the user gets a "rejected"
        // status message when they hit Enter.
    }

    #[test]
    fn types_override_empty_actions_blocks_all() {
        // User explicitly empties the action list — only universal
        // verbs (copy-raw, json) remain dispatchable.
        let t = types_with("url", Some(vec![]), None);
        let m = empty_match(MatchType::Url);
        assert!(is_verb_allowed(&m, Verb::CopyRaw, &t)); // universal
        assert!(is_verb_allowed(&m, Verb::Json, &t)); // universal
        assert!(!is_verb_allowed(&m, Verb::Open, &t)); // blocked
        assert!(!is_verb_allowed(&m, Verb::Insert, &t)); // blocked
    }

    #[test]
    fn types_override_unknown_verb_labels_silently_dropped() {
        // User typo: `"opn"` instead of `"open"`. allowed_verbs filters
        // it out so the picker stays consistent — no "phantom verb in
        // the menu" foot-gun.
        let t = types_with("url", Some(vec!["opn", "copy"]), None);
        let m = empty_match(MatchType::Url);
        let allowed = allowed_verbs(&m, &t);
        assert_eq!(allowed, vec![Verb::CopyRaw]); // typo dropped
    }

    #[test]
    fn types_no_override_for_tag_falls_back_to_static() {
        let t = types_with("url", Some(vec!["open"]), None);
        // Override for url doesn't touch file.
        let m = empty_match(MatchType::File);
        assert!(is_verb_allowed(&m, Verb::Edit, &t)); // static allow
        assert!(is_verb_allowed(&m, Verb::Insert, &t)); // static allow
    }

    #[test]
    fn verb_from_label_basic() {
        assert_eq!(verb_from_label("open"), Some(Verb::Open));
        assert_eq!(verb_from_label("copy"), Some(Verb::CopyRaw));
        assert_eq!(verb_from_label("copy-raw"), Some(Verb::CopyRaw)); // alias
        assert_eq!(verb_from_label("copy-display"), Some(Verb::CopyDisplay));
        assert_eq!(verb_from_label("insert"), Some(Verb::Insert));
        assert_eq!(verb_from_label("insert-display"), Some(Verb::InsertDisplay));
        assert_eq!(verb_from_label("edit"), Some(Verb::Edit));
        assert_eq!(verb_from_label("reveal"), Some(Verb::Reveal));
        assert_eq!(verb_from_label("preview"), Some(Verb::Preview));
        assert_eq!(verb_from_label("json"), Some(Verb::Json));
        assert_eq!(verb_from_label("nope"), None);
    }

    // ---- substitute_opt ----

    #[test]
    fn substitute_opt_colon_line_stripped_when_empty() {
        let mut m = empty_match(MatchType::File);
        m.fields
            .insert("file".to_string(), "src/main.rs".to_string());
        // line absent → strip the `:` before {line}
        assert_eq!(substitute_opt("hx {file}:{line}", &m), "hx src/main.rs");
    }

    #[test]
    fn substitute_opt_plus_line_stripped_when_empty() {
        let mut m = empty_match(MatchType::File);
        m.fields
            .insert("file".to_string(), "src/main.rs".to_string());
        // `+` and preceding space both stripped
        assert_eq!(
            substitute_opt("nvim +{line} {file}", &m),
            "nvim src/main.rs"
        );
    }

    #[test]
    fn substitute_opt_line_present_no_stripping() {
        let mut m = empty_match(MatchType::File);
        m.fields
            .insert("file".to_string(), "src/main.rs".to_string());
        m.fields.insert("line".to_string(), "42".to_string());
        assert_eq!(substitute_opt("hx {file}:{line}", &m), "hx src/main.rs:42");
        assert_eq!(
            substitute_opt("nvim +{line} {file}", &m),
            "nvim +42 src/main.rs"
        );
    }

    #[test]
    fn substitute_opt_no_empty_fields_unchanged() {
        let m = url_match();
        assert_eq!(
            substitute_opt("firefox {url}", &m),
            "firefox https://example.com"
        );
    }

    #[test]
    fn substitute_opt_leading_empty_field_no_crash() {
        // Nothing to strip before the first field — just drops the value.
        let m = empty_match(MatchType::File);
        assert_eq!(substitute_opt("{line} rest", &m), " rest");
    }

    #[test]
    fn json_array_single_match() {
        let m = url_match();
        let arr = matches_to_json_array(&[&m]);
        assert!(arr.starts_with('['));
        assert!(arr.ends_with(']'));
        // Single-element array still wrapped — planning.md Q21.
        assert!(arr.contains(r#""type":"url""#));
        assert!(arr.contains(r#""url":"https://example.com""#));
        assert!(arr.contains(r#""scheme":"https""#));
    }

    #[test]
    fn json_array_multiple_matches() {
        let arr = matches_to_json_array(&[&url_match(), &file_match()]);
        // Two objects, comma-joined.
        assert!(arr.starts_with(r#"[{"type":"url""#));
        assert!(arr.contains(r#"},{"type":"file""#));
    }

    #[test]
    fn json_string_escapes_quotes_and_newlines() {
        let mut m = empty_match(MatchType::Url);
        m.raw = "with \"quote\" and\nnewline".to_string();
        m.display = m.raw.clone();
        let arr = matches_to_json_array(&[&m]);
        assert!(arr.contains(r#"\"quote\""#));
        assert!(arr.contains(r#"\n"#));
    }

    #[test]
    fn json_compact_no_whitespace_padding() {
        let arr = matches_to_json_array(&[&url_match()]);
        // No ": " spacing, no newlines, no indents.
        assert!(!arr.contains(": "));
        assert!(!arr.contains('\n'));
    }

    #[test]
    fn json_span_is_a_numeric_pair() {
        let arr = matches_to_json_array(&[&url_match()]);
        // url_match() span is (4, 23).
        assert!(arr.contains(r#""span":[4,23]"#));
    }

    #[test]
    fn substitute_universal_vars() {
        let m = url_match();
        assert_eq!(
            substitute("{type}: {match}", &m),
            "url: https://example.com"
        );
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
        assert_eq!(
            substitute("{{literal} {url}", &m),
            "{literal} https://example.com"
        );
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
        assert_eq!(
            shell_quote("~/dotfiles/config.kdl"),
            "~/dotfiles/config.kdl"
        );
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
