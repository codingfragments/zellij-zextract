//! zextract — Zellij plugin for typed scrollback extraction.
//!
//! Phase 2 scope: URL extraction + live fuzzy filter with smart-case.
//! Plain typing edits the query; Up/Down navigate filtered list; Enter
//! copies highlighted URL to clipboard; Esc closes. Still single-mode.
//! See planning.md Phase 2 for acceptance criteria.

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

use crate::extract::{Match, MatchType};
use crate::fuzzy::{FuzzyEngine, ScoredMatch};

/// Hardcoded grab cap for Phase 1+2. Becomes configurable in Phase 7.
const RECENT_LINES: usize = 150;

struct State {
    matches: Vec<Match>,
    query: String,
    fuzzy: FuzzyEngine,
    filtered: Vec<ScoredMatch>,
    list_state: ListState,
    source_pane: Option<u32>,
    extraction_done: bool,
}

impl Default for State {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            matches: Vec::new(),
            query: String::new(),
            fuzzy: FuzzyEngine::new(),
            filtered: Vec::new(),
            list_state,
            source_pane: None,
            extraction_done: false,
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
        ]);
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
                Constraint::Length(3), // footer
            ])
            .split(area);

        self.render_input(chunks[0], &mut buf);
        self.render_list(chunks[1], &mut buf);
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
        self.matches = extract::extract(&trimmed);
        self.extraction_done = true;
        self.refilter();
    }

    fn handle_key(&mut self, key: KeyWithModifier) -> bool {
        match key.bare_key {
            BareKey::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                if i > 0 {
                    self.list_state.select(Some(i - 1));
                }
                true
            }
            BareKey::Down => {
                let i = self.list_state.selected().unwrap_or(0);
                if !self.filtered.is_empty() && i + 1 < self.filtered.len() {
                    self.list_state.select(Some(i + 1));
                }
                true
            }
            BareKey::Enter => {
                if let Some(i) = self.list_state.selected() {
                    if let Some(scored) = self.filtered.get(i) {
                        if let Some(m) = self.matches.get(scored.index) {
                            copy_to_clipboard(&m.raw);
                            close_self();
                        }
                    }
                }
                false
            }
            BareKey::Esc => {
                close_self();
                false
            }
            BareKey::Backspace => {
                if self.query.pop().is_some() {
                    self.refilter();
                }
                true
            }
            BareKey::Char(c) if !c.is_control() => {
                // Plain printable char: append to query.
                // Skip when any non-shift modifier is held — those are reserved
                // for future action keys (Phase 4+).
                if key.has_no_modifiers()
                    || (key.has_modifiers(&[KeyModifier::Shift]) && key.key_modifiers.len() == 1)
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

    fn refilter(&mut self) {
        let displays: Vec<&str> = self.matches.iter().map(|m| m.display.as_str()).collect();
        // Remember the previously-selected match's index so we can preserve
        // selection across filter changes if it's still in the result set.
        let prev_selected_match_idx = self
            .list_state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .map(|s| s.index);

        self.filtered = self.fuzzy.filter(&self.query, &displays);

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
        let line = Line::from(vec![
            Span::styled(
                "▍ ",
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            ),
            Span::raw(self.query.clone()),
            Span::styled(
                "█",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ),
            Span::raw("   "),
            Span::styled(
                count_text,
                Style::default().fg(Color::DarkGray),
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
                let mut spans = vec![
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

    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let spans = vec![
            Span::raw(" type to filter  ·  "),
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" navigate  ·  "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" copy  ·  "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" close"),
        ];
        Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::ALL))
            .render(area, buf);
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

fn type_color(ty: MatchType) -> Color {
    match ty {
        MatchType::Url => Color::Blue,
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
