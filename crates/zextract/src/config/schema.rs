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
    /// back to defaults.
    ///
    /// Per-section parsing is added in later commits — this skeleton
    /// returns defaults regardless of input. Phase-7 commits 3+ fill
    /// in `ui`, `grab`, `limits`, etc.
    pub fn from_ast(_nodes: &[Node]) -> Self {
        Self::default()
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
    fn from_ast_ignores_unknown_sections_for_now() {
        // Skeleton from_ast returns defaults regardless of input.
        // Per-section commits will replace this gradually.
        let nodes = parse::parse(
            r#"
            future_section "unknown thing"
            ui { preview "always" }
        "#,
        )
        .unwrap();
        let config = Config::from_ast(&nodes);
        // Until commit 3 wires `ui` parsing, defaults remain.
        assert_eq!(config.ui.preview, PreviewDefault::Off);
    }
}
