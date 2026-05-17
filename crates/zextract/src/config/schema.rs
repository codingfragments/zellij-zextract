//! Typed configuration schema with defaults.
//!
//! `Config` mirrors the top-level KDL document structure. Sections
//! are filled in one at a time across subsequent commits; for now
//! `from_ast` returns the defaults and ignores the AST. This keeps
//! the State plumbing PR-able independently of each section's
//! per-block parsing logic.
//!
//! Defaults here are the **single source of truth** for "what does
//! zextract do with no config file?". Any hardcoded constant in
//! source code that today acts as a default eventually moves into
//! one of these fields.

use std::collections::HashMap;

use crate::config::parse::Node;

/// Result of loading the user's config file, or all-defaults if no
/// file was found. Always-valid: failure modes (parse error, missing
/// section, bad value) are absorbed at construction time and surfaced
/// via warnings — the picker never sees an invalid Config.
#[derive(Debug, Clone)]
pub struct Config {
    pub ui: UiConfig,
    pub grab: GrabConfig,
    pub limits: LimitsConfig,
    pub types: TypesConfig,
    pub actions: ActionsConfig,
    pub patterns: PatternsConfig,
    pub log_level: LogLevel,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            grab: GrabConfig::default(),
            limits: LimitsConfig::default(),
            types: TypesConfig::default(),
            actions: ActionsConfig::default(),
            patterns: PatternsConfig::default(),
            log_level: LogLevel::Info,
        }
    }
}

impl Config {
    /// Parse a typed Config from a list of top-level KDL nodes.
    /// Unrecognized sections are silently ignored (forward-compatible
    /// with users editing a newer schema). Missing sections fall
    /// back to defaults. Unrecognized values within a known section
    /// are also skipped — never fail the whole load.
    ///
    /// Per-section parsing lands incrementally; this commit wires
    /// `ui` only. Upcoming commits add `grab`, `limits`, `actions`,
    /// `types`, `patterns`, and the top-level scalars.
    pub fn from_ast(nodes: &[Node]) -> Self {
        let mut config = Self::default();
        for node in nodes {
            match node.name.as_str() {
                "ui" => parse_ui_block(&node.children, &mut config.ui),
                "grab" => parse_grab_block(&node.children, &mut config.grab),
                "limits" => parse_limits_block(&node.children, &mut config.limits),
                "types" => parse_types_block(&node.children, &mut config.types),
                "actions" => parse_actions_block(&node.children, &mut config.actions),
                "patterns" => parse_patterns_block(&node.children, &mut config.patterns),
                "log_level" => {
                    if let Some(s) = node.args.first().and_then(|v| v.as_string()) {
                        if let Some(lvl) = LogLevel::parse(s) {
                            config.log_level = lvl;
                        }
                    }
                }
                // Other sections wired in upcoming commits. Unknown
                // names ignored for forward-compat.
                _ => {}
            }
        }
        config
    }
}

/// Parse a `types { url { actions "open" "copy" "insert"; default "open" } }`
/// block. Each child is a per-type override keyed by the type tag
/// (`url`, `file`, `diag`, `sha`, `ipv4`, `ipv6`, `uuid`, `quote`,
/// `cmd`, `secret`). Unknown tags are accepted at parse time and only
/// rejected at use time (defensive: lets users keep config across
/// future MatchType additions).
fn parse_types_block(nodes: &[Node], types: &mut TypesConfig) {
    for type_node in nodes {
        let tag = type_node.name.clone();
        let mut over = TypeOverride::default();
        for child in &type_node.children {
            match child.name.as_str() {
                "actions" => {
                    let list: Vec<String> = child
                        .args
                        .iter()
                        .filter_map(|v| v.as_string().map(|s| s.to_string()))
                        .collect();
                    // Explicit empty `actions` (no args) means "no
                    // verbs allowed for this type" — honor it. Only
                    // skip if the user gave args but they all failed
                    // to parse as strings (defensive).
                    if !child.args.is_empty() && list.is_empty() {
                        continue;
                    }
                    over.actions = Some(list);
                }
                "default" => {
                    if let Some(s) = child.args.first().and_then(|v| v.as_string()) {
                        over.default = Some(s.to_string());
                    }
                }
                _ => {} // forward-compat
            }
        }
        // Only record if the user set something — empty `url { }` has
        // no effect, preserving the static behavior.
        if over.actions.is_some() || over.default.is_some() {
            types.overrides.insert(tag, over);
        }
    }
}

/// Parse a `limits { copy 100; insert 5; open 10; ... }` block.
/// Each key maps directly to a `LimitsConfig` field. Negative or
/// non-integer values are silently dropped (default preserved) so a
/// typo can't lock the user out of a verb — the planning-mandated cap
/// stays in place.
fn parse_limits_block(nodes: &[Node], limits: &mut LimitsConfig) {
    for node in nodes {
        let Some(n) = node.args.first().and_then(|v| v.as_int()) else {
            continue;
        };
        if n < 0 {
            continue;
        }
        let n = n as u32;
        match node.name.as_str() {
            "copy" => limits.copy = n,
            "insert" => limits.insert = n,
            "open" => limits.open = n,
            "edit" => limits.edit = n,
            "reveal" => limits.reveal = n,
            "json" => limits.json = n,
            _ => {} // forward-compat
        }
    }
}

/// Parse a `grab { default_profile "..." profiles { ... } }` block.
/// Profiles **replace** the default set rather than merging — if the
/// user defines `profiles { quick { ... } }` only `quick` exists.
/// Lets users curate their own list without the four-default baggage.
fn parse_grab_block(nodes: &[Node], grab: &mut GrabConfig) {
    for node in nodes {
        match node.name.as_str() {
            "default_profile" => {
                if let Some(s) = node.args.first().and_then(|v| v.as_string()) {
                    grab.default_profile = s.to_string();
                }
            }
            "profiles" => {
                let mut profiles: Vec<GrabProfile> = Vec::new();
                for profile_node in &node.children {
                    if let Some(p) = parse_grab_profile(profile_node) {
                        profiles.push(p);
                    }
                }
                if !profiles.is_empty() {
                    grab.profiles = profiles;
                }
            }
            _ => {} // forward-compat
        }
    }
    // Ensure default_profile points at an existing profile. If the user
    // misspelled it (or removed the named profile), fall back to the
    // first profile in the list so Ctrl-g cycling still works.
    if !grab.profiles.iter().any(|p| p.name == grab.default_profile) {
        if let Some(first) = grab.profiles.first() {
            grab.default_profile = first.name.clone();
        }
    }
}

fn parse_grab_profile(node: &Node) -> Option<GrabProfile> {
    let mut source = GrabSource::Scrollback;
    let mut lines: Option<u32> = None;
    for child in &node.children {
        match child.name.as_str() {
            "source" => {
                if let Some(s) = child.args.first().and_then(|v| v.as_string()) {
                    source = match s {
                        "scrollback" => GrabSource::Scrollback,
                        "viewport" => GrabSource::Viewport,
                        _ => source, // keep default on unknown
                    };
                }
            }
            "lines" => {
                if let Some(n) = child.args.first().and_then(|v| v.as_int()) {
                    if n > 0 {
                        lines = Some(n as u32);
                    } else {
                        lines = None; // 0 or negative ⇒ unbounded
                    }
                }
            }
            _ => {} // forward-compat
        }
    }
    Some(GrabProfile {
        name: node.name.clone(),
        source,
        lines,
    })
}

fn parse_ui_block(nodes: &[Node], ui: &mut UiConfig) {
    for node in nodes {
        match node.name.as_str() {
            "preview" => {
                if let Some(s) = node.args.first().and_then(|v| v.as_string()) {
                    ui.preview = match s {
                        "off" => PreviewDefault::Off,
                        "auto" => PreviewDefault::Auto,
                        "always" => PreviewDefault::Always,
                        _ => ui.preview,
                    };
                }
            }
            "preview_open_width" => {
                if let Some(s) = node.args.first().and_then(|v| v.as_string()) {
                    ui.preview_open_width = s.to_string();
                }
            }
            "preview_closed_width" => {
                if let Some(s) = node.args.first().and_then(|v| v.as_string()) {
                    ui.preview_closed_width = s.to_string();
                }
            }
            "mask_secrets" => {
                if let Some(b) = node.args.first().and_then(|v| v.as_bool()) {
                    ui.mask_secrets = b;
                }
            }
            _ => {} // forward-compat: ignore unknown
        }
    }
}

// ---- UI ----

#[derive(Debug, Clone)]
pub struct UiConfig {
    pub preview: PreviewDefault,
    pub preview_open_width: String,
    pub preview_closed_width: String,
    pub mask_secrets: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            preview: PreviewDefault::Off,
            preview_open_width: "90%".to_string(),
            preview_closed_width: "70%".to_string(),
            mask_secrets: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewDefault {
    Off,
    Auto,
    Always,
}

// ---- Grab profiles ----

#[derive(Debug, Clone)]
pub struct GrabConfig {
    pub default_profile: String,
    pub profiles: Vec<GrabProfile>,
}

impl Default for GrabConfig {
    fn default() -> Self {
        Self {
            default_profile: "quick".to_string(),
            profiles: vec![
                GrabProfile {
                    name: "quick".to_string(),
                    source: GrabSource::Scrollback,
                    lines: Some(150),
                },
                GrabProfile {
                    name: "deep".to_string(),
                    source: GrabSource::Scrollback,
                    lines: Some(1500),
                },
                GrabProfile {
                    name: "viewport".to_string(),
                    source: GrabSource::Viewport,
                    lines: None,
                },
                GrabProfile {
                    name: "full".to_string(),
                    source: GrabSource::Scrollback,
                    lines: None,
                },
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct GrabProfile {
    pub name: String,
    pub source: GrabSource,
    /// `None` = unbounded (full scrollback).
    pub lines: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabSource {
    Scrollback,
    Viewport,
}

// ---- Types ----

/// Per-type configuration: allow-list and default verb overrides. Keys
/// are type tags (the strings returned by `MatchType::tag`). Stored as
/// raw strings — interpretation happens in `action.rs`. Domain-free
/// keeps the schema testable without pulling in extract/action.
#[derive(Debug, Clone, Default)]
pub struct TypesConfig {
    pub overrides: HashMap<String, TypeOverride>,
}

#[derive(Debug, Clone, Default)]
pub struct TypeOverride {
    /// User-provided verb labels for this type, in display order.
    /// `None` = no override, fall back to static allow-list.
    /// `Some(vec![])` = explicit empty = no verbs allowed.
    pub actions: Option<Vec<String>>,
    /// Verb label fired by Enter. `None` = no override.
    pub default: Option<String>,
}

// ---- Patterns ----

/// User-defined custom regex patterns from the `patterns { }` block.
#[derive(Debug, Clone, Default)]
pub struct PatternsConfig {
    pub command: CommandPatternConfig,
    pub custom: Vec<CustomPattern>,
}

/// Built-in command-pattern tuning, under `patterns { command { ... } }`.
#[derive(Debug, Clone, Default)]
pub struct CommandPatternConfig {
    /// Opt-in heuristic: lines that contain a `-x`/`-xyz`/`--long` style
    /// argument are scanned for a command word by walking back to the
    /// nearest boundary character (`][}{><:;|&`). Off by default because
    /// it produces false positives on prose that incidentally contains
    /// flag-looking tokens. Enable when you want to catch commands that
    /// don't appear after a prompt marker and aren't in the trigger list.
    pub flag_anchored: bool,
}

#[derive(Debug, Clone)]
pub struct CustomPattern {
    /// Name used in error messages and as a display label.
    pub name: String,
    /// Raw regex string. Compiled at extraction time; invalid patterns
    /// are skipped with a log message.
    pub regex: String,
    /// Type tag the match is classified as. Unknown tags fall back to
    /// `"url"`. Must be one of the values returned by `MatchType::tag()`.
    pub ty: String,
    /// Optional `{match}` template applied to the raw match text to
    /// produce the display value. For URL-type patterns this becomes
    /// the URL that open/copy act on.
    pub template: Option<String>,
}

fn parse_patterns_block(nodes: &[Node], patterns: &mut PatternsConfig) {
    for pat_node in nodes {
        if pat_node.name == "command" {
            for child in &pat_node.children {
                if child.name == "flag_anchored" {
                    if let Some(b) = child.args.first().and_then(|v| v.as_bool()) {
                        patterns.command.flag_anchored = b;
                    }
                }
            }
            continue;
        }
        let name = pat_node.name.clone();
        let mut regex: Option<String> = None;
        let mut ty = "url".to_string();
        let mut template: Option<String> = None;
        for child in &pat_node.children {
            match child.name.as_str() {
                "regex" => {
                    if let Some(s) = child.args.first().and_then(|v| v.as_string()) {
                        regex = Some(s.to_string());
                    }
                }
                "type" => {
                    if let Some(s) = child.args.first().and_then(|v| v.as_string()) {
                        ty = s.to_string();
                    }
                }
                "template" => {
                    if let Some(s) = child.args.first().and_then(|v| v.as_string()) {
                        template = Some(s.to_string());
                    }
                }
                _ => {} // forward-compat
            }
        }
        let Some(regex) = regex else {
            continue; // no regex = no pattern
        };
        patterns.custom.push(CustomPattern {
            name,
            regex,
            ty,
            template,
        });
    }
}

// ---- Actions ----

/// Per-type command-template overrides for open / edit / reveal.
/// Keys are type tags (`"url"`, `"file"`, etc.). Unknown tags are
/// accepted at parse time — resolved against `MatchType::tag()` at
/// dispatch time.
#[derive(Debug, Clone, Default)]
pub struct ActionsConfig {
    pub overrides: HashMap<String, VerbTemplates>,
}

/// Command templates for the three "external process" verbs on one
/// type. `None` means fall back to the built-in implementation.
#[derive(Debug, Clone, Default)]
pub struct VerbTemplates {
    /// Shell command template for `open`. Executed via `run_command`.
    /// Example: `"firefox {url}"`, `"open {file}"`.
    pub open: Option<String>,
    /// Shell command template for `edit`. **Inserted into the source
    /// pane** (not run directly) so the user can review + hit Enter.
    /// Example: `"hx {file}:{line}"`, `"nvim +{line} {file}"`.
    pub edit: Option<String>,
    /// Shell command template for `reveal`. Executed via `run_command`.
    /// Example: `"open -R {file}"`, `"nautilus {file}"`.
    pub reveal: Option<String>,
}

/// Parse an `actions { url { open command "..." } }` block.
///
/// Each child is a per-type node whose name is the **type tag** (the
/// short form returned by `MatchType::tag()`):
///   url, file, diag, sha, ipv4, ipv6, uuid, quote, cmd, secret
///
/// Inside each type node, the recognised keys are `open`, `edit`,
/// `reveal`, each in the form `<verb> command "<template>"`. The
/// keyword `command` is required — it reserves space for future verb
/// forms (`command-insert`, `script`, etc.) without a breaking change.
fn parse_actions_block(nodes: &[Node], actions: &mut ActionsConfig) {
    for type_node in nodes {
        let tag = type_node.name.clone();
        let mut over = VerbTemplates::default();
        for child in &type_node.children {
            // Expected form: `<verb> command "<template>"`
            // args[0] = Ident("command"), args[1] = String(template)
            let is_command = child
                .args
                .first()
                .and_then(|v| v.as_string())
                .map(|s| s == "command")
                .unwrap_or(false);
            if !is_command {
                continue;
            }
            let Some(template) = child.args.get(1).and_then(|v| v.as_string()) else {
                continue;
            };
            let template = template.to_string();
            match child.name.as_str() {
                "open" => over.open = Some(template),
                "edit" => over.edit = Some(template),
                "reveal" => over.reveal = Some(template),
                _ => {} // forward-compat
            }
        }
        if over.open.is_some() || over.edit.is_some() || over.reveal.is_some() {
            actions.overrides.insert(tag, over);
        }
    }
}

// ---- Limits ----

#[derive(Debug, Clone)]
pub struct LimitsConfig {
    pub copy: u32,
    pub insert: u32,
    pub open: u32,
    pub edit: u32,
    pub reveal: u32,
    pub json: u32,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        // Mirrors `cap_for_verb` in main.rs (planning.md Q24).
        Self {
            copy: 100,
            insert: 5,
            open: 10,
            edit: 5,
            reveal: 10,
            json: 100,
        }
    }
}

// ---- Logging ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "error" => Some(Self::Error),
            "warn" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }
}

/// Whether a message at `target` should be emitted given the current
/// threshold. Ordering of the enum (Off < Error < Warn < Info < Debug)
/// is leveraged via the `#[derive(PartialOrd, Ord)]` above: a target
/// of `Debug` is the chattiest and only emits when current is `Debug`;
/// `Error` emits whenever current >= Error.
pub fn should_log(target: LogLevel, current: LogLevel) -> bool {
    target != LogLevel::Off && target <= current
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    #[test]
    fn default_values_match_existing_hardcoded() {
        // These defaults should mirror what's hardcoded in source today,
        // so that `Config::default()` produces the same observable
        // behavior as having no config file. If you change these,
        // change the corresponding hardcoded constant — or move the
        // hardcoded constant to call `Config::default().…` instead.
        let c = Config::default();
        assert_eq!(c.ui.preview, PreviewDefault::Off);
        assert_eq!(c.ui.preview_open_width, "90%");
        assert_eq!(c.ui.preview_closed_width, "70%");
        assert!(!c.ui.mask_secrets);
        assert_eq!(c.log_level, LogLevel::Info);

        // Grab profiles: quick(150) / deep(1500) / viewport / full
        assert_eq!(c.grab.default_profile, "quick");
        let names: Vec<&str> = c.grab.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["quick", "deep", "viewport", "full"]);
        let quick = &c.grab.profiles[0];
        assert_eq!(quick.source, GrabSource::Scrollback);
        assert_eq!(quick.lines, Some(150));

        // Limits: planning.md Q24 numbers
        assert_eq!(c.limits.copy, 100);
        assert_eq!(c.limits.insert, 5);
        assert_eq!(c.limits.open, 10);
        assert_eq!(c.limits.edit, 5);
        assert_eq!(c.limits.reveal, 10);
        assert_eq!(c.limits.json, 100);
    }

    #[test]
    fn from_ast_empty_returns_defaults() {
        let config = Config::from_ast(&[]);
        assert_eq!(config.log_level, LogLevel::Info);
    }

    #[test]
    fn from_ast_ignores_unknown_top_level_sections() {
        let nodes = parse::parse(
            r#"
            future_section "unknown thing"
            ui { preview "always" }
        "#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        // unknown_section silently dropped; ui still parsed.
        assert_eq!(config.ui.preview, PreviewDefault::Always);
    }

    // ---- ui block parsing ----

    #[test]
    fn ui_preview_off_auto_always() {
        for (text, expected) in [
            (r#"ui { preview "off" }"#, PreviewDefault::Off),
            (r#"ui { preview "auto" }"#, PreviewDefault::Auto),
            (r#"ui { preview "always" }"#, PreviewDefault::Always),
        ] {
            let nodes = parse::parse(text).unwrap();
            let config = Config::from_ast(&nodes);
            assert_eq!(config.ui.preview, expected, "input: {text}");
        }
    }

    #[test]
    fn ui_unknown_preview_value_keeps_default() {
        let nodes = parse::parse(r#"ui { preview "garbage" }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.ui.preview, PreviewDefault::Off); // default
    }

    #[test]
    fn ui_preview_widths_set_strings() {
        let nodes = parse::parse(
            r#"ui {
                preview_open_width "85%"
                preview_closed_width "60%"
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.ui.preview_open_width, "85%");
        assert_eq!(config.ui.preview_closed_width, "60%");
    }

    #[test]
    fn ui_mask_secrets_bool() {
        let nodes = parse::parse(r#"ui { mask_secrets true }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert!(config.ui.mask_secrets);

        let nodes = parse::parse(r#"ui { mask_secrets false }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert!(!config.ui.mask_secrets);
    }

    #[test]
    fn ui_unknown_inner_keys_ignored() {
        let nodes = parse::parse(
            r#"ui {
                preview "auto"
                future_setting "value"
                mask_secrets true
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        // known keys still applied; unknown silently dropped.
        assert_eq!(config.ui.preview, PreviewDefault::Auto);
        assert!(config.ui.mask_secrets);
    }

    #[test]
    fn ui_partial_block_inherits_defaults_for_missing_keys() {
        // Only `preview` set — other ui fields stay at defaults.
        let nodes = parse::parse(r#"ui { preview "always" }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.ui.preview, PreviewDefault::Always);
        assert_eq!(config.ui.preview_open_width, "90%"); // default
        assert_eq!(config.ui.preview_closed_width, "70%"); // default
        assert!(!config.ui.mask_secrets); // default
    }

    #[test]
    fn ui_bad_type_for_field_keeps_default() {
        // `mask_secrets "yes"` — string where bool expected. Skip silently.
        let nodes = parse::parse(r#"ui { mask_secrets "yes" }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert!(!config.ui.mask_secrets); // default
    }

    // ---- grab block parsing ----

    #[test]
    fn grab_default_block_omitted_keeps_defaults() {
        let config = Config::from_ast(&[]);
        let names: Vec<&str> = config
            .grab
            .profiles
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["quick", "deep", "viewport", "full"]);
        assert_eq!(config.grab.default_profile, "quick");
    }

    #[test]
    fn grab_user_profiles_replace_defaults() {
        let nodes = parse::parse(
            r#"grab {
                default_profile "tiny"
                profiles {
                    tiny {
                        source "scrollback"
                        lines 50
                    }
                    huge {
                        source "scrollback"
                        lines 10000
                    }
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        let names: Vec<&str> = config
            .grab
            .profiles
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["tiny", "huge"]);
        assert_eq!(config.grab.default_profile, "tiny");
        assert_eq!(config.grab.profiles[0].lines, Some(50));
        assert_eq!(config.grab.profiles[1].lines, Some(10000));
    }

    #[test]
    fn grab_viewport_profile_no_lines() {
        let nodes = parse::parse(
            r#"grab {
                profiles {
                    viewport { source "viewport" }
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        let vp = config
            .grab
            .profiles
            .iter()
            .find(|p| p.name == "viewport")
            .unwrap();
        assert_eq!(vp.source, GrabSource::Viewport);
        assert_eq!(vp.lines, None);
    }

    #[test]
    fn grab_full_profile_unbounded() {
        // `lines` omitted entirely → None (unbounded).
        let nodes = parse::parse(
            r#"grab {
                profiles {
                    full { source "scrollback" }
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.grab.profiles[0].lines, None);
    }

    #[test]
    fn grab_lines_zero_means_unbounded() {
        let nodes = parse::parse(
            r#"grab {
                profiles {
                    everything {
                        source "scrollback"
                        lines 0
                    }
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.grab.profiles[0].lines, None);
    }

    #[test]
    fn grab_default_profile_misspelled_falls_back_to_first() {
        // User wrote `default_profile "qiuck"` (typo). We don't error;
        // we silently fall back to the first defined profile so
        // Ctrl-g cycling still has somewhere to start.
        let nodes = parse::parse(
            r#"grab {
                default_profile "qiuck"
                profiles {
                    quick {
                        source "scrollback"
                        lines 150
                    }
                    deep {
                        source "scrollback"
                        lines 1500
                    }
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.grab.default_profile, "quick");
    }

    #[test]
    fn grab_unknown_source_keeps_scrollback_default() {
        let nodes = parse::parse(
            r#"grab {
                profiles {
                    weird {
                        source "nonsense"
                        lines 100
                    }
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.grab.profiles[0].source, GrabSource::Scrollback);
    }

    // ---- patterns block parsing ----

    #[test]
    fn patterns_default_empty() {
        let config = Config::from_ast(&[]);
        assert!(config.patterns.custom.is_empty());
    }

    #[test]
    fn patterns_full_definition() {
        let nodes = parse::parse(
            r#"patterns {
                jira {
                    regex "[A-Z]+-[0-9]+"
                    type "url"
                    template "https://jira.example.com/browse/{match}"
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.patterns.custom.len(), 1);
        let p = &config.patterns.custom[0];
        assert_eq!(p.name, "jira");
        assert_eq!(p.regex, "[A-Z]+-[0-9]+");
        assert_eq!(p.ty, "url");
        assert_eq!(
            p.template.as_deref(),
            Some("https://jira.example.com/browse/{match}")
        );
    }

    #[test]
    fn patterns_no_template_is_ok() {
        let nodes = parse::parse(
            r#"patterns {
                mysecret { regex "MY_[A-Z0-9]{32}" type "secret" }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.patterns.custom[0].template, None);
    }

    #[test]
    fn patterns_missing_regex_skipped() {
        // A pattern block without `regex` is dropped — can't extract
        // without a regex.
        let nodes = parse::parse(r#"patterns { broken { type "url" } }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert!(config.patterns.custom.is_empty());
    }

    #[test]
    fn patterns_multiple_patterns() {
        let nodes = parse::parse(
            r#"patterns {
                jira { regex "[A-Z]+-[0-9]+" type "url" template "https://jira/{match}" }
                pr   { regex "PR#[0-9]+" type "url" template "https://github.com/org/repo/pull/{match}" }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.patterns.custom.len(), 2);
        assert_eq!(config.patterns.custom[0].name, "jira");
        assert_eq!(config.patterns.custom[1].name, "pr");
    }

    // ---- actions block parsing ----

    #[test]
    fn actions_default_block_omitted_has_no_overrides() {
        let config = Config::from_ast(&[]);
        assert!(config.actions.overrides.is_empty());
    }

    #[test]
    fn actions_edit_template_for_file() {
        let nodes = parse::parse(
            r#"actions {
                file {
                    edit command "hx {file}:{line}"
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        let over = config.actions.overrides.get("file").expect("file override");
        assert_eq!(over.edit.as_deref(), Some("hx {file}:{line}"));
        assert!(over.open.is_none());
        assert!(over.reveal.is_none());
    }

    #[test]
    fn actions_open_template_for_url() {
        let nodes = parse::parse(
            r#"actions {
                url {
                    open command "firefox {url}"
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        let over = config.actions.overrides.get("url").unwrap();
        assert_eq!(over.open.as_deref(), Some("firefox {url}"));
    }

    #[test]
    fn actions_multiple_verbs_same_type() {
        let nodes = parse::parse(
            r#"actions {
                file {
                    edit command "hx {file}:{line}"
                    reveal command "open -R {file}"
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        let over = config.actions.overrides.get("file").unwrap();
        assert!(over.edit.is_some());
        assert!(over.reveal.is_some());
        assert!(over.open.is_none());
    }

    #[test]
    fn actions_missing_command_keyword_ignored() {
        // `edit "hx {file}"` without `command` keyword is ignored.
        let nodes = parse::parse(r#"actions { file { edit "hx {file}" } }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert!(config.actions.overrides.is_empty());
    }

    #[test]
    fn actions_unknown_verb_ignored() {
        // `future_verb command "..."` — unknown verb, silently dropped.
        let nodes =
            parse::parse(r#"actions { file { future_verb command "something" } }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert!(config.actions.overrides.is_empty());
    }

    #[test]
    fn actions_multiple_types() {
        let nodes = parse::parse(
            r#"actions {
                file { edit command "hx {file}:{line}" }
                url  { open command "firefox {url}" }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert!(config.actions.overrides.contains_key("file"));
        assert!(config.actions.overrides.contains_key("url"));
    }

    // ---- log_level + should_log ----

    #[test]
    fn log_level_default_info() {
        let config = Config::from_ast(&[]);
        assert_eq!(config.log_level, LogLevel::Info);
    }

    #[test]
    fn log_level_all_named_values_parse() {
        for (text, expected) in [
            (r#"log_level "off""#, LogLevel::Off),
            (r#"log_level "error""#, LogLevel::Error),
            (r#"log_level "warn""#, LogLevel::Warn),
            (r#"log_level "info""#, LogLevel::Info),
            (r#"log_level "debug""#, LogLevel::Debug),
        ] {
            let nodes = parse::parse(text).unwrap();
            let config = Config::from_ast(&nodes);
            assert_eq!(config.log_level, expected, "input: {text}");
        }
    }

    #[test]
    fn log_level_unknown_keeps_default() {
        let nodes = parse::parse(r#"log_level "TRACE""#).unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.log_level, LogLevel::Info);
    }

    #[test]
    fn should_log_threshold_semantics() {
        // Off means: never emit anything, even errors.
        assert!(!should_log(LogLevel::Error, LogLevel::Off));
        assert!(!should_log(LogLevel::Debug, LogLevel::Off));

        // Error threshold: only Error gets through.
        assert!(should_log(LogLevel::Error, LogLevel::Error));
        assert!(!should_log(LogLevel::Warn, LogLevel::Error));

        // Info threshold: Error/Warn/Info pass, Debug doesn't.
        assert!(should_log(LogLevel::Error, LogLevel::Info));
        assert!(should_log(LogLevel::Warn, LogLevel::Info));
        assert!(should_log(LogLevel::Info, LogLevel::Info));
        assert!(!should_log(LogLevel::Debug, LogLevel::Info));

        // Debug threshold: everything passes (except a target of Off,
        // which is nonsensical — you don't emit at "off level").
        assert!(should_log(LogLevel::Error, LogLevel::Debug));
        assert!(should_log(LogLevel::Debug, LogLevel::Debug));
        assert!(!should_log(LogLevel::Off, LogLevel::Debug));
    }

    // ---- types block parsing ----

    #[test]
    fn types_default_block_omitted_has_no_overrides() {
        let config = Config::from_ast(&[]);
        assert!(config.types.overrides.is_empty());
    }

    #[test]
    fn types_user_override_actions_and_default() {
        let nodes = parse::parse(
            r#"types {
                url {
                    actions "open" "copy" "insert"
                    default "open"
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        let over = config.types.overrides.get("url").expect("url override set");
        assert_eq!(
            over.actions.as_deref(),
            Some(&["open".to_string(), "copy".to_string(), "insert".to_string()][..])
        );
        assert_eq!(over.default.as_deref(), Some("open"));
    }

    #[test]
    fn types_block_with_partial_overrides() {
        // Only actions set, no default — `default` should be None.
        let nodes = parse::parse(
            r#"types {
                file { actions "edit" "copy" }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        let over = config.types.overrides.get("file").unwrap();
        assert!(over.actions.is_some());
        assert!(over.default.is_none());
    }

    #[test]
    fn types_block_empty_per_type_record_is_dropped() {
        // `url { }` with no children leaves no override — equivalent to
        // omitting it. Avoids polluting `overrides` with no-op entries.
        let nodes = parse::parse(r#"types { url { } }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert!(config.types.overrides.is_empty());
    }

    #[test]
    fn types_block_explicit_empty_actions_recorded() {
        // `actions` with no args = "block all verbs for this type".
        let nodes = parse::parse(r#"types { url { actions } }"#).unwrap();
        let config = Config::from_ast(&nodes);
        let over = config.types.overrides.get("url").unwrap();
        assert_eq!(over.actions.as_deref(), Some(&[][..]));
    }

    #[test]
    fn types_block_unknown_inner_keys_dropped() {
        let nodes = parse::parse(
            r#"types {
                url {
                    actions "open"
                    future_setting "x"
                }
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        let over = config.types.overrides.get("url").unwrap();
        assert_eq!(over.actions.as_deref(), Some(&["open".to_string()][..]));
    }

    // ---- limits block parsing ----

    #[test]
    fn limits_default_block_omitted_keeps_defaults() {
        let config = Config::from_ast(&[]);
        assert_eq!(config.limits.copy, 100);
        assert_eq!(config.limits.insert, 5);
        assert_eq!(config.limits.open, 10);
        assert_eq!(config.limits.edit, 5);
        assert_eq!(config.limits.reveal, 10);
        assert_eq!(config.limits.json, 100);
    }

    #[test]
    fn limits_user_values_override_defaults() {
        let nodes = parse::parse(
            r#"limits {
                copy 200
                insert 10
                open 25
                edit 8
                reveal 50
                json 500
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.limits.copy, 200);
        assert_eq!(config.limits.insert, 10);
        assert_eq!(config.limits.open, 25);
        assert_eq!(config.limits.edit, 8);
        assert_eq!(config.limits.reveal, 50);
        assert_eq!(config.limits.json, 500);
    }

    #[test]
    fn limits_partial_block_keeps_other_defaults() {
        let nodes = parse::parse(r#"limits { open 25 }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.limits.open, 25); // overridden
        assert_eq!(config.limits.copy, 100); // default
        assert_eq!(config.limits.insert, 5); // default
    }

    #[test]
    fn limits_zero_means_no_dispatch() {
        // 0 is a legal value: blocks the verb entirely. Useful for
        // sandboxing (e.g., `insert 0` to fully disable insert).
        let nodes = parse::parse(r#"limits { insert 0 }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.limits.insert, 0);
    }

    #[test]
    fn limits_negative_value_keeps_default() {
        let nodes = parse::parse(r#"limits { copy -5 }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.limits.copy, 100); // default
    }

    #[test]
    fn limits_unknown_keys_ignored() {
        let nodes = parse::parse(
            r#"limits {
                copy 50
                future_verb 99
            }"#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.limits.copy, 50);
    }

    #[test]
    fn limits_string_value_keeps_default() {
        let nodes = parse::parse(r#"limits { copy "many" }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.limits.copy, 100); // default
    }

    #[test]
    fn grab_only_default_profile_set_keeps_default_profiles() {
        // Setting only `default_profile "deep"` doesn't blow away the
        // four default profiles — we only replace when `profiles { }`
        // is itself present.
        let nodes = parse::parse(r#"grab { default_profile "deep" }"#).unwrap();
        let config = Config::from_ast(&nodes);
        assert_eq!(config.grab.default_profile, "deep");
        assert_eq!(config.grab.profiles.len(), 4); // defaults preserved
    }
}
