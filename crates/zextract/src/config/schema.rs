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

// The whole schema is intentional "API surface" — fields exist for
// later commits to read in place of today's hardcoded constants.
// Some get consumed in commit 4 (UI), some in 5 (grab), some in 6
// (limits/editor/log_level). Suppressing per-commit until they wire.
#![allow(dead_code)]

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
    pub editor_command_prefix: String,
    pub log_level: LogLevel,
    // Reserved for upcoming commits:
    //   pub patterns: PatternsConfig,
    //   pub types: TypesConfig,
    //   pub actions: ActionsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            grab: GrabConfig::default(),
            limits: LimitsConfig::default(),
            editor_command_prefix: "nvim".to_string(),
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
                // Other sections wired in upcoming commits. Unknown
                // names ignored for forward-compat.
                _ => {}
            }
        }
        config
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
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
        assert_eq!(c.editor_command_prefix, "nvim");
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
        assert_eq!(config.editor_command_prefix, "nvim");
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
        let names: Vec<&str> = config.grab.profiles.iter().map(|p| p.name.as_str()).collect();
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
        let names: Vec<&str> = config.grab.profiles.iter().map(|p| p.name.as_str()).collect();
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
        let vp = config.grab.profiles.iter().find(|p| p.name == "viewport").unwrap();
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
