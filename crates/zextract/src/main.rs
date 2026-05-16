//! zextract — Zellij plugin for typed scrollback extraction.
//!
//! Phase 4 scope: full pattern set + live fuzzy filter + modal flow
//! (Input ↔ List, Tab toggles) + per-type action layer (open / edit /
//! reveal / insert / copy + display variants) backed by Zellij's
//! `run_command` and `write_chars_to_pane_id` APIs.
//! See planning.md Phase 4 for acceptance criteria.

mod action;
mod config;
mod extract;
mod fuzzy;
mod pattern;
mod query;
mod render;
mod source_pane;

use std::collections::BTreeMap;
use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget};
use zellij_tile::prelude::*;

use crate::action::{DispatchResult, Verb};
use crate::config::{should_log, Config, LimitsConfig, LogLevel};
use crate::extract::{Match, MatchType};
use crate::fuzzy::{FuzzyEngine, ScoredMatch};
use crate::query::ParsedQuery;

/// Log a message at `$level` if `self.config.log_level` allows it.
/// `$self` is expected to be a `&State` (or anything with `.config`).
/// Prefix `[zextract] ` is added automatically — call sites pass the
/// raw message body. The format!() is short-circuited away when the
/// level is filtered, so cheap when log_level is `off`.
macro_rules! plog {
    ($self:expr, $level:expr, $($arg:tt)*) => {
        if should_log($level, $self.config.log_level) {
            eprintln!("[zextract] {}", format!($($arg)*));
        }
    };
}

// The Phase 1 hardcoded `RECENT_LINES = 150` is now driven by
// `config.grab.profiles[current].lines`. See `apply_config_after_load`
// for how the current profile index is selected and `try_extract`
// for how its source/lines values shape the scrollback grab.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// User is editing the query. Plain printable chars type into the
    /// query; Tab switches to List mode.
    Input,
    /// Plain letter keys are action verbs (y/Y/o/e/r/i/I/p). Tab
    /// switches back to Input mode.
    List,
}

struct State {
    /// True once the async config-load chain has reached a terminal
    /// state (success OR any failure path: no $HOME, host change
    /// rejected, file missing, parse error). Used by `render()` to
    /// gate content display — until the flag is set the picker shows
    /// a minimal "loading…" placeholder. This avoids the visible
    /// reflow that would otherwise happen when content renders at
    /// the keybind's default size, then jumps to the config-driven
    /// width once apply_config_after_load fires.
    config_loaded: bool,
    /// Loaded user config or defaults. Replaced once after plugin
    /// load completes the host-folder handshake (see HostFolderChanged
    /// event handler). Until then, all Phase 7-plumbed settings read
    /// from `Config::default()`. Phase 7's later commits read fields
    /// from here in place of today's hardcoded constants.
    ///
    /// **Timing caveat for config-driven extraction settings:**
    /// today the extraction kicks off on the first `PaneUpdate` (which
    /// usually arrives before `HostFolderChanged`), so the FIRST
    /// extraction uses defaults regardless of what's in the config
    /// file. Settings that don't affect extraction (UI widths, action
    /// templates, limits, editor_command_prefix) take effect on first
    /// render after `HostFolderChanged` — fine. Settings that DO
    /// affect extraction (`grab.recent_lines`, custom `patterns.*`
    /// blocks) need a re-extract once config lands — that wiring is
    /// owed to commit 5 (grab) and commit 8 (custom patterns). Track
    /// `config_loaded: bool` on State and gate extraction or re-trigger
    /// it from the HostFolderChanged handler.
    config: Config,
    matches: Vec<Match>,
    /// The text we extracted from — retained so the preview pane can
    /// render surrounding lines for any selected match. Costs ~12 KB
    /// for the default 150-line cap. Empty before first extraction.
    captured_text: String,
    query: String,
    /// Result of running `query::parse_query` over `self.query` —
    /// recomputed in `refilter` so the renderer can show active
    /// filters as pills without re-parsing each frame.
    parsed_query: ParsedQuery,
    fuzzy: FuzzyEngine,
    filtered: Vec<ScoredMatch>,
    list_state: ListState,
    /// Multi-selection: indices into `self.matches` (stable across
    /// filter changes — a row stays selected even when filtered out,
    /// and re-appears already-selected when the filter brings it back).
    /// Cleared on picker close.
    selected: HashSet<usize>,
    source_pane: Option<u32>,
    /// Our own plugin's pane id, used to call
    /// `change_floating_panes_coordinates` when the preview toggles
    /// (grows the pane to make room).
    own_plugin_id: u32,
    /// Index into `config.grab.profiles` for the active scrollback-
    /// grab profile. Set in `apply_config_after_load` from the
    /// config's `default_profile`. `Ctrl-g` (commit 7c) cycles by
    /// incrementing this mod profiles.len() and re-extracting.
    current_grab_profile_index: usize,
    extraction_done: bool,
    mode: Mode,
    preview_open: bool,
    /// Transient status-bar message. Cleared on the next keystroke.
    /// Phase 9 will time these out; for now any keypress clears.
    message: Option<String>,
    /// Reused ratatui Buffer for rendering. Allocating a fresh one
    /// per frame (rows × cols × ~40 bytes/cell ≈ 500 KB at 90% × 60%)
    /// churns the WASM allocator hard — linear memory keeps growing
    /// until Zellij's host refuses (manifests as
    /// "growth operation limited"). We hold one and re-use it,
    /// resetting cells per frame and reallocating only when the
    /// terminal size actually changes.
    render_buffer: Option<Buffer>,
}

impl Default for State {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            config_loaded: false,
            config: Config::default(),
            matches: Vec::new(),
            captured_text: String::new(),
            query: String::new(),
            parsed_query: ParsedQuery::default(),
            fuzzy: FuzzyEngine::new(),
            filtered: Vec::new(),
            list_state,
            selected: HashSet::new(),
            source_pane: None,
            own_plugin_id: 0,
            current_grab_profile_index: 0,
            extraction_done: false,
            mode: Mode::Input,
            preview_open: false,
            message: None,
            render_buffer: None,
        }
    }
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::ReadPaneContents,
            PermissionType::WriteToClipboard,
            // Phase 4:
            PermissionType::RunCommands,    // open / edit / reveal actions
            PermissionType::WriteToStdin,   // insert action (write_chars_to_pane_id)
            // Phase 7 probe: attempt to read ~/.config/zellij/zextract.kdl
            // from inside the WASI sandbox. Whether this works tells us
            // which config-loading path to commit to (Option A direct
            // read vs Option B configuration-map vs Option C run_command
            // shell-out). See probe_config_read() below.
            PermissionType::FullHdAccess,
        ]);

        let ids = get_plugin_ids();
        plog!(
            self,
            LogLevel::Debug,
            "plugin loaded; plugin_id={} initial_cwd={:?}",
            ids.plugin_id,
            ids.initial_cwd.display().to_string(),
        );
        self.own_plugin_id = ids.plugin_id;
        // The probe runs from the PermissionRequestResult handler —
        // calling change_host_folder before the runtime registers the
        // grant produces "permission denied" even when the cache
        // shows FullHdAccess granted.
        subscribe(&[
            EventType::Key,
            EventType::PaneUpdate,
            EventType::PermissionRequestResult,
            // Phase 7 probe: change_host_folder is async; these events
            // signal completion / failure.
            EventType::HostFolderChanged,
            EventType::FailedToChangeHostFolder,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(_) => {
                // Permissions just landed — kick the async two-step
                // config load: (1) request /host to repoint to $HOME,
                // (2) read once HostFolderChanged confirms the swap.
                // If $HOME is missing the async chain never starts, so
                // mark config_loaded synchronously so the placeholder
                // clears.
                if !self.request_host_change_for_config_load() {
                    self.config_loaded = true;
                }
                self.try_extract();
                true
            }
            Event::HostFolderChanged(new_path) => {
                plog!(
                    self,
                    LogLevel::Debug,
                    "HostFolderChanged: /host -> {:?}",
                    new_path.display().to_string()
                );
                self.load_config_from_host();
                // Whatever happened in load_config_from_host (success,
                // read error, parse error), the load attempt is done —
                // unblock the picker.
                self.config_loaded = true;
                true
            }
            Event::FailedToChangeHostFolder(err) => {
                plog!(
                    self,
                    LogLevel::Warn,
                    "FailedToChangeHostFolder: err={err:?}. \
                     Falling back to defaults."
                );
                self.config_loaded = true;
                true
            }
            Event::PaneUpdate(manifest) => {
                let new_source = source_pane::pick(&manifest);
                let changed = new_source.is_some() && self.source_pane != new_source;
                if changed {
                    self.source_pane = new_source;
                }
                if !self.extraction_done {
                    self.try_extract();
                    return true;
                }
                changed
            }
            Event::Key(key) => self.handle_key(key),
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        let area = Rect {
            x: 0,
            y: 0,
            width: cols as u16,
            height: rows as u16,
        };

        // Reuse the buffer across renders. Reallocate only when the
        // terminal size actually changes. `Buffer::reset` clears all
        // cells in-place without freeing. Hits Zellij's per-plugin
        // wasm memory cap otherwise — see the field doc on State.
        //
        // Split-borrow pattern: take the buffer out of self, render
        // through it (which needs &mut self for the helpers), then
        // put it back. mem::take leaves a None placeholder.
        let mut local_buf = match self.render_buffer.take() {
            Some(mut b) if b.area() == &area => {
                b.reset();
                b
            }
            _ => Buffer::empty(area),
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // input
                Constraint::Min(1),    // list
                Constraint::Length(4), // footer (2 lines + 2 borders)
            ])
            .split(area);

        if !self.config_loaded {
            // Defer real content until the async config-load chain
            // completes — avoids the visible reflow that would
            // otherwise happen when the pane resizes from the
            // keybind's default size to the config-driven width.
            render_loading_placeholder(area, &mut local_buf);
        } else {
            self.render_input(chunks[0], &mut local_buf);
            if self.preview_open {
                let split = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(chunks[1]);
                self.render_list(split[0], &mut local_buf);
                self.render_preview(split[1], &mut local_buf);
            } else {
                self.render_list(chunks[1], &mut local_buf);
            }
            self.render_footer(chunks[2], &mut local_buf);
        }

        render::flush(&local_buf);
        self.render_buffer = Some(local_buf);
    }
}

impl State {
    /// Step 2 of the async config load. Called once `HostFolderChanged`
    /// confirms `/host` now points at `$HOME`. Reads
    /// `/host/.config/zellij/zextract.kdl`, parses to AST, converts to
    /// typed `Config`, and replaces `self.config`. On any failure,
    /// `self.config` stays at its current value (Config::default() if
    /// this is the first call). Parse errors will surface to the user
    /// via a banner in a later commit; for now we log and degrade.
    fn load_config_from_host(&mut self) {
        let path = "/host/.config/zellij/zextract.kdl";
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                plog!(
                    self,
                    LogLevel::Debug,
                    "config load: no file at {path:?} — using defaults"
                );
                return;
            }
            Err(e) => {
                plog!(
                    self,
                    LogLevel::Warn,
                    "config load: read err path={path:?} \
                     kind={:?} err={e} — using defaults",
                    e.kind()
                );
                return;
            }
        };
        match config::parse::parse(&text) {
            Ok(nodes) => {
                self.config = Config::from_ast(&nodes);
                plog!(
                    self,
                    LogLevel::Debug,
                    "config load: OK ({} bytes, {} top-level nodes)",
                    text.len(),
                    nodes.len(),
                );
                self.apply_config_after_load();
            }
            Err(e) => {
                plog!(
                    self,
                    LogLevel::Warn,
                    "config load: parse err {e} — using defaults"
                );
            }
        }
    }

    /// Apply config-driven runtime state after a successful load.
    /// Today: set `preview_open` per the `preview` setting (Always =>
    /// open at launch), select the active grab profile from
    /// `default_profile`, and trigger a pane resize so the configured
    /// preview widths take effect immediately rather than waiting for
    /// the next preview toggle.
    fn apply_config_after_load(&mut self) {
        let initial_preview_open = matches!(
            self.config.ui.preview,
            config::PreviewDefault::Always
        );
        if self.preview_open != initial_preview_open {
            self.preview_open = initial_preview_open;
        }
        // Resolve default_profile name to a position in profiles.
        // Phase 7a's grab parser already falls back to the first
        // profile when the name doesn't match, so this lookup is
        // guaranteed to find a hit when profiles is non-empty.
        self.current_grab_profile_index = self
            .config
            .grab
            .profiles
            .iter()
            .position(|p| p.name == self.config.grab.default_profile)
            .unwrap_or(0);

        // Pane resize regardless of preview_open value — picks up
        // any width changes the user set in config even when preview
        // starts closed.
        self.resize_for_preview();
    }

    fn try_extract(&mut self) {
        if self.extraction_done {
            return;
        }
        let Some(source) = self.source_pane else { return };

        // Pick up the active grab profile. current_grab_profile_index
        // was set in apply_config_after_load — or remains 0 (the first
        // default profile = `quick { source scrollback; lines 150 }`)
        // if config hasn't loaded yet, which matches the old Phase 1
        // behavior verbatim.
        let profile = match self.config.grab.profiles.get(self.current_grab_profile_index) {
            Some(p) => p.clone(),
            None => {
                plog!(self, LogLevel::Warn, "try_extract: no grab profiles available");
                return;
            }
        };

        // `get_full_scrollback` controls whether `lines_above_viewport`
        // is populated. Required for any scrollback-source profile;
        // viewport-only profiles save the extra cost.
        let want_full = matches!(profile.source, config::GrabSource::Scrollback);
        let Ok(contents) = get_pane_scrollback(PaneId::Terminal(source), want_full) else {
            return;
        };

        let mut all = String::new();
        match profile.source {
            config::GrabSource::Scrollback => {
                for line in contents
                    .lines_above_viewport
                    .iter()
                    .chain(contents.viewport.iter())
                {
                    all.push_str(line);
                    all.push('\n');
                }
            }
            config::GrabSource::Viewport => {
                for line in &contents.viewport {
                    all.push_str(line);
                    all.push('\n');
                }
            }
        }
        let trimmed = match profile.lines {
            Some(n) => extract::take_recent(&all, n as usize),
            None => all,
        };

        plog!(
            self,
            LogLevel::Debug,
            "extraction starting; source_pane={source} \
             profile={:?} source_kind={:?} cap={:?} captured_lines={} chars={}",
            profile.name,
            profile.source,
            profile.lines,
            trimmed.lines().count(),
            trimmed.len(),
        );
        self.matches = extract::extract(&trimmed);
        // Retain the source text for the preview pane.
        self.captured_text = trimmed;
        plog!(
            self,
            LogLevel::Debug,
            "extraction done; matches={}",
            self.matches.len()
        );
        self.extraction_done = true;
        self.refilter();
    }

    fn handle_key(&mut self, key: KeyWithModifier) -> bool {
        // Any keystroke clears the transient message from the previous
        // action (e.g. "insert failed: no source pane").
        self.message = None;

        let only_ctrl = key.has_modifiers(&[KeyModifier::Ctrl]) && key.key_modifiers.len() == 1;
        let only_shift = key.has_modifiers(&[KeyModifier::Shift]) && key.key_modifiers.len() == 1;

        // Universal keys handled in both modes.
        match key.bare_key {
            BareKey::Esc => {
                close_self();
                return false;
            }
            BareKey::Tab => {
                self.mode = match self.mode {
                    Mode::Input => Mode::List,
                    Mode::List => Mode::Input,
                };
                return true;
            }
            BareKey::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                if i > 0 {
                    self.list_state.select(Some(i - 1));
                }
                return true;
            }
            BareKey::Down => {
                let i = self.list_state.selected().unwrap_or(0);
                if !self.filtered.is_empty() && i + 1 < self.filtered.len() {
                    self.list_state.select(Some(i + 1));
                }
                return true;
            }
            // Shift-Enter → force insert (raw), regardless of type default.
            BareKey::Enter if only_shift => {
                return self.fire_verb(Verb::Insert);
            }
            BareKey::Enter => {
                return self.fire_default_action();
            }
            // Ctrl-p → toggle preview from either mode.
            BareKey::Char('p') if only_ctrl => {
                self.toggle_preview();
                return true;
            }
            // Ctrl-y → force copy-raw from either mode.
            BareKey::Char('y') if only_ctrl => {
                return self.fire_verb(Verb::CopyRaw);
            }
            // Ctrl-a → select every match currently visible (post-filter).
            BareKey::Char('a') if only_ctrl => {
                self.select_all_visible();
                return true;
            }
            // Ctrl-d → clear the entire selection.
            BareKey::Char('d') if only_ctrl => {
                self.deselect_all();
                return true;
            }
            // Ctrl-g → cycle through grab profiles + re-extract.
            BareKey::Char('g') if only_ctrl => {
                self.cycle_grab_profile();
                return true;
            }
            _ => {}
        }
        // Mode-specific routing.
        match self.mode {
            Mode::Input => self.handle_key_input_mode(key),
            Mode::List => self.handle_key_list_mode(key),
        }
    }

    /// Advance `current_grab_profile_index` to the next configured
    /// grab profile (wrapping), clear `extraction_done`, and re-run
    /// extraction. Status bar reports the new profile + match count
    /// delta so the user sees whether widening/narrowing helped.
    fn cycle_grab_profile(&mut self) {
        if self.config.grab.profiles.is_empty() {
            self.message = Some("no grab profiles configured".into());
            return;
        }
        let prev_count = self.matches.len();
        let n = self.config.grab.profiles.len();
        self.current_grab_profile_index = (self.current_grab_profile_index + 1) % n;
        let name = self.config.grab.profiles[self.current_grab_profile_index]
            .name
            .clone();

        // Force re-extraction with the new profile.
        self.extraction_done = false;
        self.try_extract();

        let delta = self.matches.len() as i64 - prev_count as i64;
        let sign = if delta >= 0 { "+" } else { "" };
        self.message = Some(format!(
            "grab → {name} ({sign}{delta} matches, now {})",
            self.matches.len()
        ));
    }

    fn toggle_preview(&mut self) {
        self.preview_open = !self.preview_open;
        self.message = Some(format!(
            "preview {}",
            if self.preview_open { "on" } else { "off" }
        ));
        self.resize_for_preview();
    }

    fn handle_key_input_mode(&mut self, key: KeyWithModifier) -> bool {
        match key.bare_key {
            BareKey::Backspace => {
                if self.query.pop().is_some() {
                    self.refilter();
                }
                true
            }
            BareKey::Char(c) if !c.is_control() => {
                // Plain printable char with no non-shift modifier: type
                // into the query.
                if key.has_no_modifiers()
                    || (key.has_modifiers(&[KeyModifier::Shift])
                        && key.key_modifiers.len() == 1)
                {
                    self.query.push(c);
                    self.refilter();
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn handle_key_list_mode(&mut self, key: KeyWithModifier) -> bool {
        // Space toggles selection on the current row. Lives here (not
        // the universal handler) because Space in Input mode is part of
        // the query.
        if matches!(key.bare_key, BareKey::Char(' ')) && key.has_no_modifiers() {
            self.toggle_select_current();
            return true;
        }
        match key.bare_key {
            BareKey::Char(c) => {
                // Letter keys are action verbs. Non-shift modifiers
                // (Ctrl/Alt) reserved for Phase 5 (Ctrl-a, Ctrl-d).
                if !(key.has_no_modifiers()
                    || (key.has_modifiers(&[KeyModifier::Shift])
                        && key.key_modifiers.len() == 1))
                {
                    return false;
                }
                let Some(verb) = action::verb_from_char(c) else {
                    return false; // silent reject — key unbound
                };
                self.fire_verb(verb)
            }
            _ => false,
        }
    }

    fn fire_default_action(&mut self) -> bool {
        // Default is determined by the highlighted row's type; multi-
        // select then routes that verb through `fire_verb` which
        // operates on `effective_targets`.
        let Some(m) = self.current_match().cloned() else {
            return false;
        };
        self.fire_verb(action::default_verb(&m, &self.config.types))
    }

    fn fire_verb(&mut self, verb: Verb) -> bool {
        // Preview is a UI-state toggle, short-circuit before the
        // selection/cap machinery.
        if matches!(verb, Verb::Preview) {
            self.toggle_preview();
            return true;
        }
        let targets = self.effective_targets();
        if targets.is_empty() {
            return false;
        }
        self.dispatch_verb_on_targets(verb, &targets)
    }

    /// Apply `verb` to every Match index in `targets`. Semantics
    /// (planning.md Q24):
    ///
    /// 1. **Silent-permissive type-mismatch**: skip targets whose type
    ///    doesn't allow the verb (CopyRaw is universally allowed; secret
    ///    hardcoded-denies open/edit/reveal; etc.). No status message —
    ///    user sees only the side effect of the rows that did fire.
    /// 2. **Loud-reject if zero allowed**: status message, picker stays
    ///    open. Lets the user re-pick.
    /// 3. **Per-verb cap**: if N allowed > cap, refuse loudly (no
    ///    partial fire). User narrows the selection.
    /// 4. **Single-target path**: delegate to `fire_verb_on_match` for
    ///    full per-row semantics (line-aware edit command, etc.).
    /// 5. **Multi-target path**:
    ///      - copy[raw|display] → join all by `\n`, ONE clipboard write
    ///      - insert[raw|display] → join all by space (avoid accidental
    ///        shell-exec from embedded newlines), ONE write_chars
    ///      - open/reveal → N independent invocations
    ///      - edit → one combined `$EDITOR file1 file2 …` command
    ///        (per-file `+line` is dropped — only makes sense for the
    ///        single-file case)
    fn dispatch_verb_on_targets(&mut self, verb: Verb, targets: &[usize]) -> bool {
        let allowed: Vec<usize> = targets
            .iter()
            .filter(|&&i| {
                self.matches
                    .get(i)
                    .map(|m| action::is_verb_allowed(m, verb, &self.config.types))
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        if allowed.is_empty() {
            let sample = targets
                .first()
                .and_then(|&i| self.matches.get(i))
                .map(|m| m.ty.tag())
                .unwrap_or("selection");
            self.message = Some(format!(
                "'{}' not available for [{}]",
                verb.label(),
                sample
            ));
            return true;
        }

        let cap = cap_for_verb(verb, &self.config.limits);
        if allowed.len() > cap {
            self.message = Some(format!(
                "Refused: {} matches exceeds cap of {} for '{}'",
                allowed.len(),
                cap,
                verb.label()
            ));
            return true;
        }

        // Single-target → reuse the existing per-row dispatch path
        // (preserves edit's +line behavior, action.rs's logging, etc.).
        if allowed.len() == 1 {
            let Some(m) = self.matches.get(allowed[0]).cloned() else {
                return false;
            };
            return self.fire_verb_on_match(verb, &m);
        }

        // Multi-target paths.
        match verb {
            Verb::CopyRaw | Verb::CopyDisplay => {
                let pieces: Vec<String> = allowed
                    .iter()
                    .filter_map(|&i| self.matches.get(i))
                    .map(|m| {
                        if matches!(verb, Verb::CopyDisplay) {
                            m.display.clone()
                        } else {
                            m.raw.clone()
                        }
                    })
                    .collect();
                copy_to_clipboard(&pieces.join("\n"));
                close_self();
                false
            }
            Verb::Insert | Verb::InsertDisplay => {
                let Some(pane_id) = self.source_pane else {
                    self.message = Some("insert: no source pane".into());
                    return true;
                };
                let pieces: Vec<String> = allowed
                    .iter()
                    .filter_map(|&i| self.matches.get(i))
                    .map(|m| {
                        if matches!(verb, Verb::InsertDisplay) {
                            m.display.clone()
                        } else {
                            m.raw.clone()
                        }
                    })
                    .collect();
                write_chars_to_pane_id(&pieces.join(" "), PaneId::Terminal(pane_id));
                close_self();
                false
            }
            Verb::Open | Verb::Reveal => {
                for &i in &allowed {
                    if let Some(m) = self.matches.get(i).cloned() {
                        action::dispatch(verb, &m, self.source_pane, &self.config.editor_command_prefix, &self.config.types);
                    }
                }
                close_self();
                false
            }
            Verb::Edit => {
                let Some(pane_id) = self.source_pane else {
                    self.message = Some("edit: no source pane".into());
                    return true;
                };
                let editor = action::resolve_editor(&self.config.editor_command_prefix);
                let files: Vec<String> = allowed
                    .iter()
                    .filter_map(|&i| self.matches.get(i))
                    .map(|m| {
                        let f = m
                            .fields
                            .get("file")
                            .map(|s| s.as_str())
                            .unwrap_or(&m.raw);
                        action::shell_quote(f)
                    })
                    .collect();
                let cmd = format!("{} {}", editor, files.join(" "));
                write_chars_to_pane_id(&cmd, PaneId::Terminal(pane_id));
                close_self();
                false
            }
            Verb::Json => {
                let refs: Vec<&Match> = allowed
                    .iter()
                    .filter_map(|&i| self.matches.get(i))
                    .collect();
                let json = action::matches_to_json_array(&refs);
                copy_to_clipboard(&json);
                close_self();
                false
            }
            Verb::Preview => unreachable!("Preview short-circuited above"),
        }
    }

    fn fire_verb_on_match(&mut self, verb: Verb, m: &Match) -> bool {
        // Preview is a UI-state toggle, not a side-effecting verb —
        // handle here so the dispatch layer stays pure.
        if matches!(verb, Verb::Preview) {
            self.toggle_preview();
            return true;
        }
        match action::dispatch(verb, m, self.source_pane, &self.config.editor_command_prefix, &self.config.types) {
            DispatchResult::Closed => {
                close_self();
                false
            }
            DispatchResult::StayOpen => {
                self.message = Some(format!("{} fired (stay-open)", verb.label()));
                true
            }
            DispatchResult::Rejected => {
                self.message = Some(format!(
                    "'{}' not available for [{}]",
                    verb.label(),
                    m.ty.tag()
                ));
                true
            }
        }
    }

    fn current_match(&self) -> Option<&Match> {
        let i = self.list_state.selected()?;
        let scored = self.filtered.get(i)?;
        self.matches.get(scored.index)
    }

    /// Index into `self.matches` for the currently-highlighted row,
    /// or None if there's no selection cursor.
    fn current_match_index(&self) -> Option<usize> {
        let i = self.list_state.selected()?;
        Some(self.filtered.get(i)?.index)
    }

    /// Toggle the highlighted row's membership in the multi-selection.
    fn toggle_select_current(&mut self) {
        let Some(idx) = self.current_match_index() else { return };
        if !self.selected.insert(idx) {
            self.selected.remove(&idx);
        }
    }

    /// Select every match currently visible in the filtered list.
    /// Matches filtered out by the current query are untouched.
    fn select_all_visible(&mut self) {
        for s in &self.filtered {
            self.selected.insert(s.index);
        }
    }

    fn deselect_all(&mut self) {
        self.selected.clear();
    }

    /// The Match indices to act on. If there's a non-empty selection,
    /// use that. Otherwise fall back to the highlighted row (so single-
    /// match flows keep working without touching Space first).
    fn effective_targets(&self) -> Vec<usize> {
        if !self.selected.is_empty() {
            // Preserve the filter's recency order in the result.
            self.filtered
                .iter()
                .filter(|s| self.selected.contains(&s.index))
                .map(|s| s.index)
                .collect()
        } else if let Some(i) = self.current_match_index() {
            vec![i]
        } else {
            Vec::new()
        }
    }

    /// Ask Zellij to resize our floating pane based on whether preview
    /// is open. Widths come from `config.ui.preview_open_width` and
    /// `preview_closed_width` (defaults `"90%"` and `"70%"`). For
    /// percent-shaped widths we recenter the x-coordinate so the pane
    /// stays centered as it grows/shrinks; for anything else we just
    /// pass the width through and leave x untouched.
    fn resize_for_preview(&self) {
        let w = if self.preview_open {
            &self.config.ui.preview_open_width
        } else {
            &self.config.ui.preview_closed_width
        };
        let x = recenter_x_for_width(w);
        let Some(coords) = FloatingPaneCoordinates::new(
            x.map(|s| s.to_string()),
            None,                       // keep current y
            Some(w.to_string()),
            None,                       // keep current height
            None,                       // pinned unchanged
            None,                       // borderless unchanged
        ) else {
            plog!(self, LogLevel::Warn, "resize: failed to build coords for w={w:?}");
            return;
        };
        change_floating_panes_coordinates(vec![(
            PaneId::Plugin(self.own_plugin_id),
            coords,
        )]);
    }

    fn refilter(&mut self) {
        // Remember the previously-selected match's index so we can preserve
        // selection across filter changes if it's still in the result set.
        let prev_selected_match_idx = self
            .list_state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .map(|s| s.index);

        // Parse the query for `#type` filter tokens. Tag set comes from
        // TYPE_PRIORITY — adding a custom type later is a one-line
        // change at the call site (extend the slice). Cache the result
        // so the renderer can show active filter pills without
        // re-parsing every frame.
        let tags: Vec<&str> = extract::TYPE_PRIORITY
            .iter()
            .map(|t| t.tag())
            .collect();
        self.parsed_query = query::parse_query(&self.query, &tags);
        let parsed = &self.parsed_query;

        // Pre-filter the match-index space by parsed includes/excludes.
        // Empty `includes` = no inclusion constraint. Excludes apply on top.
        // The fuzzy step then runs over only the surviving indices, with
        // displays held in parallel.
        let allowed_indices: Vec<usize> = self
            .matches
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                let tag = m.ty.tag();
                let include_ok = parsed.includes.is_empty()
                    || parsed.includes.iter().any(|t| t == tag);
                let exclude_ok = !parsed.excludes.iter().any(|t| t == tag);
                include_ok && exclude_ok
            })
            .map(|(i, _)| i)
            .collect();

        let allowed_displays: Vec<&str> = allowed_indices
            .iter()
            .map(|&i| self.matches[i].display.as_str())
            .collect();

        // Per-type score bonuses bias relative ranking when fuzzy scores
        // are close. Numbers are small so the primary signal is the
        // fuzzy match itself; bonuses only nudge ties.
        let matches = &self.matches;
        let alloc_idx_for_filter = &allowed_indices;
        let scored = self
            .fuzzy
            .filter_with_bonus(&parsed.fuzzy, &allowed_displays, |i| {
                alloc_idx_for_filter
                    .get(i)
                    .and_then(|&mi| matches.get(mi))
                    .map(|m| extract::type_priority_bonus(m.ty))
                    .unwrap_or(0)
            });

        // The fuzzy engine returns indices into `allowed_displays`;
        // remap back to indices into `self.matches`.
        self.filtered = scored
            .into_iter()
            .filter_map(|s| {
                allowed_indices.get(s.index).map(|&mi| ScoredMatch {
                    index: mi,
                    score: s.score,
                    indices: s.indices,
                })
            })
            .collect();

        let new_selection = if let Some(prev) = prev_selected_match_idx {
            self.filtered.iter().position(|s| s.index == prev).unwrap_or(0)
        } else {
            0
        };
        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(new_selection));
        }
    }

    fn render_input(&self, area: Rect, buf: &mut Buffer) {
        let count_text = if self.matches.is_empty() && !self.extraction_done {
            "(extracting)".to_string()
        } else if self.selected.is_empty() {
            format!("{}/{}", self.filtered.len(), self.matches.len())
        } else {
            // Selection always-visible in the count: e.g. "3 sel · 18/47"
            format!(
                "{} sel · {}/{}",
                self.selected.len(),
                self.filtered.len(),
                self.matches.len(),
            )
        };
        let (mode_tag, marker_style, query_style, cursor_glyph) = match self.mode {
            Mode::Input => (
                "[INPUT]",
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
                Style::default(),
                "█",
            ),
            Mode::List => (
                "[LIST]",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                Style::default().fg(Color::DarkGray),
                " ",
            ),
        };
        let mut spans = vec![
            Span::styled("▍ ", marker_style),
            Span::styled(self.query.clone(), query_style),
            Span::styled(
                cursor_glyph,
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ),
            Span::raw("   "),
        ];

        // Filter pills — one per active include/exclude. Each pill
        // takes the type's color; excludes prefix with `-`.
        for inc in &self.parsed_query.includes {
            let color = type_color_for_tag(inc);
            spans.push(Span::styled(
                format!("[{inc}]"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
        }
        for exc in &self.parsed_query.excludes {
            spans.push(Span::styled(
                format!("[-{exc}]"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ));
            spans.push(Span::raw(" "));
        }
        if !self.parsed_query.includes.is_empty()
            || !self.parsed_query.excludes.is_empty()
        {
            spans.push(Span::raw(" "));
        }

        spans.push(Span::styled(count_text, Style::default().fg(Color::DarkGray)));
        // Active grab profile name — small dim indicator so users
        // remember which slice of scrollback they're searching.
        if let Some(p) = self
            .config
            .grab
            .profiles
            .get(self.current_grab_profile_index)
        {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("grab:{}", p.name),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ));
        }
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            mode_tag,
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
        ));
        Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::ALL).title("zextract"))
            .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        if self.matches.is_empty() {
            let msg = if self.extraction_done {
                "No URLs in pane scrollback."
            } else {
                "Extracting..."
            };
            Paragraph::new(msg)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL))
                .render(area, buf);
            return;
        }

        if self.filtered.is_empty() {
            Paragraph::new(format!("No matches for \"{}\"", self.query))
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL))
                .render(area, buf);
            return;
        }

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .filter_map(|s| self.matches.get(s.index).map(|m| (s, m)))
            .map(|(s, m)| {
                // Leftmost gutter: `● ` for selected rows, `  ` otherwise.
                // The `▸ ` cursor marker comes from highlight_symbol and
                // sits BETWEEN this gutter and the type tag — both visual
                // signals coexist.
                let selected = self.selected.contains(&s.index);
                let gutter = if selected {
                    Span::styled(
                        "● ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("  ")
                };
                let mut spans = vec![
                    gutter,
                    Span::styled(
                        format!("[{}]  ", m.ty.tag()),
                        Style::default().fg(type_color(m.ty)),
                    ),
                ];
                spans.extend(highlight_spans(&m.display, &s.indices));
                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(
                // Rule for this whole UI: every solid .bg() pairs with an
                // explicit contrasting .fg(); never inherit fg from theme.
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");
        StatefulWidget::render(list, area, buf, &mut self.list_state);
    }

    /// Preview pane: shows ±3 lines around the current match in the
    /// captured scrollback. Match line(s) rendered normal; surrounding
    /// context dimmed. Line numbers left-gutter (absolute line in the
    /// captured text, 1-based). No filesystem reads — all content
    /// comes from `self.captured_text`.
    fn render_preview(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default().borders(Borders::ALL).title("preview");
        let Some(m) = self.current_match() else {
            Paragraph::new("(no selection)")
                .style(Style::default().fg(Color::DarkGray))
                .block(block)
                .render(area, buf);
            return;
        };
        if self.captured_text.is_empty() {
            Paragraph::new("(no captured text)")
                .style(Style::default().fg(Color::DarkGray))
                .block(block)
                .render(area, buf);
            return;
        }

        let lines: Vec<&str> = self.captured_text.lines().collect();
        if lines.is_empty() {
            block.render(area, buf);
            return;
        }

        let match_line = line_index_for_span(&self.captured_text, m.span.0);
        let match_line_end = line_index_for_span(&self.captured_text, m.span.1);
        let start = match_line.saturating_sub(3);
        let end = (match_line_end + 3).min(lines.len().saturating_sub(1));
        let line_num_width = (end + 1).to_string().len();

        let dim = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM);
        let gutter_style = Style::default().fg(Color::DarkGray);
        let match_gutter_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);

        let mut content: Vec<Line<'static>> = Vec::new();
        for i in start..=end {
            let is_match = i >= match_line && i <= match_line_end;
            let (line_style, marker, marker_style) = if is_match {
                (Style::default(), "▸", match_gutter_style)
            } else {
                (dim, " ", gutter_style)
            };
            content.push(Line::from(vec![
                Span::styled(
                    format!("{:>w$} ", i + 1, w = line_num_width),
                    gutter_style,
                ),
                Span::styled(marker, marker_style),
                Span::raw(" "),
                Span::styled(lines[i].to_string(), line_style),
            ]));
        }
        Paragraph::new(content).block(block).render(area, buf);
    }

    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(Color::DarkGray);

        // Line 1: type-aware hints (only meaningful in List mode; in
        // Input mode we show a generic hint plus the type-default).
        let mut line1: Vec<Span<'static>> = Vec::new();

        if let Some(m) = self.current_match() {
            let default = action::default_verb(m, &self.config.types);
            line1.push(Span::styled(
                format!(" {}", m.ty.tag()),
                Style::default()
                    .fg(type_color(m.ty))
                    .add_modifier(Modifier::BOLD),
            ));
            line1.push(Span::raw("  ·  "));
            line1.push(Span::styled("Enter", bold));
            line1.push(Span::raw(format!(":{}  ", default.label())));

            // Only show action keys in List mode; in Input mode all those
            // letters go into the query, not actions.
            if matches!(self.mode, Mode::List) {
                for verb in action::allowed_verbs(m, &self.config.types) {
                    if verb == default {
                        continue; // Already shown as Enter:label.
                    }
                    line1.push(Span::styled(verb.key_label(), bold));
                    line1.push(Span::raw(format!(":{}  ", verb.label())));
                }
                // Universal-in-List-mode keys (work for every type).
                line1.push(Span::styled("p", bold));
                line1.push(Span::raw(format!(
                    ":{}  ",
                    if self.preview_open { "preview-off" } else { "preview-on" }
                )));
                line1.push(Span::styled("J", bold));
                line1.push(Span::raw(":json  "));
                line1.push(Span::styled("Space", bold));
                line1.push(Span::raw(":select  "));
            }
        } else {
            line1.push(Span::raw(" "));
            line1.push(Span::styled("no selection", dim));
        }

        // Line 2: universal-shortcut hints (Input mode only — in List
        // mode the plain-letter equivalents are already on line 1, so
        // re-advertising the Ctrl-/Shift- forms would clutter without
        // adding info. The shortcuts STILL WORK in List mode; they're
        // just hidden from the footer).
        let mut line2: Vec<Span<'static>> = vec![
            Span::raw(" "),
            Span::styled("Tab", bold),
            Span::raw(match self.mode {
                Mode::Input => ":list  ",
                Mode::List => ":input  ",
            }),
        ];
        if matches!(self.mode, Mode::Input) {
            line2.extend([
                Span::styled("^Y", bold),
                Span::raw(":copy  "),
                Span::styled("^P", bold),
                Span::raw(":preview  "),
                Span::styled("⇧⏎", bold),
                Span::raw(":insert  "),
                Span::styled("^A", bold),
                Span::raw(":select-all  "),
                Span::styled("^D", bold),
                Span::raw(":clear-sel  "),
                Span::styled("^G", bold),
                Span::raw(":grab  "),
            ]);
        }
        line2.push(Span::styled("Esc", bold));
        line2.push(Span::raw(":close"));
        if let Some(msg) = &self.message {
            line2.push(Span::raw("    "));
            line2.push(Span::styled(
                msg.clone(),
                Style::default().fg(Color::Yellow),
            ));
        }
        let _ = dim; // kept for parity with line1 styling pattern

        let para = Paragraph::new(vec![Line::from(line1), Line::from(line2)])
            .block(Block::default().borders(Borders::ALL));
        para.render(area, buf);
    }
}

/// Build a span sequence for `display` where chars at `indices` are
/// rendered in a highlight style. Char-index based (matches nucleo's
/// returned positions), so URLs (ASCII) and grapheme-simple Unicode
/// both work correctly.
fn highlight_spans(display: &str, indices: &[u32]) -> Vec<Span<'static>> {
    if indices.is_empty() {
        return vec![Span::raw(display.to_string())];
    }
    let hi: HashSet<u32> = indices.iter().copied().collect();
    let highlight = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current = String::new();
    let mut current_hi = false;

    for (i, ch) in display.chars().enumerate() {
        let this_hi = hi.contains(&(i as u32));
        if this_hi != current_hi && !current.is_empty() {
            let style = if current_hi { highlight } else { Style::default() };
            spans.push(Span::styled(std::mem::take(&mut current), style));
        }
        current_hi = this_hi;
        current.push(ch);
    }
    if !current.is_empty() {
        let style = if current_hi { highlight } else { Style::default() };
        spans.push(Span::styled(current, style));
    }
    spans
}

/// Step 1 of the async config load — request that the WASI sandbox's
/// `/host` preopen repoints to the user's home directory. Called
/// from `Event::PermissionRequestResult` so we know FullHdAccess has
/// landed. The actual read fires from `State::load_config_from_host`
/// on the subsequent `Event::HostFolderChanged`.
///
/// Returns true if the async chain was kicked off (HostFolderChanged
/// event will fire), false if we gave up synchronously (no $HOME).
/// The caller uses this to decide whether to leave `config_loaded`
/// false (event will set it later) or flip it now (no event coming).
///
/// Why this dance: the WASI sandbox only preopens `/host`, `/data`,
/// `/tmp`. Reading the user's `~/.config/zellij/zextract.kdl` requires
/// reaching `/host/.config/zellij/zextract.kdl` after `/host` has been
/// repointed at `$HOME`. See planning.md Phase 7 for the rationale.
impl State {
    fn request_host_change_for_config_load(&self) -> bool {
        let home = match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => h,
            _ => {
                plog!(self, LogLevel::Warn, "config load: no $HOME — using defaults");
                return false;
            }
        };
        plog!(self, LogLevel::Debug, "config load: change_host_folder -> {home:?}");
        change_host_folder(std::path::PathBuf::from(&home));
        true
    }
}

/// Per-verb cap on multi-target dispatch. User-configurable via the
/// `limits { ... }` KDL block; defaults come from planning.md Q24
/// (mirrored in `LimitsConfig::default()`). Preview has no cap — it
/// affects only the selection cursor, not external side effects.
fn cap_for_verb(verb: Verb, limits: &LimitsConfig) -> usize {
    match verb {
        Verb::CopyRaw | Verb::CopyDisplay => limits.copy as usize,
        Verb::Insert | Verb::InsertDisplay => limits.insert as usize,
        Verb::Open => limits.open as usize,
        Verb::Edit => limits.edit as usize,
        Verb::Reveal => limits.reveal as usize,
        Verb::Json => limits.json as usize,
        Verb::Preview => usize::MAX,
    }
}

/// Minimal placeholder shown while the async config-load chain
/// hasn't completed yet. Renders inside the existing bordered block
/// so the chrome looks consistent with the loaded state. Lifespan in
/// practice: ~130 ms from plugin load (initial cwd captured) to
/// HostFolderChanged event (read + parse complete).
fn render_loading_placeholder(area: Rect, buf: &mut Buffer) {
    use ratatui::widgets::Wrap;
    let para = Paragraph::new("zextract — loading config…")
        .style(Style::default().fg(Color::DarkGray))
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title("zextract"));
    para.render(area, buf);
}

/// Compute the recentered x-coordinate for a floating pane of the
/// given width. For percent widths `"N%"` returns `"(100-N)/2 %"`
/// so the pane stays centered as the width grows/shrinks. For any
/// other shape (absolute cells, malformed, etc.) returns `None`
/// meaning "don't change x, just let Zellij keep the previous one."
fn recenter_x_for_width(width: &str) -> Option<&'static str> {
    let percent_str = width.strip_suffix('%')?;
    let percent: u32 = percent_str.parse().ok()?;
    if percent >= 100 {
        return Some("0%");
    }
    // Map common percentages to static strings so we don't allocate
    // each render. The defaults exercise just two values.
    match (100 - percent) / 2 {
        0 => Some("0%"),
        5 => Some("5%"),
        10 => Some("10%"),
        15 => Some("15%"),
        20 => Some("20%"),
        25 => Some("25%"),
        // Any unusual width gets None — Zellij keeps the previous x.
        // Acceptable; rare in practice.
        _ => None,
    }
}

/// Compute the 0-based line index of a byte offset within `text`.
/// Used by the preview pane to locate the line that contains a match.
fn line_index_for_span(text: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(text.len());
    text[..clamped].bytes().filter(|&b| b == b'\n').count()
}

/// Resolve a tag-string back to its color. Used by pill rendering
/// where we only have the tag name (the parser doesn't know about
/// MatchType). Unknown tags fall back to gray — only happens if
/// Phase 7's KDL custom-type names get a pill without a registered
/// color, and we'll add user-color config in that phase.
fn type_color_for_tag(tag: &str) -> Color {
    extract::TYPE_PRIORITY
        .iter()
        .find(|t| t.tag() == tag)
        .map(|&t| type_color(t))
        .unwrap_or(Color::Gray)
}

fn type_color(ty: MatchType) -> Color {
    match ty {
        MatchType::Url => Color::Blue,
        MatchType::File => Color::Green,
        MatchType::Diagnostic => Color::LightRed,
        MatchType::Sha => Color::Yellow,
        MatchType::Ipv4 => Color::Cyan,
        MatchType::Ipv6 => Color::Cyan,
        MatchType::Uuid => Color::Magenta,
        MatchType::QuotedString => Color::Gray,
        MatchType::Command => Color::LightMagenta,
        MatchType::Secret => Color::LightRed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_spans_empty_indices() {
        let spans = highlight_spans("hello", &[]);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn highlight_spans_alternating() {
        // "abcde" with indices [0, 2, 4] → "a"+hi, "b"+plain, "c"+hi, "d"+plain, "e"+hi
        let spans = highlight_spans("abcde", &[0, 2, 4]);
        assert_eq!(spans.len(), 5);
    }

    #[test]
    fn highlight_spans_contiguous_run() {
        // "abcde" with indices [1, 2, 3] → "a"+plain, "bcd"+hi, "e"+plain
        let spans = highlight_spans("abcde", &[1, 2, 3]);
        assert_eq!(spans.len(), 3);
    }

    // ---- recenter_x_for_width ----

    #[test]
    fn recenter_x_typical_widths() {
        assert_eq!(recenter_x_for_width("90%"), Some("5%"));
        assert_eq!(recenter_x_for_width("70%"), Some("15%"));
        assert_eq!(recenter_x_for_width("60%"), Some("20%"));
        assert_eq!(recenter_x_for_width("80%"), Some("10%"));
        assert_eq!(recenter_x_for_width("50%"), Some("25%"));
        assert_eq!(recenter_x_for_width("100%"), Some("0%"));
    }

    #[test]
    fn recenter_x_oversize_clamps_to_zero() {
        assert_eq!(recenter_x_for_width("150%"), Some("0%"));
    }

    #[test]
    fn recenter_x_non_percent_returns_none() {
        // Absolute cell widths — let Zellij keep the previous x.
        assert_eq!(recenter_x_for_width("120"), None);
        assert_eq!(recenter_x_for_width(""), None);
        assert_eq!(recenter_x_for_width("nonsense"), None);
    }

    #[test]
    fn recenter_x_uncommon_percent_returns_none() {
        // 77% would recenter to 11.5% — not in our lookup. Fall
        // through to None so Zellij keeps the previous x.
        assert_eq!(recenter_x_for_width("77%"), None);
    }
}
