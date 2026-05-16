//! zextract — Zellij plugin for typed scrollback extraction.
//!
//! Phase 4 scope: full pattern set + live fuzzy filter + modal flow
//! (Input ↔ List, Tab toggles) + per-type action layer (open / edit /
//! reveal / insert / copy + display variants) backed by Zellij's
//! `run_command` and `write_chars_to_pane_id` APIs.
//! See planning.md Phase 4 for acceptance criteria.

mod action;
mod extract;
mod fuzzy;
mod pattern;
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
use crate::extract::{Match, MatchType};
use crate::fuzzy::{FuzzyEngine, ScoredMatch};

/// Hardcoded grab cap for Phase 1+2. Becomes configurable in Phase 7.
const RECENT_LINES: usize = 150;

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
    matches: Vec<Match>,
    /// The text we extracted from — retained so the preview pane can
    /// render surrounding lines for any selected match. Costs ~12 KB
    /// for the default 150-line cap. Empty before first extraction.
    captured_text: String,
    query: String,
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
    extraction_done: bool,
    mode: Mode,
    preview_open: bool,
    /// Transient status-bar message. Cleared on the next keystroke.
    /// Phase 9 will time these out; for now any keypress clears.
    message: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            matches: Vec::new(),
            captured_text: String::new(),
            query: String::new(),
            fuzzy: FuzzyEngine::new(),
            filtered: Vec::new(),
            list_state,
            selected: HashSet::new(),
            source_pane: None,
            own_plugin_id: 0,
            extraction_done: false,
            mode: Mode::Input,
            preview_open: false,
            message: None,
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
            // New for Phase 4:
            PermissionType::RunCommands,    // open / edit / reveal actions
            PermissionType::WriteToStdin,   // insert action (write_chars_to_pane_id)
        ]);

        let ids = get_plugin_ids();
        eprintln!(
            "[zextract] plugin loaded; plugin_id={} initial_cwd={:?}",
            ids.plugin_id,
            ids.initial_cwd.display().to_string(),
        );
        self.own_plugin_id = ids.plugin_id;
        subscribe(&[
            EventType::Key,
            EventType::PaneUpdate,
            EventType::PermissionRequestResult,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(_) => {
                self.try_extract();
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
        let mut buf = Buffer::empty(area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // input
                Constraint::Min(1),    // list
                Constraint::Length(4), // footer (2 lines + 2 borders)
            ])
            .split(area);

        self.render_input(chunks[0], &mut buf);
        if self.preview_open {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            self.render_list(split[0], &mut buf);
            self.render_preview(split[1], &mut buf);
        } else {
            self.render_list(chunks[1], &mut buf);
        }
        self.render_footer(chunks[2], &mut buf);

        render::flush(&buf);
    }
}

impl State {
    fn try_extract(&mut self) {
        if self.extraction_done {
            return;
        }
        let Some(source) = self.source_pane else { return };
        let Ok(contents) = get_pane_scrollback(PaneId::Terminal(source), true) else {
            return;
        };
        let mut all = String::new();
        for line in contents
            .lines_above_viewport
            .iter()
            .chain(contents.viewport.iter())
        {
            all.push_str(line);
            all.push('\n');
        }
        let trimmed = extract::take_recent(&all, RECENT_LINES);
        eprintln!(
            "[zextract] extraction starting; source_pane={source} captured_lines={} chars={}",
            trimmed.lines().count(),
            trimmed.len(),
        );
        self.matches = extract::extract(&trimmed);
        // Retain the source text for the preview pane.
        self.captured_text = trimmed;
        eprintln!(
            "[zextract] extraction done; matches={}",
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
            _ => {}
        }
        // Mode-specific routing.
        match self.mode {
            Mode::Input => self.handle_key_input_mode(key),
            Mode::List => self.handle_key_list_mode(key),
        }
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
        let Some(m) = self.current_match().cloned() else {
            return false;
        };
        self.fire_verb_on_match(action::default_verb(&m), &m)
    }

    fn fire_verb(&mut self, verb: Verb) -> bool {
        let Some(m) = self.current_match().cloned() else {
            return false;
        };
        self.fire_verb_on_match(verb, &m)
    }

    fn fire_verb_on_match(&mut self, verb: Verb, m: &Match) -> bool {
        // Preview is a UI-state toggle, not a side-effecting verb —
        // handle here so the dispatch layer stays pure.
        if matches!(verb, Verb::Preview) {
            self.toggle_preview();
            return true;
        }
        match action::dispatch(verb, m, self.source_pane) {
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
    /// is open. Open → 90% wide, recentered to x=5%. Closed → 70%
    /// wide, recentered to x=15%. Height unchanged (left None so
    /// Zellij keeps whatever the keybind set).
    ///
    /// Phase 7 will make the open/closed widths configurable.
    fn resize_for_preview(&self) {
        let (x, w) = if self.preview_open { ("5%", "90%") } else { ("15%", "70%") };
        let Some(coords) = FloatingPaneCoordinates::new(
            Some(x.to_string()),
            None,                       // keep current y
            Some(w.to_string()),
            None,                       // keep current height
            None,                       // pinned unchanged
            None,                       // borderless unchanged
        ) else {
            eprintln!("[zextract] resize: failed to build coords");
            return;
        };
        change_floating_panes_coordinates(vec![(
            PaneId::Plugin(self.own_plugin_id),
            coords,
        )]);
    }

    fn refilter(&mut self) {
        let displays: Vec<&str> = self.matches.iter().map(|m| m.display.as_str()).collect();
        // Remember the previously-selected match's index so we can preserve
        // selection across filter changes if it's still in the result set.
        let prev_selected_match_idx = self
            .list_state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .map(|s| s.index);

        // Per-type score bonuses bias relative ranking when fuzzy scores
        // are close. Numbers are small so the primary signal is the
        // fuzzy match itself; bonuses only nudge ties.
        let matches = &self.matches;
        self.filtered = self.fuzzy.filter_with_bonus(&self.query, &displays, |i| {
            matches.get(i).map(|m| extract::type_priority_bonus(m.ty)).unwrap_or(0)
        });

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
        } else {
            format!("{}/{}", self.filtered.len(), self.matches.len())
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
        let line = Line::from(vec![
            Span::styled("▍ ", marker_style),
            Span::styled(self.query.clone(), query_style),
            Span::styled(
                cursor_glyph,
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ),
            Span::raw("   "),
            Span::styled(count_text, Style::default().fg(Color::DarkGray)),
            Span::raw("   "),
            Span::styled(
                mode_tag,
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ),
        ]);
        Paragraph::new(line)
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
            let default = action::default_verb(m);
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
                for verb in action::allowed_verbs(m) {
                    if verb == default {
                        continue; // Already shown as Enter:label.
                    }
                    line1.push(Span::styled(verb.key_label(), bold));
                    line1.push(Span::raw(format!(":{}  ", verb.label())));
                }
                // Preview key — universal in List mode, always show.
                line1.push(Span::styled("p", bold));
                line1.push(Span::raw(format!(
                    ":{}  ",
                    if self.preview_open { "preview-off" } else { "preview-on" }
                )));
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

/// Compute the 0-based line index of a byte offset within `text`.
/// Used by the preview pane to locate the line that contains a match.
fn line_index_for_span(text: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(text.len());
    text[..clamped].bytes().filter(|&b| b == b'\n').count()
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
}
